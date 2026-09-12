//! Message framing, `no_std` and sans-io.
//!
//! MCP over a byte stream is one JSON-RPC object per line, in both
//! directions. [`Framer`] owns that rule and nothing else: it holds the rx
//! and tx buffers, reassembles messages split across reads, and hands each
//! complete line to your handler. It never touches a socket, a peripheral,
//! or an allocator, so the same code drives a Unix socket on Linux and a
//! UART on a microcontroller.
//!
//! # Wiring it to a device
//!
//! On a bare-metal target there is no `Read`/`Write` to implement against,
//! so `Framer` inverts the relationship: you own the peripheral and push
//! bytes in as they arrive.
//!
//! ```ignore
//! use mcp_edge::{Framer, Runtime};
//!
//! let mut rt: Runtime<'_, 2, 64> = Runtime::new();
//! rt.register(&sensor).unwrap();
//!
//! // RX caps one inbound message, TX one outbound response.
//! let mut framer: Framer<256, 256> = Framer::new();
//!
//! loop {
//!     let byte = uart.read_byte();
//!     framer.feed(
//!         &[byte],
//!         |msg, out| rt.handle(msg, out),
//!         |resp| uart.write_all(resp),
//!     ).ok();
//! }
//! ```
//!
//! `feed` accepts any chunk size, so a UART interrupt handing over one byte
//! and a USB-CDC endpoint handing over 64 work identically.
//!
//! # Reading straight into the buffer
//!
//! [`Framer::rx_space`] exposes the unfilled tail of the rx buffer so a
//! blocking read or a DMA transfer can land bytes in it directly, with no
//! intermediate copy. Tell the framer how many arrived with
//! [`Framer::commit`]:
//!
//! ```ignore
//! let n = uart.read(framer.rx_space());
//! framer.commit(n, |msg, out| rt.handle(msg, out), |resp| uart.write_all(resp)).ok();
//! ```
//!
//! This is the path the crate's own `std` transports take.

/// Why a [`Framer`] rejected input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameError {
    /// A message reached `RX` bytes without a newline terminator. The
    /// oversized frame is dropped and the framer resynchronizes on the next
    /// newline, so the tail of a dropped message is never parsed as a fresh
    /// request. Raise `RX` if legitimate messages are being rejected.
    Overflow,
}

/// Newline-delimited framing over a caller-owned byte stream.
///
/// - `RX`: largest inbound message accepted, in bytes.
/// - `TX`: largest response the handler may produce, in bytes.
///
/// Both buffers live inside the struct, so `size_of::<Framer<RX, TX>>()` is
/// `RX + TX` plus a couple of words of bookkeeping. Put it wherever you'd put
/// any other fixed buffer: a `static mut`, a stack frame, or a peripheral
/// driver struct.
pub struct Framer<const RX: usize = 256, const TX: usize = 512> {
    rx: [u8; RX],
    /// Valid bytes in `rx`.
    filled: usize,
    tx: [u8; TX],
    /// Set after an oversized frame. While set, bytes are discarded until a
    /// newline restores alignment with the sender's message boundaries.
    resyncing: bool,
}

impl<const RX: usize, const TX: usize> Framer<RX, TX> {
    pub const fn new() -> Self {
        Self { rx: [0u8; RX], filled: 0, tx: [0u8; TX], resyncing: false }
    }

    /// The unfilled tail of the rx buffer. Read or DMA into this, then call
    /// [`commit`](Self::commit) with the number of bytes written. Never
    /// empty: the framer clears its buffer rather than leaving no room.
    pub fn rx_space(&mut self) -> &mut [u8] {
        &mut self.rx[self.filled..]
    }

    /// Account for `n` bytes written into [`rx_space`](Self::rx_space) and
    /// dispatch every complete message now buffered.
    ///
    /// `handler` is called once per message with the message bytes and the tx
    /// buffer, and returns the number of response bytes written (0 to send
    /// nothing, which is what `Runtime::handle` does for notifications).
    /// `emit` receives each non-empty response.
    ///
    /// # Panics
    ///
    /// If `n` exceeds the length of the slice `rx_space` last returned.
    pub fn commit<H, E>(&mut self, n: usize, handler: H, emit: E) -> Result<(), FrameError>
    where
        H: Fn(&[u8], &mut [u8]) -> usize,
        E: FnMut(&[u8]),
    {
        assert!(n <= RX - self.filled, "commit(n) exceeds rx_space()");
        self.filled += n;
        self.drain(handler, emit)
    }

