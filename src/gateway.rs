//! Gateway mode: aggregates tools from multiple leaf MCP devices.
//!
//! At startup, each leaf is queried for its tool list. The routing table
//! and tools-list JSON are built once and cached. Tool calls are proxied
//! to the owning leaf via a fresh connection (stateless). No heap — all
//! storage is in fixed arrays sized by const generics.
//!
//! # Memory layout (on a 32-bit target)
//!
//! `Gateway<L, T, B, C>` struct size:
//! - L × 109 bytes  — leaf addresses (108 + 1 length byte each; sized for
//!   Linux `sun_path` but also stores TCP/TLS `host:port`)
//! - T × 34 bytes   — routing table entries (32-byte name + 2 bytes indices)
//! - B bytes        — cached tools-list JSON
//! - 12 bytes       — three usize counters (4 bytes each on 32-bit)
//! - sizeof(C)      — connector. `UnixConnector` / `TcpConnector` /
//!   `MultiConnector` are zero-sized; total cost is **0 bytes**.
//!
//! Default `Gateway<4, 32, 2048>` ≈ 436 + 1088 + 2048 + 12 = **3.6 KB**.
//!
//! Per-call peak stack in `handle()` → `proxy_call()`:
//! - 256-byte request buffer + 1024-byte response buffer = **1.3 KB**.
//!
//! Each `proxy_call` issues exactly two syscalls on the leaf socket
//! (one `write`, one `read` loop until newline). The MCP `initialize`
//! handshake is performed once at startup in `add_leaf` and never repeated —
//! mcp-edge leaves are stateless and don't require a per-call handshake.

use crate::{obj_get, rpc_err, skip_delimited, write_initialize, write_tool, write_tool_size, Writer};
use serde::Deserialize;
use std::io::{Read, Write as IoWrite};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;

// Sized for Linux's `sun_path` (UNIX_PATH_MAX = 108). The same buffer also
// stores TCP/TLS `host:port` strings, which fit comfortably in 108 bytes for
// realistic deployments.
const LEAF_ADDR_MAX: usize = 108;
const NAME_MAX: usize = 32;
const INIT_MSG: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\"}\n";

// ---------------------------------------------------------------------------
// Connector — abstraction over how the gateway opens a leaf connection.
//
// Each `proxy_call` invokes `connector.connect(addr)` once. Implementations
// are zero-sized whenever possible (`UnixConnector`, `TcpConnector`, the
// pre-TLS `MultiConnector`) so they add 0 bytes to the `Gateway` struct.
// Static dispatch — every concrete `Connector` gets its own monomorphized
// `proxy_call`, no vtable on the hot path.
// ---------------------------------------------------------------------------

pub trait Connector {
    type Conn: Read + IoWrite;
    fn connect(&self, addr: &str) -> Result<Self::Conn, &'static str>;
}

/// Connect via a Unix domain socket. `addr` is a filesystem path.
#[derive(Default, Clone, Copy)]
pub struct UnixConnector;

impl Connector for UnixConnector {
    type Conn = UnixStream;
    fn connect(&self, addr: &str) -> Result<UnixStream, &'static str> {
        UnixStream::connect(addr).map_err(|_| "leaf connect failed")
    }
}

/// Connect via TCP. `addr` is `host:port`.
#[derive(Default, Clone, Copy)]
pub struct TcpConnector;

impl Connector for TcpConnector {
    type Conn = TcpStream;
    fn connect(&self, addr: &str) -> Result<TcpStream, &'static str> {
        TcpStream::connect(addr).map_err(|_| "leaf connect failed")
    }
}

/// URL-scheme dispatching connector. Dispatches on the address prefix:
///
/// - `unix://<path>` or bare `<path>` → Unix socket
/// - `tcp://<host:port>`              → TCP
/// - `tls://<host:port>`              → TLS over TCP (requires `tls` feature;
///   stubbed in this PR, integrated in a follow-up)
///
/// Pre-TLS this is zero-sized. Once TLS lands, gains an
/// `Option<Arc<rustls::ClientConfig>>` field.
#[derive(Default, Clone, Copy)]
pub struct MultiConnector;

