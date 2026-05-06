//! mcp-edge: minimal MCP runtime for resource-constrained devices.
//!
//! Zero heap allocation. `no_std`-compatible core. Runs on 64 KB RAM.
//!
//! # Tuning for your device
//!
//! `Runtime<'p, N, OUT>`:
//! - `N`   — max registered providers (default 8)
//! - `OUT` — max bytes a single tool result may produce (default 512)
//!
//! Peak stack: `OUT` bytes (tools/call path only) + ~80 bytes for handle() overhead.
//!
//! # Example
//!
//! ```rust
//! use mcp_edge::{Output, Provider, Runtime, Tool};
//! use core::fmt::Write;
//!
//! struct PingSensor;
//! impl Provider for PingSensor {
//!     fn tools(&self) -> &[Tool] {
//!         &[Tool { name: "ping", description: "Returns pong" }]
//!     }
//!     fn call(&self, _tool: &str, _args: &[u8], out: &mut Output) -> Result<(), &'static str> {
//!         write!(out, "pong").unwrap();
//!         Ok(())
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

use serde::Deserialize;

// ---------------------------------------------------------------------------
// Output — wraps a caller-provided buffer, implements fmt::Write
// ---------------------------------------------------------------------------

/// Write target for tool results. Passed to [`Provider::call`]; use `write!` on it.
pub struct Output<'a> {
    buf: &'a mut [u8],
    pub(crate) len: usize,
}

impl<'a> Output<'a> {
    pub(crate) fn new(buf: &'a mut [u8]) -> Self { Self { buf, len: 0 } }
    pub(crate) fn as_bytes(&self) -> &[u8] { &self.buf[..self.len] }
}

impl core::fmt::Write for Output<'_> {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        // Invariant: self.len <= self.buf.len(), so plain sub never underflows.
        debug_assert!(self.len <= self.buf.len());
        let n = s.len().min(self.buf.len() - self.len);
        self.buf[self.len..self.len + n].copy_from_slice(&s.as_bytes()[..n]);
        self.len += n;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Public API types
// ---------------------------------------------------------------------------

/// A tool exposed to connected agents.
#[derive(Clone, Copy, Debug)]
pub struct Tool {
    pub name: &'static str,
    pub description: &'static str,
}

/// Implement this for each capability your device exposes.
///
/// `call` receives the raw `arguments` JSON bytes (e.g. `{"pin":3}`).
/// Write the result into `out` using `write!`. Return `Err` for tool-level failures.
pub trait Provider {
    fn tools(&self) -> &[Tool];
    fn call(&self, tool: &str, args: &[u8], out: &mut Output) -> Result<(), &'static str>;
}

// ---------------------------------------------------------------------------
// Serde structs — serde-json-core parses id / method / tool name
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub(crate) struct Req<'a> {
    pub(crate) id: u64,
    pub(crate) method: &'a str,
}

#[derive(Deserialize)]
pub(crate) struct ToolCallParams<'a> {
    pub(crate) name: &'a str,
}

// ---------------------------------------------------------------------------
// Runtime
// ---------------------------------------------------------------------------

/// The MCP runtime. All storage is on the stack — no allocator required.
///
/// - `N`   — max registered providers
/// - `OUT` — max bytes a single tool result may produce
pub struct Runtime<'p, const N: usize = 8, const OUT: usize = 512> {
    providers: [Option<&'p dyn Provider>; N],
    count: usize,
}

impl<'p, const N: usize, const OUT: usize> Runtime<'p, N, OUT> {
    pub fn new() -> Self { Self { providers: [None; N], count: 0 } }

    /// Register a provider. Panics if more than `N` providers are registered.
    pub fn register(&mut self, p: &'p dyn Provider) {
        assert!(self.count < N, "exceeded provider limit");
        self.providers[self.count] = Some(p);
        self.count += 1;
    }

    /// Dispatch one newline-terminated JSON-RPC message.
    /// Writes a newline-terminated response into `out`. Returns bytes written.
    pub fn handle(&self, msg: &[u8], out: &mut [u8]) -> usize {
        let mut w = Writer::new(out);

        let Ok((req, _)) = serde_json_core::from_slice::<Req>(msg) else { return 0 };

        match req.method {
            "initialize" => write_initialize(&mut w, req.id),
            "tools/list" => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).u(req.id).s(r#","result":{"tools":["#);
                let mut first = true;
                // filter_map(|x| *x) avoids the panic codegen of unwrap().
                for p in self.providers[..self.count].iter().filter_map(|x| *x) {
                    for t in p.tools() {
                        if !first { w.s(","); }
                        first = false;
                        write_tool(&mut w, t.name, t.description);
                    }
                }
                w.s("]}}").nl();
            }
            "tools/call" => {
                self.dispatch_tool_call(msg, req.id, &mut w);
            }
            _ => rpc_err(&mut w, req.id, -32601, "method not found"),
        }

