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
// JSON helpers — id, method, and tool name are all extracted via obj_get
// directly. The runtime never deserializes JSON into typed structs, which
// is what lets us drop serde + serde-json-core entirely.
//
// JSON-RPC `id` is extracted as raw bytes (the spec permits string|number|null)
// and echoed verbatim in the response. Method and tool-name keys are matched
// against quoted-byte literals (e.g. `b"\"initialize\""`), avoiding a
// per-message string-decode pass. Tool names with non-trivial JSON escapes
// would not match — fine in practice, since names are short identifiers.
// ---------------------------------------------------------------------------

/// Strip the outer `"` from a JSON-string value (as returned by `obj_get`).
/// Returns the inner bytes verbatim — does not decode `\u00XX` / `\n` / etc.
/// Adequate for short identifiers (method names, tool names); unsuitable
/// for arbitrary text where escape decoding matters.
#[inline]
pub(crate) fn unquote(b: &[u8]) -> Option<&[u8]> {
    if b.len() >= 2 && b[0] == b'"' && b[b.len() - 1] == b'"' {
        Some(&b[1..b.len() - 1])
    } else {
        None
    }
}

/// Walk a JSON object once, calling `on_pair(key_bytes, value_bytes)` for
/// every member. This is the multi-key complement to `obj_get`: when a
/// caller needs three or four fields from the same object, one walk through
/// the bytes is dramatically cheaper than calling `obj_get` four times
/// (which restarts the linear scan each call).
///
/// Both `key_bytes` and `value_bytes` borrow from `obj`; `value_bytes`
/// includes the surrounding `"` for string values, the surrounding `{}`
/// for objects, etc. — same shape as `obj_get`'s return.
///
/// Stops silently on malformed input (e.g. mid-value EOF). The closure is
/// invoked for every well-formed pair seen up to that point — callers must
/// tolerate partial data rather than rely on "all-or-nothing" parsing.
#[inline]
pub(crate) fn walk_obj_for<'a, F: FnMut(&'a [u8], &'a [u8])>(obj: &'a [u8], mut on_pair: F) {
    let mut p = sp(obj, 0);
    if obj.get(p) != Some(&b'{') { return; }
    p += 1;
    loop {
        p = sp(obj, p);
        match obj.get(p) {
            Some(b'}') | None => return,
            Some(b'"') => {}
            _ => return,
        }
        p += 1;
        let k0 = p;
        if eat_str(obj, &mut p).is_none() { return; }
        let k1 = p - 1;
        p = sp(obj, p);
        if obj.get(p) != Some(&b':') { return; }
        p += 1;
        p = sp(obj, p);
        let v0 = p;
        if skip_val(obj, &mut p).is_none() { return; }
        let v1 = p;
        on_pair(&obj[k0..k1], &obj[v0..v1]);
        p = sp(obj, p);
        if obj.get(p) == Some(&b',') { p += 1; }
    }
}

/// Classification of an inbound JSON-RPC frame, used by both `Runtime::handle`
/// and `Gateway::handle` to enforce the spec's three response cases:
///
/// - `ParseError`: input isn't a JSON object → respond with id:null + -32700.
/// - `Notification`: valid JSON object with no `id` field → MUST NOT respond.
/// - `Request`: valid JSON object with an `id` (including explicit `null`).
///   `method` may be empty if the field is missing — dispatchers fall through
///   to `-32601 method not found` in that case. `params` is `None` when
///   absent, which the dispatchers treat as `{}`.
///
/// `classify` does a single walk through the message bytes. Callers used to
/// chain three `obj_get` calls (id, method, params) — `walk_obj_for` lets us
/// capture all three in one linear scan, which is meaningful CPU on a small
/// MCU where every JSON pass is paid for in cycles.
pub(crate) enum Msg<'a> {
    ParseError,
    Notification,
    Request {
        id: &'a [u8],
        method: &'a [u8],
        params: Option<&'a [u8]>,
    },
}