/// Address scheme parsed from a leaf address. `pub(crate)` so unit tests can
/// exercise the dispatch logic without opening sockets.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Scheme<'a> {
    Unix(&'a str),
    Tcp(&'a str),
    Tls(&'a str),
}

#[inline]
pub(crate) fn parse_scheme(addr: &str) -> Scheme<'_> {
    // Order chosen for the common case first: bare paths + `unix://` win.
    if let Some(p) = addr.strip_prefix("unix://") { Scheme::Unix(p) }
    else if let Some(p) = addr.strip_prefix("tcp://") { Scheme::Tcp(p) }
    else if let Some(p) = addr.strip_prefix("tls://") { Scheme::Tls(p) }
    else { Scheme::Unix(addr) } // backwards compat: bare path → Unix
}

/// Sum-type connection so `MultiConnector` can return one of several stream
/// kinds without trait objects or `Box`. `Read`/`Write` forwarders are
/// `#[inline]` so the variant match collapses at every call site in
/// `proxy_call` — single branch per syscall, no vtable.
///
/// `#[non_exhaustive]`: a `Tls` variant lands once the `tls` feature is wired
/// up. Match externally with a `_` arm to stay forward-compatible.
#[non_exhaustive]
pub enum LeafConnection {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Read for LeafConnection {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            LeafConnection::Unix(s) => s.read(buf),
            LeafConnection::Tcp(s)  => s.read(buf),
        }
    }
}

impl IoWrite for LeafConnection {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            LeafConnection::Unix(s) => s.write(buf),
            LeafConnection::Tcp(s)  => s.write(buf),
        }
    }
    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            LeafConnection::Unix(s) => s.flush(),
            LeafConnection::Tcp(s)  => s.flush(),
        }
    }
}