        w.pos
    }

    fn find(&self, tool: &str) -> Option<&dyn Provider> {
        self.providers[..self.count]
            .iter()
            .filter_map(|x| *x)
            .find(|p| p.tools().iter().any(|t| t.name == tool))
    }

    // Separate function so [0u8; OUT] is stack-allocated only on the tools/call path,
    // not in every handle() frame regardless of method.
    #[inline(never)]
    fn dispatch_tool_call(&self, msg: &[u8], id: u64, w: &mut Writer) {
        let params_raw = obj_get(msg, b"params").unwrap_or(b"{}");

        let Ok((tc, _)) = serde_json_core::from_slice::<ToolCallParams>(params_raw) else {
            rpc_err(w, id, -32600, "bad params");
            return;
        };

        let args = obj_get(params_raw, b"arguments").unwrap_or(b"{}");

        let mut tool_out = [0u8; OUT];
        let mut output = Output::new(&mut tool_out);

        match self.find(tc.name) {
            Some(p) => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).u(id);
                match p.call(tc.name, args, &mut output) {
                    Ok(()) => {
                        let text = core::str::from_utf8(output.as_bytes()).unwrap_or("");
                        w.s(r#","result":{"content":[{"type":"text","text":""#)
                         .esc(text).s(r#""}]}}"#);
                    }
                    Err(e) => {
                        w.s(r#","error":{"code":-1,"message":""#).esc(e).s(r#""}}"#);
                    }
                }
                w.nl();
            }
            None => rpc_err(w, id, -32601, "unknown tool"),
        }
    }
}

impl<'p, const N: usize, const OUT: usize> Default for Runtime<'p, N, OUT> {
    fn default() -> Self { Self::new() }
}

pub(crate) fn write_initialize(w: &mut Writer, id: u64) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).u(id)
     .s(r#","result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}}}}"#)
     .nl();
}

