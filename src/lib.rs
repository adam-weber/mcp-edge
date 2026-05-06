//! mcp-edge: minimal MCP runtime for resource-constrained devices.
//!
//! Zero heap allocation. `no_std`-compatible core. Runs on 64 KB RAM.
//!
//! # Tuning for your device
//!
//! `Runtime<'p, N, OUT>`:
//! - `N`   — max number of registered providers (default 8)
//! - `OUT` — max bytes a single tool call may produce (default 512)
//!
//! Total stack per `handle()` call: `OUT` + ~64 bytes overhead.
//!
//! # Example
//!
//! ```rust
//! use mcp_edge::{Provider, Runtime, Tool, ToolResult};
//!
//! struct PingSensor;
//! impl Provider for PingSensor {
//!     fn tools(&self) -> &[Tool] {
//!         &[Tool { name: "ping", description: "Returns pong" }]
//!     }
//!     fn call(&self, _tool: &str, _params: &str, out: &mut [u8]) -> (ToolResult, usize) {
//!         let s = b"pong";
//!         out[..s.len()].copy_from_slice(s);
//!         (ToolResult::Ok, s.len())
//!     }
//! }
//!
//! let sensor = PingSensor;
//! let mut rt: Runtime<'_, 1> = Runtime::new();
//! rt.register(&sensor);
//!
//! let mut out = [0u8; 256];
//! let n = rt.handle(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}", &mut out);
//! assert!(n > 0);
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A tool exposed to connected agents.
#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
}

/// Outcome of a tool call. The content itself is written into the `out` buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolResult {
    Ok,
    Err,
}

/// Implement this for each capability your device exposes (sensors, actuators, …).
///
/// All methods take `&self` — providers may be plain statics or stack values.
/// No `Box`, no `Arc`, no allocator.
pub trait Provider {
    fn tools(&self) -> &[Tool];

    /// Call a tool. Write the result into `out`; return the outcome and bytes written.
    ///
    /// `params` is the raw JSON arguments object, e.g. `{"pin": 3}`.
    fn call(&self, tool: &str, params: &str, out: &mut [u8]) -> (ToolResult, usize);
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// The MCP runtime.
///
/// - `N`   — maximum number of registered providers
/// - `OUT` — maximum bytes a single tool result may occupy
///
/// All storage is on the stack. No heap, no allocator required.
pub struct Runtime<'p, const N: usize = 8, const OUT: usize = 512> {
    providers: [Option<&'p dyn Provider>; N],
    count: usize,
}

impl<'p, const N: usize, const OUT: usize> Runtime<'p, N, OUT> {
    pub fn new() -> Self {
        Self { providers: [None; N], count: 0 }
    }

    /// Register a provider. Panics if more than `N` providers are registered.
    pub fn register(&mut self, p: &'p dyn Provider) {
        assert!(self.count < N, "exceeded provider limit");
        self.providers[self.count] = Some(p);
        self.count += 1;
    }

