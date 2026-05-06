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
//! - 256-byte request buffer + 1024-byte response buffer + 280-byte read buffer = **1.6 KB**.

use crate::{obj_get, rpc_err, skip_delimited, write_initialize, write_tool, Writer};
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
        core::str::from_utf8(&self.path[..self.len as usize]).unwrap_or("")
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
    pub fn add_leaf(&mut self, path: &str) -> Result<(), &'static str> {
        if self.leaf_count >= L { return Err("leaf limit reached"); }

        // Discover tools before committing the leaf slot, so a failed
        // connection doesn't waste an index.
        let mut resp = [0u8; 4096]; // generous: tools/list response can be large
        let n = {
            let mut conn = UnixStream::connect(path).map_err(|_| "connect failed")?;
            conn.write_all(INIT_MSG).map_err(|_| "write failed")?;
            Reader::new(&mut conn).read_line(&mut resp).ok_or("no initialize response")?;

            conn.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
                .map_err(|_| "write failed")?;
            Reader::new(&mut conn).read_line(&mut resp).ok_or("no tools/list response")?
        };

        let result = obj_get(&resp[..n], b"result").ok_or("no result in leaf response")?;
        let tools_arr = obj_get(result, b"tools").ok_or("no tools in leaf response")?;

        // Commit the leaf slot only after a successful connection.
        let leaf_idx = self.leaf_count as u8;
        let path_bytes = path.as_bytes();
        let plen = path_bytes.len().min(PATH_MAX);
        self.leaves[self.leaf_count].path[..plen].copy_from_slice(&path_bytes[..plen]);
        self.leaves[self.leaf_count].len = plen as u8;
        self.leaf_count += 1;

        // Iterate `[{...},{...}]` without allocating.
        let mut p = crate::sp(tools_arr, 0);
        if tools_arr.get(p) != Some(&b'[') { return Ok(()); }
        p += 1;

        loop {
            p = crate::sp(tools_arr, p);
            match tools_arr.get(p) {
                Some(b']') | None => break,
                Some(b',') => { p += 1; continue; }
                Some(b'{') => {}
                _ => break,
            }

            let obj_start = p;
            if skip_delimited(tools_arr, &mut p, b'{', b'}').is_none() { break; }
            let tool_obj = &tools_arr[obj_start..p];

            let Ok((tool, _)) = serde_json_core::from_slice::<LeafTool>(tool_obj) else {
                continue;
            };
            self.add_route(tool.name, leaf_idx);
            self.append_tool_json(tool.name, tool.description);
        }

        Ok(())
    }

    fn add_route(&mut self, name: &str, leaf: u8) {
        if self.route_count >= T { return; }
        // Slot is already zeroed from Gateway::new(); write fields directly.
        let slot = &mut self.routes[self.route_count];
        let n = name.len().min(NAME_MAX);
        slot.name[..n].copy_from_slice(&name.as_bytes()[..n]);
        slot.len = n as u8;
        slot.leaf = leaf;
        self.route_count += 1;
    }

    fn append_tool_json(&mut self, name: &str, desc: &str) {
        if self.tools_json_len > 0 && self.tools_json_len < B {
            self.tools_json[self.tools_json_len] = b',';
            self.tools_json_len += 1;
        }
        let mut w = Writer::new(&mut self.tools_json[self.tools_json_len..]);
        write_tool(&mut w, name, desc);
        self.tools_json_len += w.pos;
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

        let Ok((req, _)) = serde_json_core::from_slice::<crate::Req>(msg) else { return 0 };

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
                        if !proxy_call(leaf_path, tc.name, args, req.id, &mut w) {
                            w.pos = start;
                            rpc_err(&mut w, req.id, -1, "leaf unreachable");
                        }
                    }
                    None => rpc_err(&mut w, req.id, -32601, "unknown tool"),
                }
            }
            _ => rpc_err(&mut w, req.id, -32601, "method not found"),
        }

        w.pos
    }
}

impl<const L: usize, const T: usize, const B: usize> Default for Gateway<L, T, B> {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Proxy a single tool call to a leaf
// ---------------------------------------------------------------------------

fn proxy_call(leaf_path: &str, tool: &str, args: &[u8], req_id: u64, w: &mut Writer) -> bool {
    // Request is short: ~60 bytes fixed + tool name + args.
    let mut req_buf = [0u8; 256];
    let req_n = {
        let mut rw = Writer::new(&mut req_buf);
        rw.s(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":""#)
          .s(tool).s(r#"","arguments":"#).push(args).s("}}\n");
        rw.pos
    };

    // Response is bounded by the leaf's OUT parameter (default 512 bytes of content
    // wrapped in ~80 bytes of JSON envelope).
    let mut resp_buf = [0u8; 1024];

    let Ok(mut conn) = UnixStream::connect(leaf_path) else { return false };

    if conn.write_all(INIT_MSG).is_err() { return false; }
    if Reader::new(&mut conn).read_line(&mut resp_buf).is_none() { return false; }

    if conn.write_all(&req_buf[..req_n]).is_err() { return false; }
    let Some(rn) = Reader::new(&mut conn).read_line(&mut resp_buf) else { return false };

    let resp = &resp_buf[..rn];
    w.s(r#"{"jsonrpc":"2.0","id":"#).u(req_id);

    if let Some(res) = obj_get(resp, b"result") {
        w.s(r#","result":"#).push(res);
    } else if let Some(err) = obj_get(resp, b"error") {
        w.s(r#","error":"#).push(err);
    } else {
        w.s(r#","error":{"code":-1,"message":"leaf error"}"#);
    }
    w.s("}").nl();
    true
}

// ---------------------------------------------------------------------------
// Buffered line reader — bulk reads to avoid one syscall per byte
// ---------------------------------------------------------------------------

struct Reader<'a> {
    conn:  &'a mut UnixStream,
    buf:   [u8; 256],
    start: usize,
    end:   usize,
}

impl<'a> Reader<'a> {
    fn new(conn: &'a mut UnixStream) -> Self {
        Self { conn, buf: [0u8; 256], start: 0, end: 0 }
    }

    fn read_line(&mut self, out: &mut [u8]) -> Option<usize> {
        let mut pos = 0;
        loop {
            if self.start >= self.end {
                self.start = 0;
                self.end = self.conn.read(&mut self.buf).ok().filter(|&n| n > 0)?;
            }
            let b = self.buf[self.start];
            self.start += 1;
            if b == b'\n' { return Some(pos); }
            if pos < out.len() { out[pos] = b; pos += 1; }
        }
    }
}

// sp, eat_str, and skip_delimited come from crate:: (lib.rs, pub(crate))
// No local reimplementation needed.