pub(crate) fn write_tool(w: &mut Writer, name: &str, desc: &str) {
    w.s(r#"{"name":""#).s(name)
     .s(r#"","description":""#).esc(desc)
     .s(r#"","inputSchema":{"type":"object"}}"#);
}

pub(crate) fn rpc_err(w: &mut Writer, id: u64, code: i32, msg: &str) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).u(id)
     .s(r#","error":{"code":"#).i(code).s(r#","message":""#)
     .esc(msg).s(r#""}}"#).nl();
}

// ---------------------------------------------------------------------------
// State-machine JSON object field extractor
//
// Finds a key in a JSON object by iterating key-value pairs, properly
// tracking string boundaries and nesting depth. No substring search.
// Returns the raw bytes of the value (including delimiters for objects/arrays).
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn obj_get<'a>(obj: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let mut p = sp(obj, 0);
    if obj.get(p)? != &b'{' { return None; }
    p += 1;

    loop {
        p = sp(obj, p);
        match obj.get(p)? {
            b'}' => return None,
            b'"' => {}
            _ => return None,
        }
        p += 1; // skip opening quote

        let k0 = p;
        eat_str(obj, &mut p)?; // advances past content + closing quote
        let k1 = p - 1;        // index of closing quote

        p = sp(obj, p);
        if obj.get(p)? != &b':' { return None; }
        p += 1;
        p = sp(obj, p);

        let v0 = p;
        skip_val(obj, &mut p)?;
        let v1 = p;

        if &obj[k0..k1] == key { return Some(&obj[v0..v1]); }

        p = sp(obj, p);
        if obj.get(p) == Some(&b',') { p += 1; }
    }
}

// Advance p past one JSON value.
#[inline]
fn skip_val(b: &[u8], p: &mut usize) -> Option<()> {
    *p = sp(b, *p);
    match b.get(*p)? {
        b'"' => { *p += 1; eat_str(b, p) }
        b'{' => skip_delimited(b, p, b'{', b'}'),
        b'[' => skip_delimited(b, p, b'[', b']'),
        b't' => { *p += 4; Some(()) }
        b'f' => { *p += 5; Some(()) }
        b'n' => { *p += 4; Some(()) }
        b'-' | b'0'..=b'9' => {
            while *p < b.len() && matches!(b[*p], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                *p += 1;
            }
            Some(())
        }
        _ => None,
    }
}

// Advance p past the body of a string; p must point just after the opening quote.
// Leaves p pointing just after the closing quote.
#[inline]
pub(crate) fn eat_str(b: &[u8], p: &mut usize) -> Option<()> {
    while *p < b.len() {
        match b[*p] {
            b'\\' => *p += 2,
            b'"'  => { *p += 1; return Some(()); }
            _     => *p += 1,
        }
    }
    None
}

// Advance p past a matched open/close pair, handling strings and nesting.
// p must point at `open` on entry.
#[inline]
pub(crate) fn skip_delimited(b: &[u8], p: &mut usize, open: u8, close: u8) -> Option<()> {
    debug_assert_eq!(b[*p], open);
    *p += 1;
    let mut depth = 1usize;
    while *p < b.len() {
        match b[*p] {
            b'"' => { *p += 1; eat_str(b, p)?; }
            c if c == open  => { depth += 1; *p += 1; }
            c if c == close => { depth -= 1; *p += 1; if depth == 0 { return Some(()); } }
            _ => *p += 1,
        }
    }
    None
}

// Skip whitespace, returning the new position.
#[inline]
pub(crate) fn sp(b: &[u8], mut p: usize) -> usize {
    while p < b.len() && matches!(b[p], b' ' | b'\t' | b'\n' | b'\r') { p += 1; }
    p
}

// ---------------------------------------------------------------------------
// Stack-only JSON writer
// ---------------------------------------------------------------------------

pub(crate) struct Writer<'a> {
    buf: &'a mut [u8],
    pub(crate) pos: usize,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(buf: &'a mut [u8]) -> Self { Self { buf, pos: 0 } }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> &mut Self {
        // Invariant: self.pos <= self.buf.len(), so plain sub never underflows.
        debug_assert!(self.pos <= self.buf.len());
        let n = bytes.len().min(self.buf.len() - self.pos);
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
        self
    }

    pub(crate) fn s(&mut self, s: &str) -> &mut Self { self.push(s.as_bytes()) }
    pub(crate) fn nl(&mut self) -> &mut Self {
        if self.pos < self.buf.len() { self.buf[self.pos] = b'\n'; self.pos += 1; }
        self
    }

    pub(crate) fn u(&mut self, n: u64) -> &mut Self {
        let mut tmp = [0u8; 20];
        self.push(fmt_u64(n, &mut tmp))
    }

    pub(crate) fn i(&mut self, n: i32) -> &mut Self {
        if n < 0 { self.push(b"-").u(n.unsigned_abs() as u64) } else { self.u(n as u64) }
    }

    pub(crate) fn esc(&mut self, s: &str) -> &mut Self {
        for b in s.bytes() {
            // Map special chars to their escape suffix; plain chars go direct.
            let second: u8 = match b {
                b'"'  => b'"',
                b'\\' => b'\\',
                b'\n' => b'n',
                b'\r' => b'r',
                b'\t' => b't',
                _ => {
                    if self.pos < self.buf.len() {
                        self.buf[self.pos] = b;
                        self.pos += 1;
                    }
                    continue;
                }
            };
            // Write backslash + escape char directly — avoids copy_from_slice overhead.
            if self.pos + 1 < self.buf.len() {
                self.buf[self.pos]     = b'\\';
                self.buf[self.pos + 1] = second;
                self.pos += 2;
            } else if self.pos < self.buf.len() {
                self.buf[self.pos] = b'\\';
                self.pos += 1;
            }
        }
        self
    }
}

fn fmt_u64(n: u64, buf: &mut [u8; 20]) -> &[u8] {
    if n == 0 { buf[19] = b'0'; return &buf[19..]; }
    let mut pos = 20usize;
    let mut v = n;
    while v > 0 { pos -= 1; buf[pos] = b'0' + (v % 10) as u8; v /= 10; }
    &buf[pos..]
}

// ---------------------------------------------------------------------------
// Unix socket transport — std-only
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
pub mod transport {
    use std::io::{Read, Write};
    use std::os::unix::net::UnixListener;

    /// Unix socket transport. Serves one client at a time.
    /// I/O buffers are stack-allocated — no heap in the connection loop.
    pub struct UnixTransport {
        path: &'static str,
    }

    impl UnixTransport {
        pub fn new(path: &'static str) -> Self {
            let _ = std::fs::remove_file(path);
            Self { path }
        }

        /// Accept connections. `handler` is called for each message.
        /// Works with both `Runtime` and `Gateway`:
        /// ```ignore
        /// transport.serve(|msg, out| rt.handle(msg, out));
        /// transport.serve(|msg, out| gw.handle(msg, out));
        /// ```
        pub fn serve(&self, handler: impl Fn(&[u8], &mut [u8]) -> usize) {
            let listener = UnixListener::bind(self.path).expect("bind failed");
            eprintln!("listening on {}", self.path);
            for mut conn in listener.incoming().flatten() {
                let mut rx = [0u8; 1024];
                let mut tx = [0u8; 1024];
                let mut filled = 0usize;    // valid bytes in rx
                let mut msg_start = 0usize; // start of next unprocessed message
                'conn: loop {
                    // Bulk read — one syscall for potentially many bytes.
                    let n = match conn.read(&mut rx[filled..]) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    filled += n;
                    // Process every complete (newline-terminated) message in the buffer.
                    loop {
                        match rx[msg_start..filled].iter().position(|&b| b == b'\n') {
                            None => break,
                            Some(rel) => {
                                let msg_end = msg_start + rel;
                                if msg_end > msg_start {
                                    let n = handler(&rx[msg_start..msg_end], &mut tx);
                                    if conn.write_all(&tx[..n]).is_err() { break 'conn; }
                                }
                                msg_start = msg_end + 1;
                            }
                        }
                    }
                    // Compact: slide unconsumed bytes to the front.
                    if msg_start > 0 {
                        rx.copy_within(msg_start..filled, 0);
                        filled -= msg_start;
                        msg_start = 0;
                    }
                    // Drop a message that overflows the buffer.
                    if filled == rx.len() { filled = 0; }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Gateway — feature-gated
// ---------------------------------------------------------------------------

#[cfg(feature = "gateway")]
pub mod gateway;
#[cfg(feature = "gateway")]
pub use gateway::Gateway;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use core::fmt::Write;

    struct Stub;
    impl Provider for Stub {
        fn tools(&self) -> &[Tool] {
            static T: &[Tool] = &[Tool { name: "temp_read", description: "Read temperature" }];
            T
        }
        fn call(&self, _: &str, _: &[u8], out: &mut Output) -> Result<(), &'static str> {
            write!(out, "22.5").unwrap();
            Ok(())
        }
    }

    #[test]
    fn test_initialize() {
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S);
        let mut out = [0u8; 512];
        let n = rt.handle(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}", &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("protocolVersion"), "{s}");
    }

    #[test]
    fn test_tools_list() {
        static S: Stub = Stub;
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
        static S: Stub = Stub;
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
    fn test_tool_error() {
        struct Fail;
        impl Provider for Fail {
            fn tools(&self) -> &[Tool] {
                static T: &[Tool] = &[Tool { name: "boom", description: "always fails" }];
                T
            }
            fn call(&self, _: &str, _: &[u8], _out: &mut Output) -> Result<(), &'static str> {
                Err("sensor offline")
            }
        }
        static F: Fail = Fail;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&F);
        let mut out = [0u8; 512];
        let n = rt.handle(
            b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"boom\",\"arguments\":{}}}",
            &mut out,
        );
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("sensor offline"), "{s}");
        assert!(s.contains("error"), "{s}");
    }

    #[test]
    fn test_unknown_tool() {
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S);
        let mut out = [0u8; 512];
        let n = rt.handle(
            b"{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"tools/call\",\"params\":{\"name\":\"nope\",\"arguments\":{}}}",
            &mut out,
        );
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("unknown tool"), "{s}");
        assert!(s.contains("-32601"), "{s}");
    }

    #[test]
    fn test_method_in_string_value_not_confused() {
        // The old memmem approach could match "method": inside a string value.
        // A proper parser must find the actual method key, not a decoy.
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S);
        let mut out = [0u8; 512];
        // "decoy" key comes first and its value contains "method":"evil"
        let msg = br#"{"jsonrpc":"2.0","decoy":"method:\"initialize\"","id":1,"method":"tools/list"}"#;
        let n = rt.handle(msg, &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        // Should dispatch tools/list, not initialize
        assert!(s.contains("tools"), "{s}");
        assert!(!s.contains("protocolVersion"), "{s}");
    }

    #[test]
    fn test_obj_get() {
        let json = br#"{"outer":{"inner":"value"},"key":"data"}"#;
        let val = obj_get(json, b"outer").unwrap();
        assert_eq!(val, br#"{"inner":"value"}"#);
        let inner = obj_get(val, b"inner").unwrap();
        assert_eq!(inner, br#""value""#);
        let data = obj_get(json, b"key").unwrap();
        assert_eq!(data, br#""data""#);
    }

    #[test]
    fn test_obj_get_not_fooled_by_string_content() {
        // Value of "trap" contains the bytes `"target":` — must not match target there.
        let json = br#"{"trap":"\"target\":fake","target":"real"}"#;
        let val = obj_get(json, b"target").unwrap();
        assert_eq!(val, br#""real""#);
    }
}