    /// Dispatch one newline-terminated JSON-RPC message.
    ///
    /// Writes a newline-terminated response into `out`. Returns bytes written.
    /// Returns 0 if the message cannot be parsed (no `id` field).
    pub fn handle(&self, msg: &[u8], out: &mut [u8]) -> usize {
        let mut w = Writer::new(out);

        let Some(id) = extract_u64(msg, b"\"id\":") else { return 0 };

        match extract_str(msg, b"\"method\":") {
            Some(b"initialize") => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).u(id)
                 .s(r#","result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}}}}"#)
                 .nl();
            }
            Some(b"tools/list") => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).u(id).s(r#","result":{"tools":["#);
                let mut first = true;
                for i in 0..self.count {
                    for t in self.providers[i].unwrap().tools() {
                        if !first { w.s(","); }
                        first = false;
                        w.s(r#"{"name":""#).s(t.name)
                         .s(r#"","description":""#).esc(t.description)
                         .s(r#"","inputSchema":{"type":"object"}}"#);
                    }
                }
                w.s("]}}").nl();
            }
            Some(b"tools/call") => {
                let name = extract_str(msg, b"\"name\":").unwrap_or(b"");
                let args = extract_obj(msg, b"\"arguments\":").unwrap_or(b"{}");
                let name = core::str::from_utf8(name).unwrap_or("");
                let args = core::str::from_utf8(args).unwrap_or("{}");

                let mut tool_out = [0u8; OUT];
                match self.find(name) {
                    Some(p) => {
                        let (res, n) = p.call(name, args, &mut tool_out);
                        let text = core::str::from_utf8(&tool_out[..n]).unwrap_or("");
                        w.s(r#"{"jsonrpc":"2.0","id":"#).u(id);
                        if res == ToolResult::Ok {
                            w.s(r#","result":{"content":[{"type":"text","text":""#)
                             .esc(text).s(r#""}]}}"#);
                        } else {
                            w.s(r#","error":{"code":-1,"message":""#).esc(text).s(r#""}}"#);
                        }
                        w.nl();
                    }
                    None => rpc_err(&mut w, id, -32601, "unknown tool"),
                }
            }
            _ => rpc_err(&mut w, id, -32601, "method not found"),
        }

        w.pos
    }

    fn find(&self, tool: &str) -> Option<&dyn Provider> {
        for i in 0..self.count {
            let p = self.providers[i].unwrap();
            if p.tools().iter().any(|t| t.name == tool) {
                return Some(p);
            }
        }
        None
    }
}

impl<'p, const N: usize, const OUT: usize> Default for Runtime<'p, N, OUT> {
    fn default() -> Self { Self::new() }
}

fn rpc_err(w: &mut Writer, id: u64, code: i32, msg: &str) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).u(id)
     .s(r#","error":{"code":"#).i(code).s(r#","message":""#)
     .esc(msg).s(r#""}}"#).nl();
}

// ---------------------------------------------------------------------------
// Stack-only JSON writer — fills a caller-provided buffer, never allocates
// ---------------------------------------------------------------------------

struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self { Self { buf, pos: 0 } }

    fn push(&mut self, bytes: &[u8]) -> &mut Self {
        let n = bytes.len().min(self.buf.len().saturating_sub(self.pos));
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
        self
    }

    fn s(&mut self, s: &str) -> &mut Self { self.push(s.as_bytes()) }
    fn nl(&mut self) -> &mut Self { self.push(b"\n") }

    fn u(&mut self, n: u64) -> &mut Self {
        let mut tmp = [0u8; 20];
        self.push(fmt_u64(n, &mut tmp))
    }

    fn i(&mut self, n: i32) -> &mut Self {
        if n < 0 { self.push(b"-").u(n.unsigned_abs() as u64) } else { self.u(n as u64) }
    }

    fn esc(&mut self, s: &str) -> &mut Self {
        for b in s.bytes() {
            match b {
                b'"'  => { self.push(b"\\\""); }
                b'\\' => { self.push(b"\\\\"); }
                b'\n' => { self.push(b"\\n"); }
                b'\r' => { self.push(b"\\r"); }
                b'\t' => { self.push(b"\\t"); }
                _     => { self.push(core::slice::from_ref(&b)); }
            }
        }
        self
    }
}

fn fmt_u64(n: u64, buf: &mut [u8; 20]) -> &[u8] {
    if n == 0 { buf[19] = b'0'; return &buf[19..]; }
    let mut pos = 20usize;
    let mut v = n;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    &buf[pos..]
}

// ---------------------------------------------------------------------------
// Zero-copy JSON extraction — returns byte slices into the input, no allocation
// ---------------------------------------------------------------------------

fn extract_str<'a>(json: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let pos = memmem(json, key)? + key.len();
    let rest = ltrim(&json[pos..]);
    if *rest.first()? != b'"' { return None; }
    let inner = &rest[1..];
    let mut i = 0;
    while i < inner.len() {
        match inner[i] {
            b'\\' => i += 2,
            b'"'  => return Some(&inner[..i]),
            _     => i += 1,
        }
    }
    None
}

fn extract_u64(json: &[u8], key: &[u8]) -> Option<u64> {
    let pos = memmem(json, key)? + key.len();
    let rest = ltrim(&json[pos..]);
    let end = rest.iter().position(|b| !b.is_ascii_digit()).unwrap_or(rest.len());
    if end == 0 { return None; }
    let mut n = 0u64;
    for &b in &rest[..end] {
        n = n.saturating_mul(10).saturating_add((b - b'0') as u64);
    }
    Some(n)
}

fn extract_obj<'a>(json: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let pos = memmem(json, key)? + key.len();
    let rest = ltrim(&json[pos..]);
    if *rest.first()? != b'{' { return None; }
    let mut depth = 0usize;
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            b'{' => depth += 1,
            b'}' => { depth -= 1; if depth == 0 { return Some(&rest[..=i]); } }
            b'"' => {
                i += 1;
                while i < rest.len() {
                    match rest[i] { b'\\' => i += 1, b'"' => break, _ => {} }
                    i += 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn memmem(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn ltrim(s: &[u8]) -> &[u8] {
    let n = s.iter().position(|&b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
              .unwrap_or(s.len());
    &s[n..]
}

// ---------------------------------------------------------------------------
// Unix socket transport — std-only, for local development and testing
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
pub mod transport {
    use super::Runtime;
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    /// Unix socket transport. Serves one client at a time.
    /// Uses stack-allocated I/O buffers — no heap in the read/write loop.
    pub struct UnixTransport {
        path: &'static str,
    }

    impl UnixTransport {
        pub fn new(path: &'static str) -> Self {
            let _ = std::fs::remove_file(path);
            Self { path }
        }

        pub fn serve<'p, const N: usize, const OUT: usize>(&self, rt: &Runtime<'p, N, OUT>) {
            let listener = UnixListener::bind(self.path).expect("bind failed");
            eprintln!("listening on {}", self.path);
            for conn in listener.incoming().flatten() {
                let mut rx = [0u8; 1024];
                let mut tx = [0u8; 1024];
                let mut rx_pos = 0usize;
                let mut conn = conn;
                loop {
                    let mut byte = [0u8; 1];
                    if conn.read_exact(&mut byte).is_err() { break; }
                    if byte[0] == b'\n' {
                        if rx_pos > 0 {
                            let n = rt.handle(&rx[..rx_pos], &mut tx);
                            if conn.write_all(&tx[..n]).is_err() { break; }
                        }
                        rx_pos = 0;
                    } else if rx_pos < rx.len() {
                        rx[rx_pos] = byte[0];
                        rx_pos += 1;
                    }
                    // if rx_pos == rx.len(): message too large, discard until next newline
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    struct Stub {
        tools: &'static [Tool],
        value: &'static [u8],
    }

    impl Provider for Stub {
        fn tools(&self) -> &[Tool] { self.tools }
        fn call(&self, _: &str, _: &str, out: &mut [u8]) -> (ToolResult, usize) {
            let n = self.value.len().min(out.len());
            out[..n].copy_from_slice(&self.value[..n]);
            (ToolResult::Ok, n)
        }
    }

    static SENSOR_TOOLS: &[Tool] = &[
        Tool { name: "temp_read", description: "Read temperature" },
    ];

    #[test]
    fn test_initialize() {
        static S: Stub = Stub { tools: SENSOR_TOOLS, value: b"22.5" };
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S);

        let mut out = [0u8; 512];
        let n = rt.handle(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}", &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("protocolVersion"), "{s}");
    }

    #[test]
    fn test_tools_list() {
        static S: Stub = Stub { tools: SENSOR_TOOLS, value: b"22.5" };
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S);

        let mut out = [0u8; 512];
        let n = rt.handle(b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}", &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("temp_read"), "{s}");
        assert!(s.contains("inputSchema"), "{s}");
    }

    #[test]
    fn test_tool_call() {
        static S: Stub = Stub { tools: SENSOR_TOOLS, value: b"22.5" };
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S);

        let mut out = [0u8; 512];
        let n = rt.handle(
            b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"temp_read\",\"arguments\":{}}}",
            &mut out,
        );
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("22.5"), "{s}");
    }

    #[test]
    fn test_unknown_tool() {
        static S: Stub = Stub { tools: SENSOR_TOOLS, value: b"" };
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S);

        let mut out = [0u8; 512];
        let n = rt.handle(
            b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"nope\",\"arguments\":{}}}",
            &mut out,
        );
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("unknown tool"), "{s}");
        assert!(s.contains("-32601"), "{s}");
    }

    #[test]
    fn test_no_alloc_boundary() {
        // Output buffer exactly as large as the response — should not panic.
        static S: Stub = Stub { tools: SENSOR_TOOLS, value: b"ok" };
        let mut rt: Runtime<'_, 1, 64> = Runtime::new();
        rt.register(&S);

        let mut out = [0u8; 64];
        let n = rt.handle(
            b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"temp_read\",\"arguments\":{}}}",
            &mut out,
        );
        // Should write something without panic even if truncated.
        assert!(n <= 64);
    }
}