impl Connector for MultiConnector {
    type Conn = LeafConnection;
    fn connect(&self, addr: &str) -> Result<LeafConnection, &'static str> {
        match parse_scheme(addr) {
            Scheme::Unix(path) => UnixStream::connect(path)
                .map(LeafConnection::Unix)
                .map_err(|_| "leaf connect failed"),
            Scheme::Tcp(hp) => TcpStream::connect(hp)
                .map(LeafConnection::Tcp)
                .map_err(|_| "leaf connect failed"),
            Scheme::Tls(_) => {
                #[cfg(feature = "tls")]
                { Err("tls connector not yet implemented") }
                #[cfg(not(feature = "tls"))]
                { Err("tls feature not enabled") }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal storage — all Copy so they live in fixed arrays, no heap
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
struct LeafAddr {
    addr: [u8; LEAF_ADDR_MAX],
    len: u8,
}

impl LeafAddr {
    const EMPTY: Self = Self { addr: [0u8; LEAF_ADDR_MAX], len: 0 };

    fn as_str(&self) -> &str {
        // SAFETY: the bytes were copied from a `&str` in `add_leaf`, so they
        // are valid UTF-8. Skipping validation avoids a per-call linear scan
        // of the address on the hot tools/call path.
        unsafe { core::str::from_utf8_unchecked(&self.addr[..self.len as usize]) }
    }
}

#[derive(Copy, Clone)]
struct Route {
    name: [u8; NAME_MAX],
    len: u8,
    leaf: u8,
}

impl Route {
    const EMPTY: Self = Self { name: [0u8; NAME_MAX], len: 0, leaf: 0 };

    fn matches(&self, tool: &str) -> bool {
        &self.name[..self.len as usize] == tool.as_bytes()
    }
}

// ---------------------------------------------------------------------------
// Serde struct for parsing a single tool entry from a leaf's tools/list
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct LeafTool<'a> {
    name: &'a str,
    #[serde(default)]
    description: &'a str,
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

/// Aggregates tools from multiple leaf MCP devices.
///
/// - `L`: max leaves (default 4)
/// - `T`: max total tools across all leaves (default 32)
/// - `B`: byte budget for the cached tools-list JSON (default 2048)
/// - `C`: leaf-connection strategy (default `MultiConnector`, which dispatches
///   on URL scheme: `unix://`, `tcp://`, `tls://`, or bare-path-as-Unix).
///   Use `Gateway<L, T, B, UnixConnector>` for a leaner Unix-only build.
///
/// Requires feature `gateway`:
/// ```toml
/// mcp-edge = { version = "*", features = ["gateway"] }
/// ```
pub struct Gateway<
    const L: usize = 4,
    const T: usize = 32,
    const B: usize = 2048,
    C: Connector = MultiConnector,
> {
    leaves: [LeafAddr; L],
    leaf_count: usize,
    routes: [Route; T],
    route_count: usize,
    /// Comma-separated tool JSON (no outer `[` `]` — those are added in handle).
    tools_json: [u8; B],
    tools_json_len: usize,
    connector: C,
}

impl<const L: usize, const T: usize, const B: usize, C: Connector + Default> Gateway<L, T, B, C> {
    pub fn new() -> Self {
        Self {
            leaves: [LeafAddr::EMPTY; L],
            leaf_count: 0,
            routes: [Route::EMPTY; T],
            route_count: 0,
            tools_json: [0u8; B],
            tools_json_len: 0,
            connector: C::default(),
        }
    }
}

impl<const L: usize, const T: usize, const B: usize, C: Connector> Gateway<L, T, B, C> {
    /// Construct a gateway with an explicit connector (e.g. one carrying a
    /// TLS config). Use `new()` for the zero-sized default connectors.
    pub fn with_connector(connector: C) -> Self {
        Self {
            leaves: [LeafAddr::EMPTY; L],
            leaf_count: 0,
            routes: [Route::EMPTY; T],
            route_count: 0,
            tools_json: [0u8; B],
            tools_json_len: 0,
            connector,
        }
    }

    /// Connect to a leaf, run the MCP handshake, discover its tools,
    /// and add them to the routing table. Called at startup, not in the hot path.
    ///
    /// On error the gateway may have partially registered some routes for this
    /// leaf — treat any error as fatal and abort startup rather than continuing.
    pub fn add_leaf(&mut self, path: &str) -> Result<(), &'static str> {
        if self.leaf_count >= L { return Err("leaf limit reached"); }
        if path.is_empty() { return Err("leaf address is empty"); }
        if path.len() > LEAF_ADDR_MAX { return Err("leaf address exceeds LEAF_ADDR_MAX bytes"); }

        // Discover tools before committing the leaf slot, so a failed
        // connection doesn't waste an index. Goes through the connector so
        // `add_leaf` works identically for Unix, TCP, TLS, or any future
        // transport — startup matches runtime.
        let mut resp = [0u8; 4096]; // generous: tools/list response can be large
        let n = {
            let mut conn = self.connector.connect(path)?;
            conn.write_all(INIT_MSG).map_err(|_| "write failed")?;
            // Validate the leaf accepted initialize before proceeding —
            // otherwise we'd silently overwrite the error response with the
            // tools/list reply and lose the failure signal.
            let init_n = read_line(&mut conn, &mut resp).ok_or("no initialize response")?;
            if obj_get(&resp[..init_n], b"error").is_some() {
                return Err("leaf returned error to initialize");
            }

            conn.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
                .map_err(|_| "write failed")?;
            read_line(&mut conn, &mut resp).ok_or("no tools/list response")?
        };

        let result = obj_get(&resp[..n], b"result").ok_or("no result in leaf response")?;
        let tools_arr = obj_get(result, b"tools").ok_or("no tools in leaf response")?;

        // Commit the leaf slot only after a successful connection.
        // Casts to u8 below: leaf_count < L <= 255 (gateway can't index more
        // than 255 leaves anyway), and path.len() <= LEAF_ADDR_MAX = 108
        // (checked above).
        #[allow(clippy::cast_possible_truncation)]
        let leaf_idx = self.leaf_count as u8;
        let path_bytes = path.as_bytes();
        self.leaves[self.leaf_count].addr[..path_bytes.len()].copy_from_slice(path_bytes);
        #[allow(clippy::cast_possible_truncation)]
        {
            self.leaves[self.leaf_count].len = path_bytes.len() as u8;
        }
        self.leaf_count += 1;

        // Iterate `[{...},{...}]` without allocating.
        let mut p = crate::sp(tools_arr, 0);
        if tools_arr.get(p) != Some(&b'[') {
            return Err("leaf returned non-array tools list");
        }
        p += 1;

        loop {
            p = crate::sp(tools_arr, p);
            match tools_arr.get(p) {
                Some(b']') | None => break,
                Some(b',') => { p += 1; continue; }
                Some(b'{') => {}
                _ => return Err("malformed tool entry in leaf response"),
            }

            let obj_start = p;
            if skip_delimited(tools_arr, &mut p, b'{', b'}').is_none() {
                return Err("unterminated tool object in leaf response");
            }
            let tool_obj = &tools_arr[obj_start..p];

            let Ok((tool, _)) = serde_json_core::from_slice::<LeafTool>(tool_obj) else {
                return Err("failed to parse tool entry");
            };
            self.add_route(tool.name, leaf_idx)?;
            self.append_tool_json(tool.name, tool.description)?;
        }

        Ok(())
    }

    fn add_route(&mut self, name: &str, leaf: u8) -> Result<(), &'static str> {
        if name.is_empty() { return Err("tool name is empty"); }
        if name.len() > NAME_MAX { return Err("tool name exceeds NAME_MAX bytes"); }
        if self.route_count >= T { return Err("tool limit (T) reached"); }
        if self.routes[..self.route_count].iter().any(|r| r.matches(name)) {
            return Err("duplicate tool name across leaves");
        }
        // Slot is already zeroed from Gateway::new(); write fields directly.
        let slot = &mut self.routes[self.route_count];
        slot.name[..name.len()].copy_from_slice(name.as_bytes());
        // Cast to u8: name.len() <= NAME_MAX = 32 (checked above).
        #[allow(clippy::cast_possible_truncation)]
        {
            slot.len = name.len() as u8;
        }
        slot.leaf = leaf;
        self.route_count += 1;
        Ok(())
    }

    fn append_tool_json(&mut self, name: &str, desc: &str) -> Result<(), &'static str> {
        let comma = usize::from(self.tools_json_len > 0);
        let needed = comma + write_tool_size(name, desc);
        if self.tools_json_len + needed > B {
            return Err("tools_json buffer (B) too small for combined tool list");
        }
        if comma == 1 {
            self.tools_json[self.tools_json_len] = b',';
            self.tools_json_len += 1;
        }
        let mut w = Writer::new(&mut self.tools_json[self.tools_json_len..]);
        write_tool(&mut w, name, desc);
        self.tools_json_len += w.pos;
        Ok(())
    }

