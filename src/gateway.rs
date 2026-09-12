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
//!   Linux `sun_path` but also stores TCP/TLS/UDP `host:port`)
//! - T × 34 bytes   — routing table entries (32-byte name + 2 bytes indices)
//! - B bytes        — cached tools-list JSON
//! - 12 bytes       — three usize counters (4 bytes each on 32-bit)
//! - sizeof(C)      — connector. `UnixConnector` / `TcpConnector` /
//!   `UdpConnector` (with `udp` feature) / `MultiConnector` are zero-sized;
//!   total cost is **0 bytes**.
//!
//! Default `Gateway<4, 32, 2048>` ≈ 436 + 1088 + 2048 + 12 = **3.6 KB**.
//!
//! Per-call peak stack in `handle()` → `proxy_call()`:
//! - 256-byte request buffer + 1024-byte response buffer = **1.3 KB**.
//!
//! `LeafConnection` (used by `MultiConnector`) is a tagged union over
//! single-fd socket wrappers, so adding `Tcp` or `Udp` variants does not
//! grow the per-call connection allocation.
//!
//! Each `proxy_call` issues exactly two syscalls on the leaf socket
//! (one `write`, one `read` loop until newline). The MCP `initialize`
//! handshake is performed once at startup in `add_leaf` and never repeated —
//! mcp-edge leaves are stateless and don't require a per-call handshake.

use crate::{
    classify, obj_get, rpc_err, skip_delimited, unquote, walk_obj_for, write_initialize_gateway,
    write_parse_error, write_tool_raw, Msg, Writer, TOOLS_LIST_CHANGED_NOTIFY, TOOL_JSON_OVERHEAD,
};
use std::io::{Read, Write as IoWrite};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

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

// 5s on the read path is generous enough for slow hardware tool calls (I2C
// bursts, sensor settle times) but short enough that a wedged leaf doesn't
// hang the gateway indefinitely. UDP keeps its tighter 1s because datagram
// loss has no other failure signal — see UdpConnector.
const STREAM_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(feature = "udp")]
const UDP_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

/// Connect via a Unix domain socket. `addr` is a filesystem path.
#[derive(Default, Clone, Copy)]
pub struct UnixConnector;

impl Connector for UnixConnector {
    type Conn = UnixStream;
    fn connect(&self, addr: &str) -> Result<UnixStream, &'static str> {
        let s = UnixStream::connect(addr).map_err(|_| "leaf connect failed")?;
        // Best-effort: a kernel that rejects timeout configuration on a Unix
        // socket is rare and not worth failing the connect over.
        let _ = s.set_read_timeout(Some(STREAM_READ_TIMEOUT));
        Ok(s)
    }
}

/// Connect via TCP. `addr` is `host:port`.
#[derive(Default, Clone, Copy)]
pub struct TcpConnector;

impl Connector for TcpConnector {
    type Conn = TcpStream;
    fn connect(&self, addr: &str) -> Result<TcpStream, &'static str> {
        let s = TcpStream::connect(addr).map_err(|_| "leaf connect failed")?;
        // Disable Nagle: the proxy pattern is one short write + one short read
        // per fresh connection, which is the worst case for Nagle interacting
        // with the receiver's delayed-ACK (can add 40–200ms per call).
        let _ = s.set_nodelay(true);
        let _ = s.set_read_timeout(Some(STREAM_READ_TIMEOUT));
        Ok(s)
    }
}

/// Connect via UDP. `addr` is `host:port`.
///
/// UDP is connectionless; what we call a "connection" here is a bound local
/// socket whose default peer is set via `UdpSocket::connect`. Each request is
/// a single datagram and each reply is expected to be a single datagram —
/// callers must size their request and response payloads to fit (typically
/// well under one MTU; see `UdpStream` docs).
///
/// A 1-second receive timeout is set so silent packet loss surfaces as an
/// error instead of hanging the gateway forever. Best-effort like the
/// stream connectors — a kernel that rejects `SO_RCVTIMEO` on UDP is rare
/// and not worth failing the connect over.
#[cfg(feature = "udp")]
#[derive(Default, Clone, Copy)]
pub struct UdpConnector;

#[cfg(feature = "udp")]
impl Connector for UdpConnector {
    type Conn = UdpStream;
    fn connect(&self, addr: &str) -> Result<UdpStream, &'static str> {
        use std::net::ToSocketAddrs;
        // Resolve once so the local bind family matches the peer's family.
        // Using a mismatched family (v4 socket → v6 peer) errors at send time.
        let peer = addr
            .to_socket_addrs()
            .map_err(|_| "udp address parse failed")?
            .next()
            .ok_or("udp address resolved to nothing")?;
        let bind: &str = if peer.is_ipv6() { "[::]:0" } else { "0.0.0.0:0" };
        let socket = std::net::UdpSocket::bind(bind).map_err(|_| "udp bind failed")?;
        socket.connect(peer).map_err(|_| "udp connect failed")?;
        let _ = socket.set_read_timeout(Some(UDP_READ_TIMEOUT));
        Ok(UdpStream { socket })
    }
}

/// Read+Write adapter over a connected `UdpSocket`.
///
/// Each `read` returns one datagram (`recv`); each `write` sends one
/// datagram (`send`). Datagrams larger than the caller's buffer are
/// **silently truncated by the kernel** — keep request and response
/// payloads under ~1 KB to stay well below typical MTU and the existing
/// `read_line` / `proxy_call` buffer sizes.
///
/// # Framing requirement for UDP leaves
///
/// The gateway's `read_line` scans for a trailing `\n` to delimit a
/// response, so UDP leaves **must** terminate each reply datagram with
/// `\n` (the in-tree `Runtime::handle` already does). A reply without
/// `\n` — or one truncated above — will trigger a 1 s read timeout
/// per call before surfacing as `"no response from leaf"`.
#[cfg(feature = "udp")]
pub struct UdpStream {
    socket: std::net::UdpSocket,
}

#[cfg(feature = "udp")]
impl Read for UdpStream {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.socket.recv(buf)
    }
}

#[cfg(feature = "udp")]
impl IoWrite for UdpStream {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.send(buf)
    }
    #[inline]
    fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
}

/// URL-scheme dispatching connector. Dispatches on the address prefix:
///
/// - `unix://<path>` or bare `<path>` → Unix socket
/// - `tcp://<host:port>`              → TCP
/// - `udp://<host:port>`              → UDP datagram (requires `udp` feature)
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
    Udp(&'a str),
}

#[inline]
pub(crate) fn parse_scheme(addr: &str) -> Scheme<'_> {
    // Order chosen for the common case first: bare paths + `unix://` win.
    if let Some(p) = addr.strip_prefix("unix://") { Scheme::Unix(p) }
    else if let Some(p) = addr.strip_prefix("tcp://") { Scheme::Tcp(p) }
    else if let Some(p) = addr.strip_prefix("tls://") { Scheme::Tls(p) }
    else if let Some(p) = addr.strip_prefix("udp://") { Scheme::Udp(p) }
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
    #[cfg(feature = "udp")]
    Udp(UdpStream),
}

impl Read for LeafConnection {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            LeafConnection::Unix(s) => s.read(buf),
            LeafConnection::Tcp(s)  => s.read(buf),
            #[cfg(feature = "udp")]
            LeafConnection::Udp(s)  => s.read(buf),
        }
    }
}

impl IoWrite for LeafConnection {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            LeafConnection::Unix(s) => s.write(buf),
            LeafConnection::Tcp(s)  => s.write(buf),
            #[cfg(feature = "udp")]
            LeafConnection::Udp(s)  => s.write(buf),
        }
    }
    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            LeafConnection::Unix(s) => s.flush(),
            LeafConnection::Tcp(s)  => s.flush(),
            #[cfg(feature = "udp")]
            LeafConnection::Udp(s)  => s.flush(),
        }
    }
}

