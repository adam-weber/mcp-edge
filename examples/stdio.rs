//! Example: an MCP server callable over stdio.
//!
//! Most MCP hosts spawn local servers as subprocesses and pipe JSON-RPC
//! over stdin/stdout. This file is a minimal target for that pattern.
//!
//! Build it:
//!
//!   cargo build --release --example stdio
//!
//! Verify it speaks the protocol without any host:
//!
//!   echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
//!     | ./target/release/examples/stdio
//!
//! See `examples/README.md` for host-side registration.

use core::fmt::Write;
use mcp_edge::transport::StdioTransport;
use mcp_edge::{Output, Provider, Runtime, Tool};

struct TempSensor;

impl Provider for TempSensor {
    fn tools(&self) -> &[Tool] {
        &[Tool {
            name: "temp_read",
            description: "Read the current temperature in Celsius",
        }]
    }

    fn call(&self, _tool: &str, _args: &[u8], out: &mut Output) -> Result<(), &'static str> {
        // Replace with your real sensor read; fixed value here for demo.
        write!(out, "22.5").unwrap();
        Ok(())
    }
}

fn main() {
    let sensor = TempSensor;
    let mut rt: Runtime<'_, 1> = Runtime::new();
    rt.register(&sensor).unwrap();

    // No socket, no port — just JSON-RPC over stdin/stdout.
    StdioTransport::serve(|msg, out| rt.handle(msg, out));
}