    fn find_leaf(&self, tool: &str) -> Option<usize> {
        self.routes[..self.route_count]
            .iter()
            .find(|r| r.matches(tool))
            .map(|r| r.leaf as usize)
    }

    /// Dispatch one JSON-RPC message. Same interface as `Runtime::handle`.
    pub fn handle(&self, msg: &[u8], out: &mut [u8]) -> usize {
        let mut w = Writer::new(out);

        let Ok((req, _)) = serde_json_core::from_slice::<crate::Req>(msg) else {
            // JSON-RPC 2.0 parse error — see `Runtime::handle` for rationale.
            w.s(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}"#).nl();
            if w.truncated { return 0; }
            return w.pos;
        };

        let id = obj_get(msg, b"id").unwrap_or(b"null");

        match req.method {
            "initialize" => write_initialize(&mut w, id),
            "tools/list" => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).push(id).s(r#","result":{"tools":["#);
                w.push(&self.tools_json[..self.tools_json_len]);
                w.s("]}}").nl();
            }
            "tools/call" => {
                let params_raw = obj_get(msg, b"params").unwrap_or(b"{}");

                let Ok((tc, _)) = serde_json_core::from_slice::<crate::ToolCallParams>(params_raw)
                else {
                    rpc_err(&mut w, id, -32600, "bad params");
                    return w.pos;
                };

                let args = obj_get(params_raw, b"arguments").unwrap_or(b"{}");

                match self.find_leaf(tc.name) {
                    Some(idx) => {
                        let leaf_path = self.leaves[idx].as_str();
                        let start = w.pos;
                        if let Err(e) = self.proxy_call(leaf_path, tc.name, args, id, &mut w) {
                            w.pos = start;
                            rpc_err(&mut w, id, -1, e);
                        }
                    }
                    None => rpc_err(&mut w, id, -32601, "unknown tool"),
                }
            }
            _ => rpc_err(&mut w, id, -32601, "method not found"),
        }