impl Connector for MultiConnector {
    type Conn = LeafConnection;
    fn connect(&self, addr: &str) -> Result<LeafConnection, &'static str> {
        match parse_scheme(addr) {
            // Delegate to the dedicated connectors so Nagle/timeout settings
            // stay defined in one place.
            Scheme::Unix(path) => UnixConnector.connect(path).map(LeafConnection::Unix),
            Scheme::Tcp(hp)    => TcpConnector.connect(hp).map(LeafConnection::Tcp),
            Scheme::Tls(_) => {
                #[cfg(feature = "tls")]
                { Err("tls connector not yet implemented") }
                #[cfg(not(feature = "tls"))]
                { Err("tls feature not enabled") }
            }
            Scheme::Udp(hp) => {
                #[cfg(feature = "udp")]
                { UdpConnector.connect(hp).map(LeafConnection::Udp) }
                #[cfg(not(feature = "udp"))]
                { let _ = hp; Err("udp feature not enabled") }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Internal storage — packed byte arena + small index entries.
//
// Pre-packed design: every leaf address and every route name lives back-to-back
// in a single `arena: [u8; A]`, and each `LeafIdx` / `RouteIdx` records only
// `(offset, len)` into that arena. This replaces the previous 109-byte-per-leaf
// and 34-byte-per-route fixed slots, which over-allocated for the common case
// (e.g. a `tcp://10.0.0.5:9000` address paid for the full 108-byte sun_path
// slot). On default settings (L=4, T=32) this saves ~870 bytes per Gateway.
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
struct LeafIdx {
    off: u16,
    len: u8,
}

impl LeafIdx {
    const EMPTY: Self = Self { off: 0, len: 0 };
}

#[derive(Copy, Clone)]
struct RouteIdx {
    name_off: u16,
    name_len: u8,
    desc_off: u16,
    desc_len: u8,
    leaf: u8,
}

impl RouteIdx {
    const EMPTY: Self = Self { name_off: 0, name_len: 0, desc_off: 0, desc_len: 0, leaf: 0 };
    /// Empty slots use `name_len == 0` as the sentinel (since a real route
    /// name must be non-empty per `add_route`'s validation).
    fn is_empty(&self) -> bool { self.name_len == 0 }
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

/// Internal mutable state. Wrapped in `RwLock` inside `Gateway` so that
/// reads (`handle`) can run concurrently while a writer (`add_leaf` /
/// `remove_leaf`) takes exclusive access.
struct GatewayInner<const L: usize, const T: usize, const B: usize, const A: usize> {
    /// Packed byte arena: leaf addresses, route names, and route descriptions
    /// live here back-to-back. Removed entries leave dead bytes — there is
    /// no compaction in this revision.
    arena: [u8; A],
    arena_used: u16,
    leaves: [LeafIdx; L],
    leaf_count: u8,
    routes: [RouteIdx; T],
    route_count: u8,
    /// Comma-separated tool JSON (no outer `[` `]` — those are added in handle).
    /// Rebuilt on every successful add/remove from the active route entries.
    tools_json: [u8; B],
    tools_json_len: u16,
}

impl<const L: usize, const T: usize, const B: usize, const A: usize> GatewayInner<L, T, B, A> {
    const fn new() -> Self {
        Self {
            arena: [0u8; A],
            arena_used: 0,
            leaves: [LeafIdx::EMPTY; L],
            leaf_count: 0,
            routes: [RouteIdx::EMPTY; T],
            route_count: 0,
            tools_json: [0u8; B],
            tools_json_len: 0,
        }
    }

    /// Append `bytes` to the arena, returning the offset where they were written.
    fn arena_push(&mut self, bytes: &[u8]) -> Result<u16, &'static str> {
        let off = self.arena_used as usize;
        let end = off + bytes.len();
        if end > A || end > u16::MAX as usize {
            return Err("name arena (A) full");
        }
        self.arena[off..end].copy_from_slice(bytes);
        #[allow(clippy::cast_possible_truncation)]
        {
            self.arena_used = end as u16;
        }
        Ok(off as u16)
    }

    fn slice(&self, off: u16, len: u8) -> &[u8] {
        let o = off as usize;
        &self.arena[o..o + len as usize]
    }

    fn route_name(&self, r: &RouteIdx) -> &[u8] { self.slice(r.name_off, r.name_len) }

    fn find_leaf_id(&self, tool: &[u8]) -> Option<u8> {
        self.routes[..self.route_count as usize]
            .iter()
            .find(|r| !r.is_empty() && self.route_name(r) == tool)
            .map(|r| r.leaf)
    }

    /// Find a leaf slot whose stored address equals `addr`. Returns the
    /// `(slot_index, leaf_id)` — currently the same value, but this lets us
    /// keep the type-level distinction explicit.
    fn find_leaf_slot_by_addr(&self, addr: &str) -> Option<usize> {
        let target = addr.as_bytes();
        self.leaves[..self.leaf_count as usize]
            .iter()
            .position(|l| l.len != 0 && self.slice(l.off, l.len) == target)
    }

    fn leaf_addr(&self, idx: usize) -> &str {
        let l = &self.leaves[idx];
        // SAFETY: bytes were copied from a `&str` in `commit_leaf`, so they
        // are valid UTF-8.
        unsafe { core::str::from_utf8_unchecked(self.slice(l.off, l.len)) }
    }

    fn add_route(&mut self, name: &[u8], desc: &[u8], leaf: u8) -> Result<(), &'static str> {
        if name.is_empty() { return Err("tool name is empty"); }
        if name.len() > NAME_MAX { return Err("tool name exceeds NAME_MAX bytes"); }
        // Names and descriptions arrive from a leaf, which is an untrusted
        // device on a bus or a network. They are re-emitted verbatim into
        // tools/list, so a non-UTF-8 byte would make the whole response
        // invalid JSON for every client and every tool, not just this entry.
        if core::str::from_utf8(name).is_err() {
            return Err("tool name is not valid UTF-8");
        }
        if core::str::from_utf8(desc).is_err() {
            return Err("tool description is not valid UTF-8");
        }
        // Reject rather than truncate. Cutting at 255 bytes can land mid
        // escape sequence, leaving a trailing `\` that escapes the closing
        // quote and corrupts the entire tools-list JSON.
        if desc.len() > u8::MAX as usize {
            return Err("tool description exceeds 255 bytes");
        }
        if self.routes[..self.route_count as usize]
            .iter()
            .any(|r| !r.is_empty() && self.route_name(r) == name)
        {
            return Err("duplicate tool name across leaves");
        }
        // Reuse an empty slot from a previous remove if one exists; only
        // grow the high-water mark when none are free. Without this, live
        // add/remove churn would eventually exhaust `T` even though no
        // routes are active.
        let slot = match self.routes[..self.route_count as usize]
            .iter()
            .position(|r| r.is_empty())
        {
            Some(i) => i,
            None => {
                if self.route_count as usize >= T { return Err("tool limit (T) reached"); }
                let i = self.route_count as usize;
                self.route_count += 1;
                i
            }
        };
        let name_off = self.arena_push(name)?;
        let desc_off = self.arena_push(desc)?;
        #[allow(clippy::cast_possible_truncation)]
        let name_len = name.len() as u8;
        #[allow(clippy::cast_possible_truncation)]
        let desc_len = desc.len() as u8;
        self.routes[slot] = RouteIdx {
            name_off, name_len, desc_off, desc_len, leaf,
        };
        Ok(())
    }

    /// Rebuild `tools_json` from the active route entries. Called after every
    /// successful add/remove so reads can serve directly from the cache.
    ///
    /// Pre-flights the total byte count before writing anything. Without
    /// the pre-flight pass, a `B`-overflow halfway through the rebuild
    /// would leave `tools_json` partially overwritten and the cached
    /// length stale — producing garbage on the next `tools/list`.
    fn rebuild_tools_json(&mut self) -> Result<(), &'static str> {
        // Pass 1: size check. No mutation, no partial writes possible.
        let mut needed = 0usize;
        let mut first = true;
        for r in self.routes[..self.route_count as usize].iter() {
            if r.is_empty() { continue; }
            let comma = usize::from(!first);
            first = false;
            needed += comma + TOOL_JSON_OVERHEAD + r.name_len as usize + r.desc_len as usize;
        }
        if needed > B {
            return Err("tools_json buffer (B) too small for combined tool list");
        }

        // Pass 2: write. Cache is only mutated after we know it'll fit.
        let arena = &self.arena;
        let routes = &self.routes;
        let route_count = self.route_count;
        let tools_json = &mut self.tools_json;
        let tools_json_len = &mut self.tools_json_len;

        *tools_json_len = 0;
        let mut first = true;
        for r in routes[..route_count as usize].iter() {
            if r.is_empty() { continue; }
            let n_o = r.name_off as usize;
            let n_e = n_o + r.name_len as usize;
            let d_o = r.desc_off as usize;
            let d_e = d_o + r.desc_len as usize;
            let name = &arena[n_o..n_e];
            let desc = &arena[d_o..d_e];

            let cur = *tools_json_len as usize;
            if !first {
                tools_json[cur] = b',';
                *tools_json_len += 1;
            }
            first = false;
            let start = *tools_json_len as usize;
            let mut w = Writer::new(&mut tools_json[start..]);
            write_tool_raw(&mut w, name, desc);
            #[allow(clippy::cast_possible_truncation)]
            {
                *tools_json_len += w.pos as u16;
            }
        }
        Ok(())
    }

    /// Commit phase of `add_leaf`: write the leaf address + tool entries
    /// into the arena/routes, then rebuild the tools_json cache. Rolls back
    /// the inner state on any error so a partial leaf can never linger.
    fn commit_leaf(&mut self, addr: &str, tools_arr: &[u8]) -> Result<(), &'static str> {
        // Reuse an empty leaf slot from a previous remove if one exists;
        // only grow the high-water mark when we have to. Without this, live
        // add/remove churn would eventually exhaust `L` even though no
        // leaves are active.
        let snapshot = (self.arena_used, self.leaf_count, self.route_count, self.tools_json_len);
        let leaf_slot = match self.leaves[..self.leaf_count as usize]
            .iter()
            .position(|l| l.len == 0)
        {
            Some(i) => i,
            None => {
                if self.leaf_count as usize >= L { return Err("leaf limit reached"); }
                self.leaf_count as usize
            }
        };
        // Cast: leaf_slot < L <= 255.
        #[allow(clippy::cast_possible_truncation)]
        let leaf_idx = leaf_slot as u8;

        let result: Result<(), &'static str> = (|| {
            let addr_off = self.arena_push(addr.as_bytes())?;
            #[allow(clippy::cast_possible_truncation)]
            let addr_len = addr.len() as u8;
            self.leaves[leaf_slot] = LeafIdx { off: addr_off, len: addr_len };
            if leaf_slot == self.leaf_count as usize {
                self.leaf_count += 1;
            }

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
                let name_q = obj_get(tool_obj, b"name").ok_or("missing tool name")?;
                let name = unquote(name_q).ok_or("malformed tool name")?;
                let desc = obj_get(tool_obj, b"description")
                    .map(|q| unquote(q).unwrap_or(b""))
                    .unwrap_or(b"");
                self.add_route(name, desc, leaf_idx)?;
            }
            self.rebuild_tools_json()
        })();

        if result.is_err() {
            // Restoring the counters is not enough. When this attempt reused a
            // slot freed by an earlier `remove_leaf`, that slot sits *below*
            // the high-water mark, so the stale entry stays visible and its
            // arena offsets now point at bytes the next `add_leaf` will
            // overwrite. That surfaced as phantom tools in `tools/list` and,
            // via `leaf_addr`'s `from_utf8_unchecked`, as undefined behaviour.
            // Clear what we wrote before rewinding the counters.
            self.leaves[leaf_slot] = LeafIdx::EMPTY;
            for r in self.routes.iter_mut() {
                if !r.is_empty() && r.leaf == leaf_idx {
                    *r = RouteIdx::EMPTY;
                }
            }
            self.arena_used = snapshot.0;
            self.leaf_count = snapshot.1;
            self.route_count = snapshot.2;
            self.tools_json_len = snapshot.3;
        }
        result
    }

    /// Remove a leaf by address. Marks the leaf slot empty, drops all of
    /// that leaf's routes, compacts the name arena to reclaim the dead
    /// bytes, and rebuilds `tools_json`. After this returns, the gateway
    /// is in the same shape it would be if the leaf had never been added.
    fn remove_leaf_inner(&mut self, addr: &str) -> Result<(), &'static str> {
        let slot = self.find_leaf_slot_by_addr(addr).ok_or("no such leaf")?;
        // Cast: slot < leaf_count <= L <= 255.
        #[allow(clippy::cast_possible_truncation)]
        let leaf_id = slot as u8;
        self.leaves[slot] = LeafIdx::EMPTY;
        for r in self.routes[..self.route_count as usize].iter_mut() {
            if !r.is_empty() && r.leaf == leaf_id {
                *r = RouteIdx::EMPTY;
            }
        }
        self.compact_arena();
        self.rebuild_tools_json()
    }

    /// Compact the name arena: copy every still-live (leaf addr, route name,
    /// route desc) chunk into a fresh `[u8; A]`, updating each entry's
    /// offset. After this, `arena_used` reflects only the bytes that back
    /// active entries; long-running gateways with churning leaves no longer
    /// drift toward the `A` cap.
    ///
    /// Disjoint field borrows let us read from `self.arena` while mutating
    /// `self.leaves` and `self.routes`. The new arena lives on the stack
    /// for the duration (A bytes — 1 KiB at default), then a single memcpy
    /// installs it.
    fn compact_arena(&mut self) {
        let mut new_arena = [0u8; A];
        let mut new_used: u16 = 0;
        let old_arena = &self.arena;
        let leaves = &mut self.leaves;
        let routes = &mut self.routes;

        for l in leaves.iter_mut() {
            if l.len == 0 { continue; }
            let lo = l.off as usize;
            let len = l.len as usize;
            let nu = new_used as usize;
            new_arena[nu..nu + len].copy_from_slice(&old_arena[lo..lo + len]);
            l.off = new_used;
            #[allow(clippy::cast_possible_truncation)]
            { new_used += len as u16; }
        }
        for r in routes.iter_mut() {
            if r.is_empty() { continue; }
            let no = r.name_off as usize;
            let nl = r.name_len as usize;
            let nu = new_used as usize;
            new_arena[nu..nu + nl].copy_from_slice(&old_arena[no..no + nl]);
            r.name_off = new_used;
            #[allow(clippy::cast_possible_truncation)]
            { new_used += nl as u16; }
            if r.desc_len > 0 {
                let dlo = r.desc_off as usize;
                let dl = r.desc_len as usize;
                let nu = new_used as usize;
                new_arena[nu..nu + dl].copy_from_slice(&old_arena[dlo..dlo + dl]);
                r.desc_off = new_used;
                #[allow(clippy::cast_possible_truncation)]
                { new_used += dl as u16; }
            }
        }
        self.arena = new_arena;
        self.arena_used = new_used;
    }
}