#[inline]
pub(crate) fn classify(msg: &[u8]) -> Msg<'_> {
    // Reject anything that isn't a JSON object outright. Per JSON-RPC 2.0,
    // a parse error is the only case where we respond despite not knowing
    // the request id (id:null).
    let p = sp(msg, 0);
    if msg.get(p) != Some(&b'{') {
        return Msg::ParseError;
    }
    let mut id: Option<&[u8]> = None;
    let mut method: Option<&[u8]> = None;
    let mut params: Option<&[u8]> = None;
    walk_obj_for(msg, |k, v| match k {
        b"id" => id = Some(v),
        b"method" => method = Some(v),
        b"params" => params = Some(v),
        _ => {}
    });
    // Absence of `id` is the canonical "this is a notification" signal —
    // including for malformed/incomplete frames that lacked closing braces.
    let Some(id) = id else { return Msg::Notification; };
    Msg::Request { id, method: method.unwrap_or(b""), params }
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
}

impl<'p, const N: usize, const OUT: usize> Runtime<'p, N, OUT> {
    pub fn new() -> Self { Self { providers: [None; N] } }

    /// Register a provider. Returns `Err` if more than `N` providers have been
    /// registered, matching `Gateway::add_leaf`'s error-returning shape so
    /// callers can choose how to react (typically `.unwrap()` at startup).
    pub fn register(&mut self, p: &'p dyn Provider) -> Result<(), &'static str> {
        let slot = self.providers.iter_mut().find(|s| s.is_none())
            .ok_or("provider limit (N) reached")?;
        *slot = Some(p);
        Ok(())
    }

    /// Dispatch one newline-terminated JSON-RPC message.
    /// Writes a newline-terminated response into `out`. Returns bytes written.
    ///
    /// Returns 0 for notifications (per JSON-RPC 2.0, a request with no `id`
    /// field is a notification and the server MUST NOT respond) and for
    /// truncated responses (callers should not forward malformed JSON).
    pub fn handle(&self, msg: &[u8], out: &mut [u8]) -> usize {
        match classify(msg) {
            Msg::ParseError => write_parse_error(out),
            // A notification (no `id`) is processed for side effects but
            // produces no response. mcp-edge has no side-effecting
            // notifications today, so we simply drop them.
            Msg::Notification => 0,
            Msg::Request { id, method, params } => {
                let mut w = Writer::new(out);
                self.dispatch(method, id, params, &mut w);
                if w.truncated { 0 } else { w.pos }
            }
        }
    }

    fn dispatch(&self, method: &[u8], id: &[u8], params: Option<&[u8]>, w: &mut Writer) {
        // Match against quoted bytes — saves a string-decode pass per message.
        match method {
            br#""initialize""# => write_initialize(w, id),
            br#""tools/list""# => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).push(id).s(r#","result":{"tools":["#);
                let mut first = true;
                for p in self.providers.iter().filter_map(|x| *x) {
                    for t in p.tools() {
                        if !first { w.s(","); }
                        first = false;
                        write_tool(w, t.name, t.description);
                    }
                }
                w.s("]}}").nl();
            }
            br#""tools/call""# => {
                self.dispatch_tools_call(params, id, w);
            }
            _ => rpc_err(w, id, -32601, "method not found"),
        }
    }

    fn find(&self, tool: &str) -> Option<&dyn Provider> {
        self.providers
            .iter()
            .filter_map(|x| *x)
            .find(|p| p.tools().iter().any(|t| t.name == tool))
    }

    // Separate function so [0u8; OUT] is stack-allocated only on the tools/call path,
    // not in every handle() frame regardless of method.
    #[inline(never)]
    fn dispatch_tools_call(&self, params: Option<&[u8]>, id: &[u8], w: &mut Writer) {
        // Walk params once: pull both `name` and `arguments` in a single
        // linear scan instead of two `obj_get` calls. For a typical 80-byte
        // params object this halves the bytes scanned per tools/call.
        let mut name_q: Option<&[u8]> = None;
        let mut args: Option<&[u8]> = None;
        if let Some(p) = params {
            walk_obj_for(p, |k, v| match k {
                b"name" => name_q = Some(v),
                b"arguments" => args = Some(v),
                _ => {}
            });
        }
        let Some(name_q) = name_q else {
            rpc_err(w, id, -32600, "bad params");
            return;
        };
        let Some(name) = unquote(name_q).and_then(|b| core::str::from_utf8(b).ok()) else {
            rpc_err(w, id, -32600, "bad params");
            return;
        };
        let args = args.unwrap_or(b"{}");

        let mut tool_out = [0u8; OUT];
        let mut output = Output::new(&mut tool_out);

        match self.find(name) {
            Some(p) => {
                w.s(r#"{"jsonrpc":"2.0","id":"#).push(id);
                match p.call(name, args, &mut output) {
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

/// JSON-RPC 2.0 parse-error reply (id:null, code -32700). Returns the
/// number of bytes written; 0 if `out` was too small to hold the response,
/// in which case the caller drops the message instead of shipping a
/// truncated frame.
#[inline]
pub(crate) fn write_parse_error(out: &mut [u8]) -> usize {
    let mut w = Writer::new(out);
    w.s(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"parse error"}}"#).nl();
    if w.truncated { 0 } else { w.pos }
}

/// `initialize` reply for the local `Runtime`. Declares only that tools exist
/// (`capabilities: {tools: {}}`) — Runtime cannot push `list_changed` because
/// its provider set is `'static` and registered before `serve` runs.
pub(crate) fn write_initialize(w: &mut Writer, id: &[u8]) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).push(id)
     .s(r#","result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}}}}"#)
     .nl();
}

/// `initialize` reply for `Gateway`. Declares `tools.listChanged: true` so
/// clients know the gateway will push `notifications/tools/list_changed`
/// when leaves are added or removed at runtime.
#[cfg(feature = "gateway")]
pub(crate) fn write_initialize_gateway(w: &mut Writer, id: &[u8]) {
    w.s(r#"{"jsonrpc":"2.0","id":"#).push(id)
     .s(r#","result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{"listChanged":true}}}}"#)
     .nl();
}

/// JSON-RPC notification frame the gateway broadcasts when its tool set
/// changes. Newline-terminated to match the rest of the wire protocol.
#[cfg(feature = "gateway")]
pub(crate) const TOOLS_LIST_CHANGED_NOTIFY: &[u8] =
    b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/tools/list_changed\"}\n";

pub(crate) fn write_tool(w: &mut Writer, name: &str, desc: &str) {
    // Escape the name as well as the description. Names are `'static` and
    // developer-supplied, so this is defence in depth rather than a live
    // hole, but an unescaped `"` in a name would emit invalid JSON.
    w.s(r#"{"name":""#).esc(name)
     .s(r#"","description":""#).esc(desc)
     .s(r#"","inputSchema":{"type":"object"}}"#);
}

/// Constant overhead `write_tool_raw` adds around `(name, desc_raw)` in bytes.
/// Used by the gateway to pre-flight tool-list cache writes.
//
//   {"name":"NAME","description":"DESC","inputSchema":{"type":"object"}}
//   ^^^^^^^^^      ^^^^^^^^^^^^^^^^^    ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
//   9              17                   33
#[cfg(feature = "gateway")]
pub(crate) const TOOL_JSON_OVERHEAD: usize = 9 + 17 + 33;

/// Like `write_tool`, but treats `name` and `desc_raw` as already-escaped
/// JSON-string content (the bytes between the surrounding `"`). The gateway
/// uses this when echoing tool entries it received from a leaf — they're
/// already valid JSON, so a decode-then-re-encode pass would just waste
/// cycles and risk re-escaping a `\"` into `\\\"`.
#[cfg(feature = "gateway")]
pub(crate) fn write_tool_raw(w: &mut Writer, name: &[u8], desc_raw: &[u8]) {
    w.s(r#"{"name":""#).push(name)
     .s(r#"","description":""#).push(desc_raw)
     .s(r#"","inputSchema":{"type":"object"}}"#);
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

// Only the gateway extracts arbitrary keys; the runtime's hot paths use
// `walk_obj_for`. Gated so `--no-default-features` builds stay warning-free.
#[cfg(any(feature = "gateway", test))]
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
        // Literals must be matched in full, not skipped by length. A blind
        // `*p += 4` walks the cursor past the end of a buffer that ends
        // mid-literal (`{"id":t`), and the caller then slices `obj[v0..v1]`
        // out of range. Every inbound message reaches here through
        // `classify`, so that was a remote panic, and `panic = "abort"`
        // turns a panic into a dead process. Matching the bytes also stops
        // garbage like `txx` being accepted as a value and echoed back into
        // a response as invalid JSON.
        b't' => lit(b, p, b"true"),
        b'f' => lit(b, p, b"false"),
        b'n' => lit(b, p, b"null"),
        b'-' | b'0'..=b'9' => {
            while *p < b.len() && matches!(b[*p], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                *p += 1;
            }
            Some(())
        }
        _ => None,
    }
}

// Match one JSON literal exactly, advancing past it. `*p < b.len()` on entry.
#[inline]
fn lit(b: &[u8], p: &mut usize, word: &[u8]) -> Option<()> {
    if b.len() - *p >= word.len() && &b[*p..*p + word.len()] == word {
        *p += word.len();
        Some(())
    } else {
        None
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
    /// `RX` / `TX` are the rx + tx buffer sizes in bytes. Defaults of 1024
    /// each suit a typical MCP message; tune them down on RAM-constrained
    /// devices (an 8 KiB-RAM part can run with `RX=256, TX=256`).
    ///
    /// Static dispatch on `H: Fn` keeps the per-message call into the handler a
    /// direct call, not an indirect one — the loop is the hot path on a busy
    /// gateway. `#[inline(never)]` keeps the rx/tx buffers in this function's
    /// frame instead of inlining them into every transport's `serve()` accept
    /// loop.
    #[inline(never)]
    pub fn run_connection<const RX: usize, const TX: usize, C, H>(mut conn: C, handler: &H)
    where
        C: Read + Write,
        H: Fn(&[u8], &mut [u8]) -> usize + ?Sized,
    {
        let mut framer: crate::Framer<RX, TX> = crate::Framer::new();
        loop {
            // Read straight into the framer's buffer: one syscall, no copy.
            let n = match conn.read(framer.rx_space()) {
                Ok(0) | Err(_) => break,
                Ok(n) => n,
            };
            let mut io_err = false;
            let framed = framer.commit(
                n,
                |msg, out| handler(msg, out),
                |resp| {
                    if conn.write_all(resp).is_err() {
                        io_err = true;
                        return;
                    }
                    // Flush is a no-op on socket streams but required for
                    // buffered ones like stdout: when this process is spawned
                    // with a pipe (the typical subprocess-MCP-server setup),
                    // stdout is fully-buffered and unflushed responses sit
                    // invisible.
                    if conn.flush().is_err() {
                        io_err = true;
                    }
                },
            );
            // An oversized frame closes the connection here rather than using
            // the framer's resync: on a stream there is a peer to disconnect,
            // and a silent recovery would hide a misconfigured RX from them.
            if io_err || framed.is_err() {
                break;
            }
        }
    }

    /// Unix socket transport. Serves one client at a time.
    /// I/O buffers are stack-allocated — no heap in the connection loop.
    ///
    /// `RX` / `TX` size the read and write buffers in bytes (defaults 1024).
    /// Drop them on RAM-tight devices: e.g. `UnixTransport::<256, 256>`.
    ///
    /// The lifetime parameter lets callers pass any `&str` (env-var-derived
    /// `String`s, args, etc.) without leaking to `'static`.
    pub struct UnixTransport<'a, const RX: usize = 1024, const TX: usize = 1024> {
        path: &'a str,
    }

    // `new` is declared on the default-sized impl so call sites like
    // `UnixTransport::new(path)` infer the const generics; users who want
    // tuned sizes write `UnixTransport::<256, 256>::new_sized(path)` against
    // the generic impl below.
    impl<'a> UnixTransport<'a, 1024, 1024> {
        pub fn new(path: &'a str) -> Self {
            Self::new_sized(path)
        }
    }

    impl<'a, const RX: usize, const TX: usize> UnixTransport<'a, RX, TX> {
        pub fn new_sized(path: &'a str) -> Self {
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
            for conn in listener.incoming().flatten() {
                run_connection::<RX, TX, _, _>(conn, &handler);
            }
        }
    }

    /// TCP socket transport. Same handler interface as `UnixTransport`.
    /// Use this for cross-machine setups (LAN, automotive Ethernet, etc.).
    /// For untrusted networks, layer TLS on top — see the `tls` feature
    /// (planned) or wrap your own `rustls` acceptor and call `run_connection`.
    pub struct TcpTransport<'a, const RX: usize = 1024, const TX: usize = 1024> {
        addr: &'a str,
    }

    impl<'a> TcpTransport<'a, 1024, 1024> {
        pub fn new(addr: &'a str) -> Self { Self::new_sized(addr) }
    }

    impl<'a, const RX: usize, const TX: usize> TcpTransport<'a, RX, TX> {
        pub fn new_sized(addr: &'a str) -> Self { Self { addr } }

        pub fn serve(&self, handler: impl Fn(&[u8], &mut [u8]) -> usize) {
            let listener = TcpListener::bind(self.addr).expect("bind failed");
            for conn in listener.incoming().flatten() {
                run_connection::<RX, TX, _, _>(conn, &handler);
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
        /// Same defaults as the socket transports (1024 + 1024). For tiny
        /// stdio servers, call `serve_sized::<RX, TX>` instead.
        pub fn serve(handler: impl Fn(&[u8], &mut [u8]) -> usize) {
            Self::serve_sized::<1024, 1024>(handler);
        }

        pub fn serve_sized<const RX: usize, const TX: usize>(handler: impl Fn(&[u8], &mut [u8]) -> usize) {
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
            run_connection::<RX, TX, _, _>(stream, &handler);
        }
    }

    /// UDP datagram transport. One datagram per request, one per reply.
    /// Each request is handled inline; there's no concept of a connection,
    /// so successive datagrams may come from different peers.
    ///
    /// Sized for short JSON-RPC messages: 1024-byte rx and tx buffers.
    /// Datagrams larger than 1024 bytes are silently truncated by the
    /// kernel — keep tool results well under that or use TCP/Unix instead.
    ///
    /// Reply datagrams are emitted exactly as the handler returns them.
    /// `Runtime::handle` already trails responses with `\n`, which the
    /// gateway's `UdpConnector` relies on for framing — see that type's
    /// docs if you're writing a custom UDP handler.
    ///
    /// ```ignore
    /// use mcp_edge::transport::UdpTransport;
    /// UdpTransport::new("0.0.0.0:9000").serve(|m, o| rt.handle(m, o));
    /// ```
    #[cfg(feature = "udp")]
    pub struct UdpTransport<'a, const RX: usize = 1024, const TX: usize = 1024> {
        addr: &'a str,
    }

    #[cfg(feature = "udp")]
    impl<'a> UdpTransport<'a, 1024, 1024> {
        pub fn new(addr: &'a str) -> Self { Self::new_sized(addr) }
    }

    #[cfg(feature = "udp")]
    impl<'a, const RX: usize, const TX: usize> UdpTransport<'a, RX, TX> {
        pub fn new_sized(addr: &'a str) -> Self { Self { addr } }

        pub fn serve(&self, handler: impl Fn(&[u8], &mut [u8]) -> usize) {
            use std::net::UdpSocket;
            let socket = UdpSocket::bind(self.addr).expect("bind failed");
            let mut rx = [0u8; RX];
            let mut tx = [0u8; TX];
            loop {
                // recv_from rather than recv — UDP is connectionless, so the
                // peer address comes per-datagram. We reply to whoever asked.
                let (n, src) = match socket.recv_from(&mut rx) {
                    Ok(v) => v,
                    Err(_) => continue, // transient errors don't kill the loop
                };
                if n == 0 { continue; }
                let m = handler(&rx[..n], &mut tx);
                if m > 0 {
                    // Best-effort: a failed send_to (peer gone, ICMP
                    // unreachable) shouldn't take down the server.
                    let _ = socket.send_to(&tx[..m], src);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Framing — no_std core, shared by every transport
// ---------------------------------------------------------------------------

pub mod frame;
pub use frame::{FrameError, Framer};

// ---------------------------------------------------------------------------
// Gateway — feature-gated
// ---------------------------------------------------------------------------

#[cfg(feature = "gateway")]
pub mod gateway;
#[cfg(feature = "gateway")]
pub use gateway::{DynamicGateway, Gateway};

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
    fn test_notification_drops_response() {
        // JSON-RPC 2.0: a request with no `id` field is a notification —
        // server MUST NOT respond. mcp-edge has no notification side effects,
        // so we drop the message silently.
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
        let mut out = [0u8; 256];
        let n = rt.handle(
            br#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            &mut out,
        );
        assert_eq!(n, 0, "notifications must produce no response");
    }

    #[test]
    fn test_explicit_null_id_is_request_not_notification() {
        // `"id": null` is a legal request id — distinct from `id` being
        // absent. The server MUST respond, echoing the null id.
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
        let mut out = [0u8; 512];
        let n = rt.handle(
            br#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#,
            &mut out,
        );
        assert!(n > 0, "explicit id:null is a request, not a notification");
        let s = core::str::from_utf8(&out[..n]).unwrap();
        assert!(s.contains(r#""id":null"#), "{s}");
        assert!(s.contains("tools"), "{s}");
    }

    #[test]
    fn test_notification_with_unknown_method_is_dropped() {
        // Unknown-method on a request returns -32601, but on a notification
        // it must still produce no response — `id` absence dominates.
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();
        let mut out = [0u8; 256];
        let n = rt.handle(br#"{"jsonrpc":"2.0","method":"foo/bar"}"#, &mut out);
        assert_eq!(n, 0, "unknown method on a notification must still be silent");
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

    #[test]
    fn fuzz_handle_never_panics_on_untrusted_input() {
        static S: Stub = Stub;
        let mut rt: Runtime<'_, 1> = Runtime::new();
        rt.register(&S).unwrap();

        // xorshift: deterministic, no dev-dependency.
        let mut st: u64 = 0x243F6A8885A308D3;
        let mut rnd = move || { st ^= st << 13; st ^= st >> 7; st ^= st << 17; st };

        // Alphabet biased toward JSON structure so we hit parser states.
        let alpha: &[u8] = b"{}[]\":,0123456789tfnaeul \\\n\t\x00\xff-+.eE";
        let seeds: &[&[u8]] = &[
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"temp_read","arguments":{}}}"#,
            br#"{"id":null,"method":"tools/list"}"#,
            br#"{"id":"x","method":"initialize"}"#,
        ];

        let mut panics = 0usize;
        let mut buf = [0u8; 512];

        // 1. Pure random bytes.
        for _ in 0..40_000 {
            let len = (rnd() % 96) as usize;
            let input: Vec<u8> = (0..len).map(|_| alpha[(rnd() % alpha.len() as u64) as usize]).collect();
            let mut out = [0u8; 512];
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { rt.handle(&input, &mut out); })).is_err() {
                panics += 1;
                if panics < 4 { println!("PANIC(random): {:?}", String::from_utf8_lossy(&input)); }
            }
        }

        // 2. Truncations of every prefix of valid messages.
        for seed in seeds {
            for cut in 0..seed.len() {
                let mut out = [0u8; 512];
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { rt.handle(&seed[..cut], &mut out); })).is_err() {
                    panics += 1;
                    println!("PANIC(truncate@{cut}): {:?}", String::from_utf8_lossy(&seed[..cut]));
                }
            }
        }

        // 3. Single-byte mutations of valid messages.
        for seed in seeds {
            for pos in 0..seed.len() {
                for &b in alpha {
                    let mut m = seed.to_vec();
                    m[pos] = b;
                    let mut out = [0u8; 512];
                    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { rt.handle(&m, &mut out); })).is_err() {
                        panics += 1;
                        if panics < 8 { println!("PANIC(mutate): {:?}", String::from_utf8_lossy(&m)); }
                    }
                }
            }
        }

        // 4. Pathological nesting (stack-overflow check).
        for depth in [64usize, 1000, 10_000] {
            let mut deep = Vec::from(&b"{\"id\":1,\"method\":\"tools/list\",\"params\":"[..]);
            deep.extend(std::iter::repeat_n(b'[', depth));
            deep.extend(std::iter::repeat_n(b']', depth));
            deep.push(b'}');
            let mut out = [0u8; 512];
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| { rt.handle(&deep, &mut out); })).is_err() {
                panics += 1;
                println!("PANIC(nesting depth {depth})");
            }
        }

        // 5. Tiny output buffers against valid input (truncation path).
        for cap in 1..80usize {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                rt.handle(seeds[0], &mut buf[..cap])
            }));
            match r {
                Ok(n) => assert!(n == 0 || n <= cap, "wrote {n} into {cap}"),
                Err(_) => { panics += 1; println!("PANIC(out cap {cap})"); }
            }
        }

        println!("total panics: {panics}");
        assert_eq!(panics, 0, "handle() must never panic on untrusted input");
    }
}