        // See `Runtime::handle`: drop a truncated response instead of
        // forwarding malformed JSON to the client.
        if w.truncated { return 0; }
        w.pos
    }

    /// Proxy a single `tools/call` to the owning leaf. Generic over `C`
    /// so each connector type gets its own monomorphized copy — no vtable
    /// on the proxy hot path.
    fn proxy_call(
        &self,
        leaf_path: &str,
        tool: &str,
        args: &[u8],
        req_id: &[u8],
        w: &mut Writer,
    ) -> Result<(), &'static str> {
        // Request is short: ~60 bytes fixed + tool name + args.
        let mut req_buf = [0u8; 256];
        let req_n = {
            let mut rw = Writer::new(&mut req_buf);
            rw.s(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":""#)
              .s(tool).s(r#"","arguments":"#).push(args).s("}}\n");
            if rw.truncated { return Err("args too large for proxy buffer"); }
            rw.pos
        };

        // Response is bounded by the leaf's OUT parameter (default 512 bytes of
        // content wrapped in ~80 bytes of JSON envelope).
        let mut resp_buf = [0u8; 1024];

        let mut conn = self.connector.connect(leaf_path)?;

        // No per-call `initialize`: the leaf was verified once at startup in
        // `add_leaf` and our runtime is stateless, so we go straight to
        // tools/call. This halves the syscall count and round-trip latency.
        conn.write_all(&req_buf[..req_n]).map_err(|_| "leaf write failed")?;
        let rn = read_line(&mut conn, &mut resp_buf).ok_or("no response from leaf")?;

        let resp = &resp_buf[..rn];
        w.s(r#"{"jsonrpc":"2.0","id":"#).push(req_id);

        if let Some(res) = obj_get(resp, b"result") {
            w.s(r#","result":"#).push(res);
        } else if let Some(err) = obj_get(resp, b"error") {
            w.s(r#","error":"#).push(err);
        } else {
            w.s(r#","error":{"code":-1,"message":"leaf returned no result"}"#);
        }
        w.s("}").nl();
        Ok(())
    }
}

impl<const L: usize, const T: usize, const B: usize, C: Connector + Default> Default
    for Gateway<L, T, B, C>
{
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Direct line reader — reads straight into the caller's buffer.
//
// Each `proxy_call` is a request/response pair on a fresh connection, so we
// don't need a separate buffered reader: we can stream bytes directly into
// `out` and scan each chunk for '\n' with a vectorized `iter().position()`.
//
// Generic over `Read` so it serves UnixStream, TcpStream, LeafConnection,
// or any future TLS-wrapped stream identically. Each concrete `Read` type
// gets its own monomorphized copy of this loop — no vtable per syscall.
// ---------------------------------------------------------------------------

fn read_line<R: Read>(conn: &mut R, out: &mut [u8]) -> Option<usize> {
    let mut pos = 0;
    while pos < out.len() {
        let n = conn.read(&mut out[pos..]).ok().filter(|&n| n > 0)?;
        if let Some(rel) = out[pos..pos + n].iter().position(|&b| b == b'\n') {
            return Some(pos + rel);
        }
        pos += n;
    }
    None
}

// sp, eat_str, and skip_delimited come from crate:: (lib.rs, pub(crate))
// No local reimplementation needed.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_route_rejects_oversized_name() {
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        let too_long = "x".repeat(NAME_MAX + 1);
        assert!(gw.add_route(&too_long, 0).is_err());
        assert!(gw.add_route("", 0).is_err());
    }

    #[test]
    fn add_route_rejects_duplicates() {
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        assert!(gw.add_route("temp", 0).is_ok());
        assert!(gw.add_route("temp", 1).is_err(), "duplicate must be rejected");
    }

    #[test]
    fn add_route_rejects_when_full() {
        let mut gw: Gateway<2, 2, 1024> = Gateway::new();
        assert!(gw.add_route("a", 0).is_ok());
        assert!(gw.add_route("b", 0).is_ok());
        assert!(gw.add_route("c", 0).is_err(), "T limit must be enforced");
    }

    #[test]
    fn parse_scheme_dispatches_correctly() {
        assert_eq!(parse_scheme("unix:///tmp/x.sock"), Scheme::Unix("/tmp/x.sock"));
        assert_eq!(parse_scheme("tcp://10.0.0.5:9000"), Scheme::Tcp("10.0.0.5:9000"));
        assert_eq!(parse_scheme("tls://host:443"),    Scheme::Tls("host:443"));
        // bare path → Unix (backwards compat with existing callers)
        assert_eq!(parse_scheme("/run/leaf.sock"),    Scheme::Unix("/run/leaf.sock"));
        // empty string is still treated as Unix path (will fail at connect time)
        assert_eq!(parse_scheme(""),                  Scheme::Unix(""));
    }

    #[test]
    fn multi_connector_returns_tls_error_when_feature_off() {
        // Without the `tls` feature, a tls:// address should produce a clear
        // error rather than being treated as a unix path. Avoiding
        // `unwrap_err` keeps `LeafConnection` from needing a `Debug` impl
        // (which would bloat code size for no runtime gain).
        let mc = MultiConnector;
        match mc.connect("tls://host:443") {
            Ok(_) => panic!("tls scheme should not connect successfully"),
            Err(e) => assert!(
                e == "tls feature not enabled" || e == "tls connector not yet implemented",
                "unexpected error: {e}"
            ),
        }
    }

    #[test]
    fn gateway_with_unix_connector_compiles_and_constructs() {
        // The escape-hatch: explicit UnixConnector for users who want a leaner
        // build (no LeafConnection enum overhead). This test exists mainly to
        // pin the type signature.
        let _gw: Gateway<2, 4, 1024, UnixConnector> = Gateway::new();
    }

    #[test]
    fn append_tool_json_rejects_overflow() {
        // B=64 is too small for even one realistic tool entry (~80+ bytes).
        let mut gw: Gateway<2, 4, 64> = Gateway::new();
        let r = gw.append_tool_json("temp_read", "Read temperature in Celsius");
        assert!(r.is_err(), "must reject when tools_json buffer (B) is too small");
        assert_eq!(gw.tools_json_len, 0, "no partial write on rejection");
    }
}