// ---------------------------------------------------------------------------
// Static Gateway — the small, fast, single-threaded variant.
//
// State lives directly inside the struct, no lock. Mutators (`add_leaf`)
// take `&mut self`; dispatch (`handle`) takes `&self`. Once `add_leaf`
// is done at startup, the borrow checker enforces that no further
// mutation can happen during the lifetime of the `transport.serve(...)`
// closure that captures `&gw` — which is exactly the runtime guarantee
// we want for the embedded use case. No runtime locking; the language
// itself proves the freeze.
// ---------------------------------------------------------------------------

/// Aggregates tools from multiple leaf MCP devices.
///
/// # Choosing between `Gateway` and [`DynamicGateway`]
///
/// |                                | `Gateway`                       | [`DynamicGateway`]                       |
/// |--------------------------------|---------------------------------|------------------------------------------|
/// | Leaves added at runtime        | startup only (`&mut self`)      | any time, from any thread (`&self`)      |
/// | Leaves removed at runtime      | not supported                   | `remove_leaf(addr)`                      |
/// | `tools/list_changed` push      | not sent                        | broadcast on every mutation              |
/// | Multi-client                   | one connection at a time        | thread-per-connection                    |
/// | Heap allocation                | none                            | `Vec`/`Arc`/`Box` for the subscriber list|
/// | Synchronization primitives     | none                            | `RwLock` + `Mutex` + `AtomicU64`         |
/// | Typical use                    | embedded device, fixed leaves   | Linux gateway, dynamic discovery         |
///
/// Both share the same wire protocol; both declare `tools.listChanged: true`
/// in `initialize` (the spec only requires the capability, not that the
/// server ever actually sends a notification).
///
/// # Const generics
///
/// - `L`: max leaves (default 4)
/// - `T`: max total tools across all leaves (default 32)
/// - `B`: byte budget for the cached tools-list JSON (default 2048)
/// - `C`: leaf-connection strategy (default `MultiConnector`).
/// - `A`: byte budget for the packed name-arena holding leaf addresses,
///   tool names, and tool descriptions (default 1024).
///
/// Requires feature `gateway`.
pub struct Gateway<
    const L: usize = 4,
    const T: usize = 32,
    const B: usize = 2048,
    C: Connector = MultiConnector,
    const A: usize = 1024,
