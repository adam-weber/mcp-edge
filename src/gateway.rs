//! Gateway mode: aggregates tools from multiple leaf MCP devices.
//!
//! At startup, each leaf is queried for its tool list. The routing table
//! and tools-list JSON are built once and cached. Tool calls are proxied
//! to the owning leaf via a fresh connection (stateless). No heap — all
//! storage is in fixed arrays sized by const generics.
//!
//! # Memory layout (on a 32-bit target)
//!
//! `Gateway<L, T, B>` struct size:
//! - L × 109 bytes  — leaf socket paths (108 + 1 length byte each)
//! - T × 34 bytes   — routing table entries (32-byte name + 2 bytes indices)
//! - B bytes        — cached tools-list JSON
//! - 12 bytes       — three usize counters (4 bytes each on 32-bit)
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
use std::os::unix::net::UnixStream;

const PATH_MAX: usize = 108; // UNIX_PATH_MAX
const NAME_MAX: usize = 32;
const INIT_MSG: &[u8] = b"{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\"}\n";

// ---------------------------------------------------------------------------
// Internal storage — all Copy so they live in fixed arrays, no heap
// ---------------------------------------------------------------------------

#[derive(Copy, Clone)]
struct LeafAddr {
    path: [u8; PATH_MAX],
    len: u8,
}

impl LeafAddr {
    const EMPTY: Self = Self { path: [0u8; PATH_MAX], len: 0 };

    fn as_str(&self) -> &str {
        // SAFETY: the bytes were copied from a `&str` in `add_leaf`, so they
        // are valid UTF-8. Skipping validation avoids a per-call linear scan
        // of the path on the hot tools/call path.
        unsafe { core::str::from_utf8_unchecked(&self.path[..self.len as usize]) }
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
///
/// Requires feature `gateway`:
/// ```toml
/// mcp-edge = { version = "*", features = ["gateway"] }
/// ```
pub struct Gateway<const L: usize = 4, const T: usize = 32, const B: usize = 2048> {
    leaves: [LeafAddr; L],
    leaf_count: usize,
    routes: [Route; T],
    route_count: usize,
    /// Comma-separated tool JSON (no outer `[` `]` — those are added in handle).
    tools_json: [u8; B],
    tools_json_len: usize,
}

impl<const L: usize, const T: usize, const B: usize> Gateway<L, T, B> {
    pub fn new() -> Self {
        Self {
            leaves: [LeafAddr::EMPTY; L],
            leaf_count: 0,
            routes: [Route::EMPTY; T],
            route_count: 0,
            tools_json: [0u8; B],
            tools_json_len: 0,
        }
    }

    /// Connect to a leaf, run the MCP handshake, discover its tools,
    /// and add them to the routing table. Called at startup, not in the hot path.
    ///
    /// On error the gateway may have partially registered some routes for this
    /// leaf — treat any error as fatal and abort startup rather than continuing.
    pub fn add_leaf(&mut self, path: &str) -> Result<(), &'static str> {
        if self.leaf_count >= L { return Err("leaf limit reached"); }
        if path.is_empty() { return Err("leaf path is empty"); }
        if path.len() > PATH_MAX { return Err("leaf path exceeds PATH_MAX bytes"); }

        // Discover tools before committing the leaf slot, so a failed
        // connection doesn't waste an index.
        let mut resp = [0u8; 4096]; // generous: tools/list response can be large
        let n = {
            let mut conn = UnixStream::connect(path).map_err(|_| "connect failed")?;
            conn.write_all(INIT_MSG).map_err(|_| "write failed")?;
            read_line(&mut conn, &mut resp).ok_or("no initialize response")?;

            conn.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
                .map_err(|_| "write failed")?;
            read_line(&mut conn, &mut resp).ok_or("no tools/list response")?
        };

        let result = obj_get(&resp[..n], b"result").ok_or("no result in leaf response")?;
        let tools_arr = obj_get(result, b"tools").ok_or("no tools in leaf response")?;

        // Commit the leaf slot only after a successful connection.
        let leaf_idx = self.leaf_count as u8;
        let path_bytes = path.as_bytes();
        // path.len() <= PATH_MAX checked above, so the copy fits exactly.
        self.leaves[self.leaf_count].path[..path_bytes.len()].copy_from_slice(path_bytes);
        self.leaves[self.leaf_count].len = path_bytes.len() as u8;
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
        slot.len = name.len() as u8;
        slot.leaf = leaf;
        self.route_count += 1;
        Ok(())
    }

    fn append_tool_json(&mut self, name: &str, desc: &str) -> Result<(), &'static str> {
        let comma = if self.tools_json_len > 0 { 1 } else { 0 };
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

        match req.method {
            "initialize" => write_initialize(&mut w, req.id),
            "tools/list" => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).u(req.id).s(r#","result":{"tools":["#);
                w.push(&self.tools_json[..self.tools_json_len]);
                w.s("]}}").nl();
            }
            "tools/call" => {
                let params_raw = obj_get(msg, b"params").unwrap_or(b"{}");

                let Ok((tc, _)) = serde_json_core::from_slice::<crate::ToolCallParams>(params_raw)
                else {
                    rpc_err(&mut w, req.id, -32600, "bad params");
                    return w.pos;
                };

                let args = obj_get(params_raw, b"arguments").unwrap_or(b"{}");

                match self.find_leaf(tc.name) {
                    Some(idx) => {
                        let leaf_path = self.leaves[idx].as_str();
                        let start = w.pos;
                        if let Err(e) = proxy_call(leaf_path, tc.name, args, req.id, &mut w) {
                            w.pos = start;
                            rpc_err(&mut w, req.id, -1, e);
                        }
                    }
                    None => rpc_err(&mut w, req.id, -32601, "unknown tool"),
                }
            }
            _ => rpc_err(&mut w, req.id, -32601, "method not found"),
        }