    /// Copy `incoming` into the rx buffer and dispatch whatever that
    /// completes. Accepts chunks larger than `RX`, processing them in
    /// buffer-sized passes.
    ///
    /// Prefer [`rx_space`](Self::rx_space) + [`commit`](Self::commit) when
    /// the bytes are not already in a buffer of your own; this entry point
    /// costs one extra copy.
    pub fn feed<H, E>(
        &mut self,
        mut incoming: &[u8],
        handler: H,
        mut emit: E,
    ) -> Result<(), FrameError>
    where
        H: Fn(&[u8], &mut [u8]) -> usize,
        E: FnMut(&[u8]),
    {
        let mut result = Ok(());
        while !incoming.is_empty() {
            let space = RX - self.filled;
            debug_assert!(space > 0, "drain always leaves room");
            let take = space.min(incoming.len());
            self.rx[self.filled..self.filled + take].copy_from_slice(&incoming[..take]);
            incoming = &incoming[take..];
            // `&H` is `Fn` and `&mut E` is `FnMut`, so the borrows below reuse
            // the caller's closures across passes without cloning them.
            if let Err(e) = self.commit(take, &handler, &mut emit) {
                result = Err(e);
            }
        }
        result
    }

    /// Discard buffered bytes and clear any resync state. Use when the
    /// underlying stream restarts (a reconnect, a link reset) and the next
    /// bytes are known to begin a fresh message.
    pub fn reset(&mut self) {
        self.filled = 0;
        self.resyncing = false;
    }

    /// Bytes currently held for an incomplete message.
    pub fn buffered(&self) -> usize {
        self.filled
    }

    fn drain<H, E>(&mut self, handler: H, mut emit: E) -> Result<(), FrameError>
    where
        H: Fn(&[u8], &mut [u8]) -> usize,
        E: FnMut(&[u8]),
    {
        // Recovering from an oversized frame: throw bytes away until the
        // sender's next message boundary.
        if self.resyncing {
            match self.rx[..self.filled].iter().position(|&b| b == b'\n') {
                Some(i) => {
                    self.rx.copy_within(i + 1..self.filled, 0);
                    self.filled -= i + 1;
                    self.resyncing = false;
                }
                None => {
                    self.filled = 0;
                    return Ok(());
                }
            }
        }

        let mut start = 0;
        while let Some(rel) = self.rx[start..self.filled].iter().position(|&b| b == b'\n') {
            let end = start + rel;
            // Skip blank lines rather than handing an empty message to the
            // handler, which would answer a parse error to a keepalive.
            if end > start {
                // Disjoint field borrows: the message reads `rx`, the handler
                // writes `tx`.
                let n = handler(&self.rx[start..end], &mut self.tx);
                if n > 0 {
                    emit(&self.tx[..n]);
                }
            }
            start = end + 1;
        }
        if start > 0 {
            self.rx.copy_within(start..self.filled, 0);
            self.filled -= start;
        }

        if self.filled == RX {
            self.filled = 0;
            self.resyncing = true;
            return Err(FrameError::Overflow);
        }
        Ok(())
    }
}