> {
    inner: GatewayInner<L, T, B, A>,
    connector: C,
}

impl<const L: usize, const T: usize, const B: usize, C: Connector, const A: usize>
    Gateway<L, T, B, C, A>
{
    /// Build a gateway with the default connector. Available when `C: Default`
    /// — true for every connector in this crate (`UnixConnector`,
    /// `TcpConnector`, `MultiConnector`, `UdpConnector`).
    pub fn new() -> Self where C: Default { Self::with_connector(C::default()) }

    /// Build a gateway with an explicit connector (e.g. one carrying a TLS
    /// config). Use `new()` for the zero-sized default connectors.
    pub fn with_connector(connector: C) -> Self {
        Self { inner: GatewayInner::new(), connector }
    }

    /// Connect to a leaf, run the MCP handshake, discover its tools, and
    /// add them to the routing table. On error the gateway is rolled
    /// back — no leaf slot consumed, no partial routes.
    pub fn add_leaf(&mut self, addr: &str) -> Result<(), &'static str> {
        if addr.is_empty() { return Err("leaf address is empty"); }
        if addr.len() > LEAF_ADDR_MAX { return Err("leaf address exceeds LEAF_ADDR_MAX bytes"); }
        let mut resp = [0u8; 4096];
        let n = discover_leaf_tools(&self.connector, addr, &mut resp)?;
        let tools_arr = parse_tools_array(&resp[..n])?;
        self.inner.commit_leaf(addr, tools_arr)
    }

    /// Dispatch one JSON-RPC message. Returns 0 for notifications and
    /// truncated responses; otherwise the number of bytes written to `out`.
    pub fn handle(&self, msg: &[u8], out: &mut [u8]) -> usize {
        match classify(msg) {
            Msg::ParseError => write_parse_error(out),
            Msg::Notification => 0,
            Msg::Request { id, method, params } => {
                let mut w = Writer::new(out);
                match method {
                    br#""initialize""# => write_initialize_gateway(&mut w, id),
                    br#""tools/list""# => write_tools_list(&mut w, id, &self.inner),
                    br#""tools/call""# => self.dispatch_tools_call(params, id, &mut w),
                    _ => rpc_err(&mut w, id, -32601, "method not found"),
                }
                if w.truncated { 0 } else { w.pos }
            }
        }
    }

    #[inline(never)]
    fn dispatch_tools_call(&self, params: Option<&[u8]>, id: &[u8], w: &mut Writer) {
        let Some((name_q, name, args)) = parse_tools_call_params(params) else {
            rpc_err(w, id, -32600, "bad params");
            return;
        };
        match self.inner.find_leaf_id(name) {
            Some(leaf_id) => {
                let leaf_addr = self.inner.leaf_addr(leaf_id as usize);
                let start = w.pos;
                if let Err(e) = proxy_call(&self.connector, leaf_addr, name_q, args, id, w) {
                    w.pos = start;
                    rpc_err(w, id, -1, e);
                }
            }
            None => rpc_err(w, id, -32601, "unknown tool"),
        }
    }
}

impl<const L: usize, const T: usize, const B: usize, C: Connector + Default, const A: usize>
    Default for Gateway<L, T, B, C, A>
{
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// DynamicGateway — multi-threaded variant with live add/remove + push.
//
// State lives behind an `RwLock` so leaves can be added or removed at any
// time, including while clients are connected. On every successful mutation
// the gateway broadcasts `notifications/tools/list_changed` to every client
// registered through [`DynamicGateway::serve_conn`] /
// [`DynamicGateway::serve_unix`] / [`DynamicGateway::serve_tcp`].
//
// Costs vs the static [`Gateway`]:
// - One `RwLock<GatewayInner>` (~32 B platform-dependent)
// - One `Mutex<Vec<Arc<Subscriber>>>` for the subscriber list
// - One `AtomicU64` for the epoch counter
// - Heap allocations on the connection-registration path (Arc, Box, Vec growth)
// - Per-mutation broadcast: zero heap when subscribers ≤ 4 (inline storage),
//   one Vec allocation only when more clients are simultaneously connected
//
// All locking happens off the hot read path: tool calls take a brief read
// lock to copy the leaf address out, then proxy with the lock released so
// in-flight calls don't block mutators.
// ---------------------------------------------------------------------------

/// A connected client subscribed to `notifications/tools/list_changed`.
/// `writer` is a clone of the connection's write half; it serializes
/// response writes (from the connection thread) with broadcast writes
/// (from the mutator thread) so neither can splice bytes into the other's
/// outbound JSON-RPC frame.
struct Subscriber {
    writer: Mutex<Box<dyn IoWrite + Send>>,
    alive: AtomicBool,
}

/// Inline storage capacity for the broadcast snapshot. Sized to the typical
/// edge gateway client count — one MCP host process is the common case,
/// 4 covers any realistic local fan-out. Beyond this, broadcast spills to
/// a heap-allocated `Vec<Arc<Subscriber>>`.
const BROADCAST_INLINE: usize = 4;

/// Multi-threaded gateway with live add/remove of leaves and push
/// `notifications/tools/list_changed` to every connected client. See
/// [`Gateway`] for the comparison table and use that one if you don't need
/// runtime mutation — it's smaller and lock-free.
pub struct DynamicGateway<
    const L: usize = 4,
    const T: usize = 32,
    const B: usize = 2048,
    C: Connector = MultiConnector,
    const A: usize = 1024,
> {
    state: RwLock<GatewayInner<L, T, B, A>>,
    subscribers: Mutex<Vec<Arc<Subscriber>>>,
    epoch: AtomicU64,
    connector: C,
}

impl<const L: usize, const T: usize, const B: usize, C: Connector, const A: usize>
    DynamicGateway<L, T, B, C, A>
{
    pub fn new() -> Self where C: Default { Self::with_connector(C::default()) }

    pub fn with_connector(connector: C) -> Self {
        Self {
            state: RwLock::new(GatewayInner::new()),
            subscribers: Mutex::new(Vec::new()),
            epoch: AtomicU64::new(0),
            connector,
        }
    }

    /// Monotonic counter bumped on every successful add/remove. Subscribers
    /// can use it to detect "did anything change since I last looked?"
    pub fn epoch(&self) -> u64 { self.epoch.load(Ordering::Acquire) }

    /// Connect to a leaf, run the MCP handshake, discover its tools, and
    /// add them to the routing table. Safe to call before or during
    /// `serve_unix` / `serve_tcp`. On success, broadcasts
    /// `notifications/tools/list_changed` to every active subscriber.
    pub fn add_leaf(&self, addr: &str) -> Result<(), &'static str> {
        if addr.is_empty() { return Err("leaf address is empty"); }
        if addr.len() > LEAF_ADDR_MAX { return Err("leaf address exceeds LEAF_ADDR_MAX bytes"); }
        // Discovery happens outside the lock — connecting may take seconds.
        let mut resp = [0u8; 4096];
        let n = discover_leaf_tools(&self.connector, addr, &mut resp)?;
        let tools_arr = parse_tools_array(&resp[..n])?;
        {
            let mut inner = self.state.write().unwrap();
            inner.commit_leaf(addr, tools_arr)?;
        }
        self.bump_epoch_and_broadcast();
        Ok(())
    }

    /// Remove a previously-added leaf by address. In-flight `tools/call`
    /// operations targeting this leaf are not interrupted (the proxy
    /// connection is already open). On success, broadcasts
    /// `notifications/tools/list_changed`.
    pub fn remove_leaf(&self, addr: &str) -> Result<(), &'static str> {
        {
            let mut inner = self.state.write().unwrap();
            inner.remove_leaf_inner(addr)?;
        }
        self.bump_epoch_and_broadcast();
        Ok(())
    }

    fn bump_epoch_and_broadcast(&self) {
        self.epoch.fetch_add(1, Ordering::Release);
        let subs_lock = self.subscribers.lock().unwrap();
        if subs_lock.is_empty() { return; }
        // Snapshot the live subscribers so per-subscriber writes happen
        // with the outer lock dropped. Inline storage holds the typical
        // case (≤4 subscribers) on the stack, with `overflow` only allocating
        // when more clients are simultaneously connected. Saves a heap
        // allocation per mutation in the common case.
        let mut inline: [Option<Arc<Subscriber>>; BROADCAST_INLINE] =
            std::array::from_fn(|_| None);
        let mut inline_count = 0usize;
        let mut overflow: Vec<Arc<Subscriber>> = Vec::new();
        for s in subs_lock.iter() {
            if !s.alive.load(Ordering::Acquire) { continue; }
            if inline_count < BROADCAST_INLINE {
                inline[inline_count] = Some(s.clone());
                inline_count += 1;
            } else {
                overflow.push(s.clone());
            }
        }
        drop(subs_lock);
        for sub in inline[..inline_count].iter().flatten().chain(overflow.iter()) {
            let mut w = match sub.writer.lock() {
                Ok(w) => w,
                Err(p) => p.into_inner(),
            };
            if w.write_all(TOOLS_LIST_CHANGED_NOTIFY).is_err() {
                sub.alive.store(false, Ordering::Release);
            }
        }
    }

    /// Dispatch one JSON-RPC message. Returns 0 for notifications and
    /// truncated responses.
    pub fn handle(&self, msg: &[u8], out: &mut [u8]) -> usize {
        match classify(msg) {
            Msg::ParseError => write_parse_error(out),
            Msg::Notification => 0,
            Msg::Request { id, method, params } => {
                let mut w = Writer::new(out);
                match method {
                    br#""initialize""# => write_initialize_gateway(&mut w, id),
                    br#""tools/list""# => {
                        let inner = self.state.read().unwrap();
                        write_tools_list(&mut w, id, &inner);
                    }
                    br#""tools/call""# => self.dispatch_tools_call(params, id, &mut w),
                    _ => rpc_err(&mut w, id, -32601, "method not found"),
                }
                if w.truncated { 0 } else { w.pos }
            }
        }
    }

    #[inline(never)]
    fn dispatch_tools_call(&self, params: Option<&[u8]>, id: &[u8], w: &mut Writer) {
        let Some((name_q, name, args)) = parse_tools_call_params(params) else {
            rpc_err(w, id, -32600, "bad params");
            return;
        };
        // Copy the leaf address out under the read lock so the proxy_call
        // (which may take seconds) doesn't block writers.
        let mut addr_buf = [0u8; LEAF_ADDR_MAX];
        let addr_len = {
            let inner = self.state.read().unwrap();
            match inner.find_leaf_id(name) {
                Some(leaf_id) => {
                    let s = inner.leaf_addr(leaf_id as usize).as_bytes();
                    addr_buf[..s.len()].copy_from_slice(s);
                    Some(s.len())
                }
                None => None,
            }
        };
        match addr_len {
            Some(len) => {
                // SAFETY: bytes copied from a verified UTF-8 &str above.
                let leaf_addr = unsafe { core::str::from_utf8_unchecked(&addr_buf[..len]) };
                let start = w.pos;
                if let Err(e) = proxy_call(&self.connector, leaf_addr, name_q, args, id, w) {
                    w.pos = start;
                    rpc_err(w, id, -1, e);
                }
            }
            None => rpc_err(w, id, -32601, "unknown tool"),
        }
    }
}