        // See `Runtime::handle`: drop a truncated response instead of
        // forwarding malformed JSON to the client.
        if w.truncated { return 0; }
        w.pos
    }
}

impl<const L: usize, const T: usize, const B: usize> Default for Gateway<L, T, B> {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Proxy a single tool call to a leaf
// ---------------------------------------------------------------------------

fn proxy_call(
    leaf_path: &str,
    tool: &str,
    args: &[u8],
    req_id: u64,
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

    // Response is bounded by the leaf's OUT parameter (default 512 bytes of content
    // wrapped in ~80 bytes of JSON envelope).
    let mut resp_buf = [0u8; 1024];

    let mut conn = UnixStream::connect(leaf_path).map_err(|_| "leaf connect failed")?;

    // No per-call `initialize`: the leaf was verified once at startup in `add_leaf`
    // and our runtime is stateless, so we go straight to tools/call. This halves
    // the syscall count and round-trip latency of every proxied tool call.
    conn.write_all(&req_buf[..req_n]).map_err(|_| "leaf write failed")?;
    let rn = read_line(&mut conn, &mut resp_buf).ok_or("no response from leaf")?;

    let resp = &resp_buf[..rn];
    w.s(r#"{"jsonrpc":"2.0","id":"#).u(req_id);

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

// ---------------------------------------------------------------------------
// Direct line reader — reads straight into the caller's buffer.
//
// Each `proxy_call` is a request/response pair on a fresh connection, so we
// don't need a separate buffered reader: we can stream bytes directly into
// `out` and scan each chunk for '\n' with a vectorized `iter().position()`.
//
// Eliminates 256 bytes of stack and one memcpy per response vs. a buffered
// Reader. Returns the byte length before the newline.
// ---------------------------------------------------------------------------

fn read_line(conn: &mut UnixStream, out: &mut [u8]) -> Option<usize> {
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
    fn append_tool_json_rejects_overflow() {
        // B=64 is too small for even one realistic tool entry (~80+ bytes).
        let mut gw: Gateway<2, 4, 64> = Gateway::new();
        let r = gw.append_tool_json("temp_read", "Read temperature in Celsius");
        assert!(r.is_err(), "must reject when tools_json buffer (B) is too small");
        assert_eq!(gw.tools_json_len, 0, "no partial write on rejection");
    }
}
