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
//! rt.register(&sensor).unwrap();
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
        // Fail loudly on overflow rather than silently truncating: a typical
        // `write!(out, "...").unwrap()` will panic at the offending site,
        // surfacing an undersized `OUT` const generic instead of shipping
        // a truncated tool result wrapped in invalid JSON.
        if s.len() > self.buf.len() - self.len {
            return Err(core::fmt::Error);
        }
        self.buf[self.len..self.len + s.len()].copy_from_slice(s.as_bytes());
        self.len += s.len();
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

// JSON-RPC `id` is intentionally absent from this struct: the spec permits
// `string | number | null`, so we extract it as raw bytes via `obj_get` and
// echo it verbatim into the response. That keeps the runtime type-agnostic
// without dragging in a heap-allocating Value type.
#[derive(Deserialize)]
pub(crate) struct Req<'a> {
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

    /// Register a provider. Returns `Err` if more than `N` providers have been
    /// registered, matching `Gateway::add_leaf`'s error-returning shape so
    /// callers can choose how to react (typically `.unwrap()` at startup).
    pub fn register(&mut self, p: &'p dyn Provider) -> Result<(), &'static str> {
        if self.count >= N { return Err("provider limit (N) reached"); }
        self.providers[self.count] = Some(p);
        self.count += 1;
        Ok(())
    }

    /// Dispatch one newline-terminated JSON-RPC message.
    /// Writes a newline-terminated response into `out`. Returns bytes written.
    pub fn handle(&self, msg: &[u8], out: &mut [u8]) -> usize {
        let mut w = Writer::new(out);

        let Ok((req, _)) = serde_json_core::from_slice::<Req>(msg) else {
            // JSON-RPC 2.0: parse error → respond with id:null and code -32700,
            // so the client gets a clear error instead of an idle connection.
            w.s(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}"#).nl();
            if w.truncated { return 0; }
            return w.pos;
        };

        // Echo the id raw — JSON-RPC permits string|number|null, and we don't
        // need to interpret it.
        let id = obj_get(msg, b"id").unwrap_or(b"null");

        match req.method {
            "initialize" => write_initialize(&mut w, id),
            "tools/list" => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).push(id).s(r#","result":{"tools":["#);
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
                self.dispatch_tool_call(msg, id, &mut w);
            }
            _ => rpc_err(&mut w, id, -32601, "method not found"),
        }

        // Drop a truncated response rather than ship malformed JSON: the
        // client will see a closed/idle reply, which is louder and clearer
        // than mysterious parse errors on a chopped-off message.
        if w.truncated { return 0; }
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
    fn dispatch_tool_call(&self, msg: &[u8], id: &[u8], w: &mut Writer) {
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
                w.s(r#"{"jsonrpc":"2.0","id":"#).push(id);
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

pub(crate) fn write_initialize(w: &mut Writer, id: &[u8]) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).push(id)
     .s(r#","result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}}}}"#)
     .nl();
}

pub(crate) fn write_tool(w: &mut Writer, name: &str, desc: &str) {
    w.s(r#"{"name":""#).s(name)
     .s(r#"","description":""#).esc(desc)
     .s(r#"","inputSchema":{"type":"object"}}"#);
}

/// Exact byte count `write_tool` will produce for `(name, desc)`. Used by the
/// gateway to pre-flight whether a tool entry will fit in its tools-list cache
/// (so we can return an error at startup instead of silently truncating).
#[cfg(feature = "gateway")]
pub(crate) fn write_tool_size(name: &str, desc: &str) -> usize {
    // Must match write_tool's literals exactly:
    //   {"name":"NAME","description":"ESC(DESC)","inputSchema":{"type":"object"}}
    //   ^^^^^^^^^      ^^^^^^^^^^^^^^^^^         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
    //   9              17                        33
    9 + name.len() + 17 + esc_len(desc) + 33
}

#[cfg(feature = "gateway")]
fn esc_len(s: &str) -> usize {
    // Mirror Writer::esc exactly: short escapes cost 2 bytes, long-form
    // \u00XX escapes cost 6, plain bytes cost 1.
    s.bytes()
        .map(|b| match b {
            b'"' | b'\\' | b'\n' | b'\r' | b'\t' | 0x08 | 0x0C => 2,
            0..=0x1F => 6,
            _ => 1,
        })
        .sum()
}

pub(crate) fn rpc_err(w: &mut Writer, id: &[u8], code: i32, msg: &str) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).push(id)
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
    /// Set when any write would overflow the buffer. Once truncated, all
    /// subsequent writes no-op so downstream content can't get reordered or
    /// corrupted. Callers (`Runtime::handle`, `Gateway::handle`) check this
    /// and drop the response rather than send malformed JSON to the client.
    pub(crate) truncated: bool,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(buf: &'a mut [u8]) -> Self { Self { buf, pos: 0, truncated: false } }

    pub(crate) fn push(&mut self, bytes: &[u8]) -> &mut Self {
        if self.truncated { return self; }
        debug_assert!(self.pos <= self.buf.len());
        // All-or-nothing: a partial write would emit invalid JSON anyway, and
        // letting later writes succeed after a partial one would re-order the
        // output relative to the source.
        if bytes.len() > self.buf.len() - self.pos {
            self.truncated = true;
            return self;
        }
        self.buf[self.pos..self.pos + bytes.len()].copy_from_slice(bytes);
        self.pos += bytes.len();
        self
    }

    pub(crate) fn s(&mut self, s: &str) -> &mut Self { self.push(s.as_bytes()) }
    pub(crate) fn nl(&mut self) -> &mut Self {
        if self.truncated { return self; }
        if self.pos < self.buf.len() {
            self.buf[self.pos] = b'\n';
            self.pos += 1;
        } else {
            self.truncated = true;
        }
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
        if self.truncated { return self; }
        // Bulk-copy runs of plain bytes between escape chars. `iter().position`
        // with a constant set of needles is loop-friendly (LLVM often
        // auto-vectorizes on x86/NEON), and `push` lowers to one memcpy per
        // run — far cheaper than the original byte-at-a-time write loop for
        // typical descriptions and tool-result text.
        let bytes = s.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // JSON requires escaping `"`, `\`, and every byte in 0x00..=0x1F.
            // Short forms exist for \n \r \t \b \f; everything else in that
            // range needs the six-byte \u00XX form.
            let rel = bytes[i..]
                .iter()
                .position(|&b| matches!(b, b'"' | b'\\' | 0..=0x1F));
            let end = rel.map(|r| i + r).unwrap_or(bytes.len());
            if end > i {
                self.push(&bytes[i..end]);
                if self.truncated { return self; }
            }
            let Some(r) = rel else { break };
            let byte = bytes[i + r];
            let short = match byte {
                b'"'  => Some(b'"'),
                b'\\' => Some(b'\\'),
                b'\n' => Some(b'n'),
                b'\r' => Some(b'r'),
                b'\t' => Some(b't'),
                0x08  => Some(b'b'),
                0x0C  => Some(b'f'),
                _ => None,
            };
            // Two-byte and six-byte escapes are both all-or-nothing: an orphan
            // `\` would be interpreted as starting an escape against whatever
            // byte follows (often the closing `"`), corrupting the JSON.
            match short {
                Some(c) => {
                    if self.pos + 2 <= self.buf.len() {
                        self.buf[self.pos]     = b'\\';
                        self.buf[self.pos + 1] = c;
                        self.pos += 2;
                    } else {
                        self.truncated = true;
                        return self;
                    }
                }
                None => {
                    if self.pos + 6 <= self.buf.len() {
                        self.buf[self.pos]     = b'\\';
                        self.buf[self.pos + 1] = b'u';
                        self.buf[self.pos + 2] = b'0';
                        self.buf[self.pos + 3] = b'0';
                        self.buf[self.pos + 4] = hex_nibble(byte >> 4);
                        self.buf[self.pos + 5] = hex_nibble(byte & 0x0F);
                        self.pos += 6;
                    } else {
                        self.truncated = true;
                        return self;
                    }
                }
            }
            i = end + 1;
        }
        self
    }
}

const fn hex_nibble(n: u8) -> u8 {
    if n < 10 { b'0' + n } else { b'a' + (n - 10) }
}

fn fmt_u64(n: u64, buf: &mut [u8; 20]) -> &[u8] {
    if n == 0 { buf[19] = b'0'; return &buf[19..]; }
    let mut pos = 20usize;
    let mut v = n;
    while v > 0 { pos -= 1; buf[pos] = b'0' + (v % 10) as u8; v /= 10; }
    &buf[pos..]
}

// ---------------------------------------------------------------------------
// Transports — std-only
// ---------------------------------------------------------------------------

#[cfg(feature = "std")]
pub mod transport {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::os::unix::net::UnixListener;

    /// Drive one connection's read/write loop. Generic over any `Read + Write`
    /// stream so it serves Unix, TCP, TLS-wrapped, etc., transports identically.
    ///
    /// Static dispatch on `H: Fn` keeps the per-message call into the handler a
    /// direct call, not an indirect one — the loop is the hot path on a busy
    /// gateway. `#[inline(never)]` keeps the 2 KB rx/tx buffers in this
    /// function's frame instead of inlining them into every transport's
    /// `serve()` accept loop.
    #[inline(never)]
    pub fn run_connection<C, H>(mut conn: C, handler: &H)
    where
        C: Read + Write,
        H: Fn(&[u8], &mut [u8]) -> usize + ?Sized,
    {
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
                            if n > 0 {
                                if conn.write_all(&tx[..n]).is_err() { break 'conn; }
                                // Flush is a no-op on socket streams but
                                // required for buffered streams like stdout:
                                // when this process is spawned with a pipe
                                // (the typical subprocess-MCP-server setup),
                                // stdout is fully-buffered by default and
                                // unflushed responses sit invisible.
                                if conn.flush().is_err() { break 'conn; }
                            }
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
            // Oversized message (> rx.len() with no newline): close the
            // connection. Resetting `filled` would re-interpret the tail of
            // the dropped message as a fresh request and parse garbage —
            // closing makes the failure visible to the client.
            if filled == rx.len() { break 'conn; }
        }
    }

    /// Unix socket transport. Serves one client at a time.
    /// I/O buffers are stack-allocated — no heap in the connection loop.
    ///
    /// The lifetime parameter lets callers pass any `&str` (env-var-derived
    /// `String`s, args, etc.) without leaking to `'static`.
    pub struct UnixTransport<'a> {
        path: &'a str,
    }

    impl<'a> UnixTransport<'a> {
        pub fn new(path: &'a str) -> Self {
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
            for conn in listener.incoming().flatten() {
                run_connection(conn, &handler);
            }
        }
    }

    /// TCP socket transport. Same handler interface as `UnixTransport`.
    /// Use this for cross-machine setups (LAN, automotive Ethernet, etc.).
    /// For untrusted networks, layer TLS on top — see the `tls` feature
    /// (planned) or wrap your own `rustls` acceptor and call `run_connection`.
    pub struct TcpTransport<'a> {
        addr: &'a str,
    }

    impl<'a> TcpTransport<'a> {
        pub fn new(addr: &'a str) -> Self { Self { addr } }

        pub fn serve(&self, handler: impl Fn(&[u8], &mut [u8]) -> usize) {
            let listener = TcpListener::bind(self.addr).expect("bind failed");
            eprintln!("listening on {}", self.addr);
            for conn in listener.incoming().flatten() {
                run_connection(conn, &handler);
            }
        }
    }

    /// Stdio transport. Reads JSON-RPC requests from stdin, writes responses
    /// to stdout — the canonical shape for MCP servers that a host spawns
    /// as a subprocess (the most common local-server pattern).
    ///
    /// ```ignore
    /// use mcp_edge::transport::StdioTransport;
    /// StdioTransport::serve(|m, o| rt.handle(m, o));
    /// ```
    ///
    /// See `examples/README.md` for host-side registration steps.
    pub struct StdioTransport;

    impl StdioTransport {
        pub fn serve(handler: impl Fn(&[u8], &mut [u8]) -> usize) {
            // Wrap stdin+stdout into a single Read+Write so the same
            // `run_connection` framing loop drives stdio identically to
            // sockets. Locks are acquired once here rather than per-syscall:
            // `Stdin::read` / `Stdout::write` reacquire the global lock on
            // every call, which is real overhead on the typical
            // subprocess-MCP path. Buffered stdout is the reason
            // `run_connection` flushes after each response.
            struct StdioStream<'a> {
                stdin: std::io::StdinLock<'a>,
                stdout: std::io::StdoutLock<'a>,
            }
            impl Read for StdioStream<'_> {
                fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                    self.stdin.read(buf)
                }
            }
            impl Write for StdioStream<'_> {
                fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                    self.stdout.write(buf)
                }
                fn flush(&mut self) -> std::io::Result<()> {
                    self.stdout.flush()
                }
            }
            let stdin = std::io::stdin();
            let stdout = std::io::stdout();
            let stream = StdioStream {
                stdin: stdin.lock(),
                stdout: stdout.lock(),
            };
            run_connection(stream, &handler);
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
        rt.register(&S).unwrap();
        let mut out = [0u8; 512];
        let n = rt.handle(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\"}", &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains("protocolVersion"), "{s}");
    }

    #[test]
    fn test_tools_list() {
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
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
        rt.register(&S).unwrap();
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
        rt.register(&F).unwrap();
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
        rt.register(&S).unwrap();
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
        rt.register(&S).unwrap();
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
    fn test_string_id_echoed_verbatim() {
        // JSON-RPC 2.0 permits string ids; mcp-edge must echo the raw bytes.
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
        let mut out = [0u8; 512];
        let n = rt.handle(
            br#"{"jsonrpc":"2.0","id":"abc-42","method":"tools/list"}"#,
            &mut out,
        );
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains(r#""id":"abc-42""#), "{s}");
    }

    #[test]
    fn test_null_id_echoed() {
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
        let mut out = [0u8; 512];
        let n = rt.handle(
            br#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#,
            &mut out,
        );
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains(r#""id":null"#), "{s}");
    }

    #[test]
    fn test_parse_error_response() {
        // Malformed JSON must produce a JSON-RPC parse-error reply with
        // id:null and code -32700, not a silent zero-byte drop.
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
        let mut out = [0u8; 256];
        let n = rt.handle(b"this is not json", &mut out);
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains(r#""id":null"#), "{s}");
        assert!(s.contains("-32700"), "{s}");
        assert!(s.contains("parse error"), "{s}");
    }

    #[test]
    fn test_register_returns_err_when_full() {
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        assert!(rt.register(&S).is_ok());
        assert!(rt.register(&S).is_err(), "second register past N must Err, not panic");
    }

    #[test]
    fn test_response_truncation_drops_response() {
        // A 16-byte out buffer can't hold any well-formed JSON-RPC reply,
        // so handle() must return 0 (drop) rather than emit garbage.
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
        let mut out = [0u8; 16];
        let n = rt.handle(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}", &mut out);
        assert_eq!(n, 0, "truncated response must be dropped, not partially emitted");
    }

    #[test]
    fn test_output_overflow_returns_err() {
        // Writing more than OUT bytes from a tool must produce fmt::Error so
        // user code (`write!(...).unwrap()`) panics loudly instead of silently
        // truncating.
        let mut buf = [0u8; 4];
        let mut out = Output::new(&mut buf);
        let r = core::fmt::Write::write_str(&mut out, "too long");
        assert!(r.is_err(), "Output::write_str must Err on overflow, got Ok");
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
    fn test_esc() {
        // Exercises Writer::esc directly: plain runs, leading/trailing escapes,
        // adjacent escapes, and an empty string.
        fn run(input: &str) -> String {
            let mut buf = [0u8; 256];
            let pos = {
                let mut w = Writer::new(&mut buf);
                w.esc(input);
                w.pos
            };
            core::str::from_utf8(&buf[..pos]).unwrap().to_string()
        }
        assert_eq!(run(""), "");
        assert_eq!(run("plain text"), "plain text");
        assert_eq!(run("a\nb"), "a\\nb");
        assert_eq!(run("\"quoted\""), "\\\"quoted\\\"");
        assert_eq!(run("\n\t"), "\\n\\t");
        assert_eq!(run("a\\b"), "a\\\\b");
        assert_eq!(run("end\n"), "end\\n");
        assert_eq!(run("\rstart"), "\\rstart");
        // Short forms for \b and \f.
        assert_eq!(run("\x08"), "\\b");
        assert_eq!(run("\x0c"), "\\f");
        // Long form \u00XX for control bytes without a short form.
        assert_eq!(run("\x01"), "\\u0001");
        assert_eq!(run("\x1f"), "\\u001f");
        // Mixed: plain text + control byte + plain text.
        assert_eq!(run("ok\x07bell"), "ok\\u0007bell");
    }

    #[test]
    fn test_obj_get_not_fooled_by_string_content() {
        // Value of "trap" contains the bytes `"target":` — must not match target there.
        let json = br#"{"trap":"\"target\":fake","target":"real"}"#;
        let val = obj_get(json, b"target").unwrap();
        assert_eq!(val, br#""real""#);
    }
}