impl<const L: usize, const T: usize, const B: usize, C: Connector + Default, const A: usize>
    Default for DynamicGateway<L, T, B, C, A>
{
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Shared dispatch helpers — used by both Gateway and DynamicGateway.
//
// These are free functions / inner-state methods rather than trait methods
// so each variant gets its own monomorphization. There is no vtable on the
// hot path.
// ---------------------------------------------------------------------------

#[inline]
fn write_tools_list<const L: usize, const T: usize, const B: usize, const A: usize>(
    w: &mut Writer,
    id: &[u8],
    inner: &GatewayInner<L, T, B, A>,
) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).push(id).s(r#","result":{"tools":["#);
    w.push(&inner.tools_json[..inner.tools_json_len as usize]);
    w.s("]}}").nl();
}

/// Single-pass parse of a `tools/call` `params` object. Walks the bytes
/// once to extract `name` (as `(name_quoted, name_unquoted)`) and
/// `arguments`. Returns `None` on missing/malformed `name`.
///
/// `name_quoted` is the raw `"..."` JSON value including the surrounding
/// quotes — used verbatim when proxying to a leaf so any client-side
/// escapes round-trip untouched.
fn parse_tools_call_params(params: Option<&[u8]>) -> Option<(&[u8], &[u8], &[u8])> {
    let p = params?;
    let mut name_q: Option<&[u8]> = None;
    let mut args: Option<&[u8]> = None;
    walk_obj_for(p, |k, v| match k {
        b"name" => name_q = Some(v),
        b"arguments" => args = Some(v),
        _ => {}
    });
    let name_q = name_q?;
    let name = unquote(name_q)?;
    Some((name_q, name, args.unwrap_or(b"{}")))
}

/// Run the MCP handshake (`initialize` + `tools/list`) against a leaf
/// using `connector`, writing the tools/list response into `resp`. Shared
/// between `Gateway::add_leaf` and `DynamicGateway::add_leaf`.
fn discover_leaf_tools<C: Connector>(
    connector: &C,
    addr: &str,
    resp: &mut [u8; 4096],
) -> Result<usize, &'static str> {
    let mut conn = connector.connect(addr)?;
    conn.write_all(INIT_MSG).map_err(|_| "write failed")?;
    let init_n = read_line(&mut conn, resp).ok_or("no initialize response")?;
    if obj_get(&resp[..init_n], b"error").is_some() {
        return Err("leaf returned error to initialize");
    }
    conn.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .map_err(|_| "write failed")?;
    read_line(&mut conn, resp).ok_or("no tools/list response")
}

#[inline]
fn parse_tools_array(resp: &[u8]) -> Result<&[u8], &'static str> {
    let result = obj_get(resp, b"result").ok_or("no result in leaf response")?;
    obj_get(result, b"tools").ok_or("no tools in leaf response")
}

