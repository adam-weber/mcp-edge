//! Example: two mock sensors exposed over a Unix socket.
//!
//! Run:  cargo run --example sensor
//! Test: echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | nc -U /tmp/mcp-edge.sock

use mcp_edge::transport::UnixTransport;
use mcp_edge::{Provider, Runtime, Tool, ToolResult};

// ---------------------------------------------------------------------------
// Providers are plain structs — no Box, no Arc, no heap.
// On a real device these would read from hardware registers or I2C/SPI.
// ---------------------------------------------------------------------------

struct TempSensor;

impl Provider for TempSensor {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "temp_read", description: "Read temperature in Celsius" }]
    }

    fn call(&self, _tool: &str, _params: &str, out: &mut [u8]) -> (ToolResult, usize) {
        let s = b"22.5";
        out[..s.len()].copy_from_slice(s);
        (ToolResult::Ok, s.len())
    }
}

struct HumiditySensor;

impl Provider for HumiditySensor {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "humidity_read", description: "Read relative humidity (%)" }]
    }

    fn call(&self, _tool: &str, _params: &str, out: &mut [u8]) -> (ToolResult, usize) {
        let s = b"45.0";
        out[..s.len()].copy_from_slice(s);
        (ToolResult::Ok, s.len())
    }
}

fn main() {
    let temp = TempSensor;
    let humidity = HumiditySensor;

    // Runtime<'_, N, OUT>:
    //   N=2   — two providers
    //   OUT   — default 512 bytes max tool output (omit to use default)
    let mut rt: Runtime<'_, 2> = Runtime::new();
    rt.register(&temp);
    rt.register(&humidity);

    UnixTransport::new("/tmp/mcp-edge.sock").serve(&rt);
}