impl<const RX: usize, const TX: usize> Default for Framer<RX, TX> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Echoes `<len>\n` per message so tests can tell responses apart.
    fn echo_len(msg: &[u8], out: &mut [u8]) -> usize {
        out[0] = b'0' + msg.len().min(9) as u8;
        out[1] = b'\n';
        2
    }

    /// Fixed-capacity sink, so these tests exercise the same no-alloc path
    /// the crate runs on a microcontroller.
    struct Sink {
        buf: [u8; 128],
        len: usize,
    }

    impl Sink {
        fn new() -> Self { Self { buf: [0u8; 128], len: 0 } }
        fn push(&mut self, b: &[u8]) {
            self.buf[self.len..self.len + b.len()].copy_from_slice(b);
            self.len += b.len();
        }
        fn as_slice(&self) -> &[u8] { &self.buf[..self.len] }
    }

    fn collect<const RX: usize, const TX: usize>(
        f: &mut Framer<RX, TX>,
        chunk: &[u8],
    ) -> (Result<(), FrameError>, Sink) {
        let mut got = Sink::new();
        let r = f.feed(chunk, echo_len, |resp| got.push(resp));
        (r, got)
    }

    #[test]
    fn dispatches_one_complete_message() {
        let mut f: Framer<64, 16> = Framer::new();
        let (r, got) = collect(&mut f, b"abc\n");
        assert_eq!(r, Ok(()));
        assert_eq!(got.as_slice(), b"3\n");
        assert_eq!(f.buffered(), 0);
    }

    #[test]
    fn holds_partial_message_until_terminated() {
        let mut f: Framer<64, 16> = Framer::new();
        let (r, got) = collect(&mut f, b"abc");
        assert_eq!(r, Ok(()));
        assert!(got.as_slice().is_empty(), "no newline yet, nothing to dispatch");
        assert_eq!(f.buffered(), 3);
        let (r, got) = collect(&mut f, b"de\n");
        assert_eq!(r, Ok(()));
        assert_eq!(got.as_slice(), b"5\n", "reassembled across two feeds");
    }

    #[test]
    fn byte_at_a_time_matches_bulk() {
        // The UART case: one byte per interrupt.
        let mut f: Framer<64, 16> = Framer::new();
        let mut got = Sink::new();
        for b in b"abcd\nef\n" {
            f.feed(&[*b], echo_len, |r| got.push(r)).unwrap();
        }
        assert_eq!(got.as_slice(), b"4\n2\n");
    }

    #[test]
    fn multiple_messages_in_one_chunk() {
        let mut f: Framer<64, 16> = Framer::new();
        let (r, got) = collect(&mut f, b"a\nbb\nccc\n");
        assert_eq!(r, Ok(()));
        assert_eq!(got.as_slice(), b"1\n2\n3\n");
    }

    #[test]
    fn blank_lines_are_skipped() {
        let mut f: Framer<64, 16> = Framer::new();
        let (r, got) = collect(&mut f, b"\n\nab\n\n");
        assert_eq!(r, Ok(()));
        assert_eq!(got.as_slice(), b"2\n", "empty lines must not reach the handler");
    }

    #[test]
    fn oversized_frame_reports_overflow_then_resyncs() {
        let mut f: Framer<8, 16> = Framer::new();
        // 12 bytes with no newline: overruns RX=8.
        let (r, got) = collect(&mut f, b"xxxxxxxxxxxx");
        assert_eq!(r, Err(FrameError::Overflow));
        assert!(got.as_slice().is_empty());
        // The tail of the dropped message must not be parsed as a request.
        let (r, got) = collect(&mut f, b"yyy\nok\n");
        assert_eq!(r, Ok(()), "overflow is reported once, not repeatedly");
        assert_eq!(got.as_slice(), b"2\n", "resynced on the newline; only `ok` dispatched");
    }

    #[test]
    fn chunk_larger_than_rx_is_processed_in_passes() {
        let mut f: Framer<8, 16> = Framer::new();
        let (r, got) = collect(&mut f, b"aa\nbb\ncc\ndd\nee\n");
        assert_eq!(r, Ok(()), "each message fits RX even though the chunk does not");
        assert_eq!(got.as_slice(), b"2\n2\n2\n2\n2\n");
    }

    #[test]
    fn exactly_full_buffer_with_terminator_is_fine() {
        let mut f: Framer<4, 16> = Framer::new();
        let (r, got) = collect(&mut f, b"abc\n");
        assert_eq!(r, Ok(()), "a message filling RX exactly is not an overflow");
        assert_eq!(got.as_slice(), b"3\n");
    }

    #[test]
    fn rx_space_and_commit_round_trip() {
        let mut f: Framer<32, 16> = Framer::new();
        let src = b"hello\n";
        f.rx_space()[..src.len()].copy_from_slice(src);
        let mut got = Sink::new();
        f.commit(src.len(), echo_len, |r| got.push(r)).unwrap();
        assert_eq!(got.as_slice(), b"5\n");
    }

    #[test]
    fn rx_space_is_never_empty_after_overflow() {
        let mut f: Framer<4, 16> = Framer::new();
        let _ = collect(&mut f, b"xxxxxx");
        assert!(!f.rx_space().is_empty(), "overflow must leave room to keep reading");
    }

    #[test]
    fn reset_clears_partial_state() {
        let mut f: Framer<16, 16> = Framer::new();
        let _ = collect(&mut f, b"partial");
        assert_eq!(f.buffered(), 7);
        f.reset();
        assert_eq!(f.buffered(), 0);
        let (_, got) = collect(&mut f, b"ab\n");
        assert_eq!(got.as_slice(), b"2\n", "no leftover bytes prepended");
    }

    #[test]
    fn handler_returning_zero_emits_nothing() {
        let mut f: Framer<32, 16> = Framer::new();
        let mut calls = 0;
        f.feed(b"abc\n", |_, _| 0, |_| calls += 1).unwrap();
        assert_eq!(calls, 0, "notifications produce no outbound bytes");
    }

    #[test]
    fn end_to_end_with_runtime() {
        use crate::{Output, Provider, Runtime, Tool};
        use core::fmt::Write;

        struct Temp;
        impl Provider for Temp {
            fn tools(&self) -> &[Tool] {
                &[Tool { name: "temp_read", description: "Read temperature" }]
            }
            fn call(&self, _: &str, _: &[u8], out: &mut Output) -> Result<(), &'static str> {
                write!(out, "22.5").unwrap();
                Ok(())
            }
        }
        static T: Temp = Temp;
        let mut rt: Runtime<'_, 1, 64> = Runtime::new();
        rt.register(&T).unwrap();

        // Split mid-message to prove reassembly drives the real runtime.
        let mut f: Framer<128, 128> = Framer::new();
        let mut got = Sink::new();
        let msg = br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"temp_read","arguments":{}}}"#;
        let (a, b) = msg.split_at(40);
        f.feed(a, |m, o| rt.handle(m, o), |r| got.push(r)).unwrap();
        assert!(got.as_slice().is_empty(), "incomplete message must not dispatch");
        f.feed(b, |m, o| rt.handle(m, o), |r| got.push(r)).unwrap();
        f.feed(b"\n", |m, o| rt.handle(m, o), |r| got.push(r)).unwrap();
        let s = core::str::from_utf8(got.as_slice()).unwrap();
        assert!(s.contains("22.5"), "{s}");
    }
}