/// Proxy a single `tools/call` to a leaf via `connector`. Free function so
/// both `Gateway` and `DynamicGateway` can call it without any trait-object
/// dispatch — `C` monomorphizes per call site.
///
/// Single buffer serves both the outbound request and inbound response.
/// Response parsing walks the leaf's reply once (instead of two `obj_get`
/// calls) by capturing both `result` and `error` in the same pass.
fn proxy_call<C: Connector>(
    connector: &C,
    leaf_addr: &str,
    tool_quoted: &[u8],
    args: &[u8],
    req_id: &[u8],
    w: &mut Writer,
) -> Result<(), &'static str> {
    let mut buf = [0u8; 1024];
    let req_n = {
        let mut rw = Writer::new(&mut buf);
        rw.s(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"#)
          .push(tool_quoted).s(r#","arguments":"#).push(args).s("}}\n");
        if rw.truncated { return Err("args too large for proxy buffer"); }
        rw.pos
    };
    let mut conn = connector.connect(leaf_addr)?;
    conn.write_all(&buf[..req_n]).map_err(|_| "leaf write failed")?;
    let rn = read_line(&mut conn, &mut buf).ok_or("no response from leaf")?;
    let resp = &buf[..rn];

    // Walk the response once to find result/error/id in a single pass.
    // `id` from the leaf is ignored — we echo the client's id back to
    // preserve their request/response correlation.
    let mut result: Option<&[u8]> = None;
    let mut error: Option<&[u8]> = None;
    walk_obj_for(resp, |k, v| match k {
        b"result" => result = Some(v),
        b"error" => error = Some(v),
        _ => {}
    });

    w.s(r#"{"jsonrpc":"2.0","id":"#).push(req_id);
    if let Some(res) = result {
        w.s(r#","result":"#).push(res);
    } else if let Some(err) = error {
        w.s(r#","error":"#).push(err);
    } else {
        w.s(r#","error":{"code":-1,"message":"leaf returned no result"}"#);
    }
    w.s("}").nl();
    Ok(())
}

// ---------------------------------------------------------------------------
// TryClone — small abstraction over UnixStream / TcpStream try_clone() so the
// generic `serve_conn` can split a connection into independent read/write
// halves over the same fd. Both halves can be used concurrently from
// different threads (kernel-side, the syscalls are independent on a duplicated
// fd).
// ---------------------------------------------------------------------------

pub trait TryClone: Sized {
    fn try_clone(&self) -> std::io::Result<Self>;
    fn set_write_timeout(&self, dur: Option<std::time::Duration>) -> std::io::Result<()>;
}

impl TryClone for UnixStream {
    fn try_clone(&self) -> std::io::Result<Self> { UnixStream::try_clone(self) }
    fn set_write_timeout(&self, dur: Option<std::time::Duration>) -> std::io::Result<()> {
        UnixStream::set_write_timeout(self, dur)
    }
}

impl TryClone for TcpStream {
    fn try_clone(&self) -> std::io::Result<Self> { TcpStream::try_clone(self) }
    fn set_write_timeout(&self, dur: Option<std::time::Duration>) -> std::io::Result<()> {
        TcpStream::set_write_timeout(self, dur)
    }
}

// ---------------------------------------------------------------------------
// serve_* — gateway-aware connection serving with subscriber registration.
//
// Each connection gets its write half registered as a subscriber and drives
// the existing newline-delimited framing loop. Slow peers are bounded by the
// 2-second write timeout: a subscriber whose `write_all` times out is marked
// dead and skipped on subsequent broadcasts, never stalling the mutator.
// ---------------------------------------------------------------------------

const BROADCAST_WRITE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

impl<
    const L: usize,
    const T: usize,
    const B: usize,
    C: Connector + Send + Sync + 'static,
    const A: usize,
> DynamicGateway<L, T, B, C, A>
{
    /// Drive one already-accepted connection. Registers it as a subscriber,
    /// runs the read/write loop, and unregisters on exit. Spawn this in a
    /// thread per connection.
    pub fn serve_conn<S>(self: &Arc<Self>, conn: S)
    where
        S: Read + IoWrite + TryClone + Send + 'static,
    {
        let writer_half = match conn.try_clone() {
            Ok(w) => w,
            Err(_) => return,
        };
        let _ = writer_half.set_write_timeout(Some(BROADCAST_WRITE_TIMEOUT));
        let sub = Arc::new(Subscriber {
            writer: Mutex::new(Box::new(writer_half)),
            alive: AtomicBool::new(true),
        });
        self.subscribers.lock().unwrap().push(sub.clone());

        let gw = self.clone();
        run_gateway_connection(conn, &gw, &sub);

        sub.alive.store(false, Ordering::Release);
        // Lazy GC of dead subscribers — keeps the active list short.
        self.subscribers
            .lock()
            .unwrap()
            .retain(|s| s.alive.load(Ordering::Acquire));
    }

    /// Bind a Unix listener and spawn a thread per accepted connection,
    /// each running `serve_conn`. Blocks until the listener closes.
    pub fn serve_unix(self: &Arc<Self>, path: &str) {
        let _ = std::fs::remove_file(path);
        let listener = match UnixListener::bind(path) {
            Ok(l) => l,
            Err(_) => return,
        };
        for conn in listener.incoming().flatten() {
            let gw = self.clone();
            spawn_conn_thread(move || gw.serve_conn(conn));
        }
    }

    /// TCP variant of [`serve_unix`].
    pub fn serve_tcp(self: &Arc<Self>, addr: &str) {
        let listener = match TcpListener::bind(addr) {
            Ok(l) => l,
            Err(_) => return,
        };
        for conn in listener.incoming().flatten() {
            let gw = self.clone();
            spawn_conn_thread(move || gw.serve_conn(conn));
        }
    }
}

// Per-connection thread stack budget. Measured peak inside `serve_conn` →
// `run_gateway_connection` → `gw.handle` → `dispatch_tools_call` →
// `proxy_call`: ~1 KiB rx + ~1 KiB tx + ~1 KiB proxy buffer + ~256 B of
// misc frames = roughly 3.5 KiB. The mutator-side `compact_arena` stack
// allocation lives on the *mutator* thread (whoever called `add_leaf` /
// `remove_leaf`), not the connection thread, so it doesn't enter this
// budget.
//
// 16 KiB leaves ~4× headroom for std/libc internals, signal-handler
// frames, and any future depth in the connector chain — while collapsing
// the default 2 MiB-per-thread reservation by ~128×. With 100 concurrent
// clients that's 1.6 MiB of address space instead of 200 MiB.
const CONN_THREAD_STACK: usize = 16 * 1024;

fn spawn_conn_thread<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    let _ = std::thread::Builder::new()
        .stack_size(CONN_THREAD_STACK)
        .spawn(f);
}

/// Per-connection read/write loop. Drives a single connection's framing
/// for `DynamicGateway`. All writes go through `sub.writer`'s `Mutex` so
/// a concurrent broadcast from `bump_epoch_and_broadcast` cannot interleave
/// its bytes with ours on the underlying socket. We read directly from
/// `conn` (the original fd) — reads on one fd and writes on the cloned fd
/// are independent at the kernel level, so reads don't block on broadcasts.
fn run_gateway_connection<
    const L: usize,
    const T: usize,
    const B: usize,
    C: Connector,
    const A: usize,
    S: Read,
>(mut conn: S, gw: &DynamicGateway<L, T, B, C, A>, sub: &Subscriber) {
    let mut framer: crate::Framer<1024, 1024> = crate::Framer::new();
    loop {
        let n = match conn.read(framer.rx_space()) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        let mut io_err = false;
        let framed = framer.commit(
            n,
            |msg, out| gw.handle(msg, out),
            |resp| {
                // Hold sub.writer for the whole response so a racing
                // broadcast can't splice its bytes into our frame.
                let mut w = match sub.writer.lock() {
                    Ok(w) => w,
                    Err(p) => p.into_inner(),
                };
                if w.write_all(resp).is_err() || w.flush().is_err() {
                    io_err = true;
                }
            },
        );
        if io_err || framed.is_err() {
            break;
        }
    }
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

    // The static `Gateway` exposes its `inner: GatewayInner` directly via
    // module-private access; tests below mutate it through `&mut gw.inner`.
    // The dynamic-mode tests (epoch, broadcast, locking) live further down
    // and use `DynamicGateway` instead.

    #[test]
    fn add_route_rejects_oversized_name() {
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        let too_long = vec![b'x'; NAME_MAX + 1];
        assert!(gw.inner.add_route(&too_long, b"", 0).is_err());
        assert!(gw.inner.add_route(b"", b"", 0).is_err());
    }

    #[test]
    fn add_route_rejects_duplicates() {
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        assert!(gw.inner.add_route(b"temp", b"", 0).is_ok());
        assert!(gw.inner.add_route(b"temp", b"", 1).is_err(), "duplicate must be rejected");
    }

    #[test]
    fn add_route_rejects_when_full() {
        let mut gw: Gateway<2, 2, 1024> = Gateway::new();
        assert!(gw.inner.add_route(b"a", b"", 0).is_ok());
        assert!(gw.inner.add_route(b"b", b"", 0).is_ok());
        assert!(gw.inner.add_route(b"c", b"", 0).is_err(), "T limit must be enforced");
    }

    #[test]
    fn add_route_rejects_when_arena_full() {
        // Arena sized to fit only 4 bytes total.
        let mut gw: Gateway<2, 4, 1024, MultiConnector, 4> = Gateway::new();
        assert!(gw.inner.add_route(b"abc", b"", 0).is_ok());
        assert!(gw.inner.add_route(b"xyz", b"", 0).is_err(),
            "second route must trip the arena (A) limit");
    }

    #[test]
    fn packed_storage_uses_actual_lengths() {
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        gw.inner.add_route(b"a", b"", 0).unwrap();
        gw.inner.add_route(b"bc", b"", 0).unwrap();
        assert_eq!(gw.inner.arena_used, 3, "1 + 2 bytes packed back-to-back (no desc)");
    }

    #[test]
    fn parse_scheme_dispatches_correctly() {
        assert_eq!(parse_scheme("unix:///tmp/x.sock"), Scheme::Unix("/tmp/x.sock"));
        assert_eq!(parse_scheme("tcp://10.0.0.5:9000"), Scheme::Tcp("10.0.0.5:9000"));
        assert_eq!(parse_scheme("tls://host:443"),    Scheme::Tls("host:443"));
        assert_eq!(parse_scheme("udp://10.0.0.6:9000"), Scheme::Udp("10.0.0.6:9000"));
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
    #[cfg(not(feature = "udp"))]
    fn multi_connector_returns_udp_error_when_feature_off() {
        let mc = MultiConnector;
        match mc.connect("udp://127.0.0.1:9000") {
            Ok(_) => panic!("udp scheme should not connect without feature"),
            Err(e) => assert_eq!(e, "udp feature not enabled"),
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
    fn rebuild_tools_json_rejects_overflow() {
        // B=64 is too small for even one realistic tool entry (~80+ bytes).
        let mut gw: Gateway<2, 4, 64> = Gateway::new();
        gw.inner.add_route(b"temp_read", b"Read temperature in Celsius", 0).unwrap();
        let r = gw.inner.rebuild_tools_json();
        assert!(r.is_err(), "must reject when tools_json buffer (B) is too small");
        assert_eq!(gw.inner.tools_json_len, 0, "rebuild leaves the cache empty on failure");
    }

    #[test]
    fn gateway_is_send_and_sync() {
        // Compile-time check: both gateway types must be Send + Sync so they
        // can be wrapped in Arc and shared across threads.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Gateway<4, 32, 2048>>();
        assert_send_sync::<DynamicGateway<4, 32, 2048>>();
        assert_send_sync::<Arc<DynamicGateway<4, 32, 2048>>>();
    }

    #[test]
    fn measure_gateway_size() {
        let s = std::mem::size_of::<Gateway>();
        let dyn_s = std::mem::size_of::<DynamicGateway>();
        // Static is `GatewayInner + ZST connector` — no locks, no atomics,
        // no Vec. Both gateways share the same `GatewayInner` so the bulk
        // of the bytes (the arena + tools_json + routes table) is identical.
        // Dynamic adds the synchronization primitives on top.
        //
        // Static < dynamic is the masterclass invariant: an embedded user
        // who picks `Gateway` MUST not be paying for dynamic-mode plumbing.
        // The exact gap depends on platform `RwLock`/`Mutex` overhead
        // (~50–150 B on Linux/macOS) so we just assert ordering plus a
        // sanity envelope.
        assert!(
            (3200..3600).contains(&s),
            "Gateway size {s} drifted from expected envelope (~3.4 KiB); \
             update README RAM table and this assertion together"
        );
        assert!(
            (3400..4200).contains(&dyn_s),
            "DynamicGateway size {dyn_s} drifted from expected envelope (~3.5 KiB)"
        );
        assert!(
            s < dyn_s,
            "static Gateway ({s}) MUST be smaller than DynamicGateway ({dyn_s}) — \
             dynamic-mode fields leaked into the static type"
        );
    }

    #[test]
    fn rebuild_failure_preserves_existing_cache() {
        // Regression: an in-place rebuild that fails halfway used to leave
        // tools_json[..tools_json_len] partially overwritten. The pre-flight
        // size pass is what prevents that.
        let mut gw: Gateway<2, 4, 80> = Gateway::new();
        gw.inner.add_route(b"a", b"", 0).unwrap();
        gw.inner.rebuild_tools_json().unwrap();
        let saved_len = gw.inner.tools_json_len;
        let saved_bytes: Vec<u8> = gw.inner.tools_json[..saved_len as usize].to_vec();
        assert!(saved_len > 0, "first rebuild produced bytes");

        gw.inner.add_route(b"b", b"", 0).unwrap();
        let r = gw.inner.rebuild_tools_json();
        assert!(r.is_err());
        assert_eq!(gw.inner.tools_json_len, saved_len, "len preserved on failed rebuild");
        assert_eq!(
            &gw.inner.tools_json[..saved_len as usize],
            saved_bytes.as_slice(),
            "bytes preserved on failed rebuild"
        );
    }

    #[test]
    fn gateway_initialize_declares_list_changed_true() {
        // Both Gateway variants advertise tools.listChanged: true so clients
        // know they MIGHT receive list_changed pushes (Gateway never sends
        // them; DynamicGateway does — but the wire capability is the same).
        let gw: Gateway<2, 4, 1024> = Gateway::new();
        let mut out = [0u8; 256];
        let n = gw.handle(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            &mut out,
        );
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains(r#""listChanged":true"#), "{s}");
    }

    #[test]
    fn remove_leaf_clears_routes_and_rebuilds() {
        // Drives the inner directly through Gateway::inner (no lock needed).
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        gw.inner.commit_leaf(
            "leaf-a",
            br#"[{"name":"a1","description":""},{"name":"a2","description":""}]"#,
        ).unwrap();
        gw.inner.commit_leaf(
            "leaf-b",
            br#"[{"name":"b1","description":""}]"#,
        ).unwrap();
        assert_eq!(gw.inner.route_count, 3);
        gw.inner.remove_leaf_inner("leaf-a").unwrap();
        assert!(gw.inner.routes[..gw.inner.route_count as usize].iter().any(|r| r.is_empty()));
        let active: Vec<&[u8]> = gw.inner.routes[..gw.inner.route_count as usize]
            .iter()
            .filter(|r| !r.is_empty())
            .map(|r| gw.inner.route_name(r))
            .collect();
        assert_eq!(active, vec![&b"b1"[..]]);

        let mut out = [0u8; 512];
        let n = gw.handle(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("b1"), "{s}");
        assert!(!s.contains("\"a1\""), "{s}");
        assert!(!s.contains("\"a2\""), "{s}");
    }

    #[test]
    fn churn_reuses_empty_slots_and_arena() {
        // Regression: before slot reuse and arena compaction, repeated
        // add+remove cycles exhausted `L`/`T` (slot counters were high-water
        // marks that only ever grew) AND `A` (removed bytes lingered in the
        // arena forever). Each cycle here adds ~10 bytes; without compaction
        // we'd OOM the 32-byte arena after 3 cycles.
        let mut gw: Gateway<2, 4, 1024, MultiConnector, 32> = Gateway::new();
        for i in 0..50 {
            let leaf = format!("leaf-{i}");
            gw.inner.commit_leaf(&leaf, br#"[{"name":"t","description":""}]"#)
                .expect("churn add — slot+arena reuse should keep us under L/A");
            gw.inner.remove_leaf_inner(&leaf).unwrap();
            // After every full cycle, no entries should be active and the
            // arena should be empty.
            assert_eq!(gw.inner.arena_used, 0, "arena not compacted at iter {i}");
        }
        assert!(gw.inner.leaf_count <= 1, "leaf_count grew unbounded");
        assert!(gw.inner.route_count <= 1, "route_count grew unbounded");
    }

    #[test]
    fn re_add_after_remove_works() {
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        gw.inner.commit_leaf("leaf-x", br#"[{"name":"x1","description":""}]"#).unwrap();
        gw.inner.remove_leaf_inner("leaf-x").unwrap();
        gw.inner.commit_leaf("leaf-x", br#"[{"name":"x1","description":""}]"#).unwrap();
        let active: Vec<&[u8]> = gw.inner.routes[..gw.inner.route_count as usize]
            .iter()
            .filter(|r| !r.is_empty())
            .map(|r| gw.inner.route_name(r))
            .collect();
        assert_eq!(active, vec![&b"x1"[..]]);
    }

    #[test]
    fn response_writes_serialize_with_broadcasts() {
        struct CountingWriter(std::sync::Arc<std::sync::atomic::AtomicUsize>);
        impl IoWrite for CountingWriter {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.fetch_add(1, Ordering::Relaxed);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let writes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let gw: Arc<DynamicGateway<2, 4, 1024>> = Arc::new(DynamicGateway::new());
        gw.subscribers.lock().unwrap().push(Arc::new(Subscriber {
            writer: Mutex::new(Box::new(CountingWriter(writes.clone()))),
            alive: AtomicBool::new(true),
        }));
        {
            let mut i = gw.state.write().unwrap();
            i.commit_leaf("seed", br#"[{"name":"s1","description":""}]"#).unwrap();
        }
        let sub = gw.subscribers.lock().unwrap()[0].clone();
        let guard = sub.writer.lock().unwrap();
        let gw2 = gw.clone();
        let t = std::thread::spawn(move || {
            gw2.remove_leaf("seed").unwrap();
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(writes.load(Ordering::Relaxed), 0,
            "broadcast must not bypass the response-write mutex");
        drop(guard);
        t.join().unwrap();
        assert_eq!(writes.load(Ordering::Relaxed), 1,
            "broadcast wrote exactly once after we released");
    }

    #[test]
    fn epoch_increments_on_mutation() {
        let gw: DynamicGateway<2, 4, 1024> = DynamicGateway::new();
        {
            let mut i = gw.state.write().unwrap();
            i.commit_leaf("leaf-q", br#"[{"name":"q1","description":""}]"#).unwrap();
        }
        let before = gw.epoch();
        gw.remove_leaf("leaf-q").unwrap();
        assert_eq!(gw.epoch(), before + 1, "remove_leaf must bump the epoch");
    }

    #[test]
    fn broadcast_to_dead_subscriber_does_not_panic() {
        use std::io::{Error, ErrorKind};
        struct FailingWriter;
        impl IoWrite for FailingWriter {
            fn write(&mut self, _b: &[u8]) -> std::io::Result<usize> {
                Err(Error::new(ErrorKind::BrokenPipe, "test"))
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let gw: DynamicGateway<2, 4, 1024> = DynamicGateway::new();
        gw.subscribers.lock().unwrap().push(Arc::new(Subscriber {
            writer: Mutex::new(Box::new(FailingWriter)),
            alive: AtomicBool::new(true),
        }));
        {
            let mut i = gw.state.write().unwrap();
            i.commit_leaf("dead-leaf", br#"[{"name":"d1","description":""}]"#).unwrap();
        }
        gw.remove_leaf("dead-leaf").unwrap();
        let subs = gw.subscribers.lock().unwrap();
        assert!(!subs[0].alive.load(Ordering::Acquire), "failed write must mark subscriber dead");
    }

    #[test]
    fn remove_leaf_returns_err_for_unknown_addr() {
        let gw: DynamicGateway<2, 4, 1024> = DynamicGateway::new();
        assert!(gw.remove_leaf("never-added").is_err());
        assert_eq!(gw.epoch(), 0, "failed remove must not bump epoch");
    }

    #[test]
    fn concurrent_add_and_handle_does_not_torn_read() {
        use std::sync::atomic::AtomicUsize;
        let gw: Arc<DynamicGateway<4, 16, 4096>> = Arc::new(DynamicGateway::new());
        {
            let mut i = gw.state.write().unwrap();
            i.commit_leaf("seed", br#"[{"name":"s1","description":"d"}]"#).unwrap();
        }
        let stop = Arc::new(AtomicBool::new(false));
        let mismatches = Arc::new(AtomicUsize::new(0));

        let mutator = {
            let gw = gw.clone();
            let stop = stop.clone();
            std::thread::spawn(move || {
                let mut toggle = false;
                while !stop.load(Ordering::Acquire) {
                    let mut i = gw.state.write().unwrap();
                    if toggle {
                        let _ = i.commit_leaf(
                            "x",
                            br#"[{"name":"x1","description":""}]"#,
                        );
                    } else {
                        let _ = i.remove_leaf_inner("x");
                    }
                    toggle = !toggle;
                }
            })
        };

        let reader = {
            let gw = gw.clone();
            let stop = stop.clone();
            let mismatches = mismatches.clone();
            std::thread::spawn(move || {
                let mut out = [0u8; 1024];
                let mut iters = 0usize;
                while !stop.load(Ordering::Acquire) && iters < 200 {
                    let n = gw.handle(
                        br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
                        &mut out,
                    );
                    if n == 0 {
                        mismatches.fetch_add(1, Ordering::Relaxed);
                    } else {
                        let s = core::str::from_utf8(&out[..n]).unwrap_or("");
                        // Must always be a balanced JSON-RPC envelope.
                        if !s.contains("\"result\"") || !s.ends_with("}\n") {
                            mismatches.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    iters += 1;
                }
            })
        };

        std::thread::sleep(std::time::Duration::from_millis(50));
        stop.store(true, Ordering::Release);
        mutator.join().unwrap();
        reader.join().unwrap();
        assert_eq!(
            mismatches.load(Ordering::Acquire),
            0,
            "concurrent reads must always see well-formed JSON"
        );
    }

    #[test]
    fn fuzz_leaf_responses_never_panic_or_corrupt() {
        // A leaf is an untrusted device on a bus or a network. Everything it
        // returns is parsed by commit_leaf and echoed into tools/list.
        let mut st: u64 = 0x9E3779B97F4A7C15;
        let mut rnd = move || { st ^= st << 13; st ^= st >> 7; st ^= st << 17; st };
        let alpha: &[u8] = b"{}[]\":,namedscriptio0123456789tfnul \\\n\x00\xff";

        let seed = br#"[{"name":"t1","description":"d"},{"name":"t2","description":"d2"}]"#;
        let mut panics = 0usize;

        let run = |arr: &[u8], label: &str, panics: &mut usize| {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut gw: Gateway<2, 8, 512> = Gateway::new();
                let _ = gw.inner.commit_leaf("leaf", arr);
                // Whatever survived must still serialize as well-formed JSON.
                let mut out = [0u8; 1024];
                let n = gw.handle(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, &mut out);
                if n > 0 {
                    let s = core::str::from_utf8(&out[..n]).unwrap_or("");
                    assert!(s.ends_with("]}}\n"), "malformed tools/list: {s}");
                }
            }));
            if r.is_err() {
                *panics += 1;
                if *panics < 5 { println!("PANIC({label}): {:?}", String::from_utf8_lossy(arr)); }
            }
        };

        for _ in 0..30_000 {
            let len = (rnd() % 80) as usize;
            let arr: Vec<u8> = (0..len).map(|_| alpha[(rnd() % alpha.len() as u64) as usize]).collect();
            run(&arr, "random", &mut panics);
        }
        for cut in 0..seed.len() { run(&seed[..cut], "truncate", &mut panics); }
        for pos in 0..seed.len() {
            for &b in alpha {
                let mut m = seed.to_vec();
                m[pos] = b;
                run(&m, "mutate", &mut panics);
            }
        }
        // Hostile shapes aimed at the JSON writer specifically.
        let hostile: &[&[u8]] = &[
            br#"[{"name":"a\"","description":"d"}]"#,
            br#"[{"name":"a","description":"d\""}]"#,
            br#"[{"name":"a","description":"\\"}]"#,
            br#"[{"name":"","description":"d"}]"#,
            br#"[{"description":"no name"}]"#,
            br#"[{"name":"a"},{"name":"a"}]"#,
        ];
        for h in hostile { run(h, "hostile", &mut panics); }

        // A long description: the 255-byte cap cutting mid-escape.
        let mut long = Vec::from(&b"[{\"name\":\"a\",\"description\":\""[..]);
        long.extend(std::iter::repeat_n(b'x', 254));
        long.extend_from_slice(b"\\n");
        long.extend_from_slice(b"\"}]");
        run(&long, "long-desc", &mut panics);

        println!("total panics: {panics}");
        assert_eq!(panics, 0, "leaf responses must never panic or emit malformed JSON");
    }

    #[test]
    fn failed_add_after_remove_leaves_no_phantom_entries() {
        // Regression: rollback restored the counters but not the slot
        // contents. A reused slot sits below the high-water mark, so the
        // stale leaf and its routes stayed live, pointing at arena bytes the
        // next add_leaf overwrote. Symptoms were duplicate/garbage tools in
        // tools/list and a `leaf_addr` resolving to a leaf never added.
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        gw.inner.commit_leaf("leaf-a", br#"[{"name":"a1","description":""}]"#).unwrap();
        gw.inner.remove_leaf_inner("leaf-a").unwrap();

        // Fails partway: the second tool duplicates the first.
        let r = gw.inner.commit_leaf(
            "leaf-b",
            br#"[{"name":"b1","description":""},{"name":"b1","description":""}]"#,
        );
        assert!(r.is_err(), "duplicate within one leaf must fail");
        assert_eq!(gw.inner.leaves[0].len, 0, "failed add must not leave a leaf slot claimed");
        assert!(gw.inner.routes[0].is_empty(), "failed add must not leave a route behind");

        // A later add must not inherit the failed attempt's ghosts.
        gw.inner.commit_leaf("leaf-c", br#"[{"name":"c1","description":""}]"#).unwrap();
        let mut out = [0u8; 512];
        let n = gw.handle(br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert_eq!(s.matches(r#""name":"c1""#).count(), 1, "exactly one c1: {s}");
        assert!(!s.contains("b1"), "rolled-back route must not appear: {s}");
    }

    #[test]
    fn leaf_supplied_junk_is_rejected_not_re_emitted() {
        let mut gw: Gateway<2, 4, 1024> = Gateway::new();
        // Non-UTF-8 in a name or description would corrupt the whole response.
        assert!(gw.inner.add_route(b"a\xffb", b"d", 0).is_err(), "non-UTF-8 name");
        assert!(gw.inner.add_route(b"ab", b"d\xff", 0).is_err(), "non-UTF-8 description");
        // Over-long descriptions are rejected, not cut mid-escape.
        let long = vec![b'x'; 256];
        assert!(gw.inner.add_route(b"ab", &long, 0).is_err(), "256-byte description");
        // The boundary case still works.
        let ok = vec![b'x'; 255];
        assert!(gw.inner.add_route(b"ab", &ok, 0).is_ok(), "255 bytes is the limit, not an error");
    }
}
