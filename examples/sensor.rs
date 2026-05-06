//! Example: mock sensors exposed over a Unix socket.
//!
//! Run (defaults: both tools, /tmp/mcp-edge.sock):
//!   cargo run --example sensor
//!
//! For the gateway demo, run two single-tool instances on distinct sockets:
//!   TOOL=temp     SOCK=/tmp/leaf1.sock cargo run --example sensor
//!   TOOL=humidity SOCK=/tmp/leaf2.sock cargo run --example sensor
//!
//! Test:  echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | nc -U /tmp/mcp-edge.sock

use core::fmt::Write;
use mcp_edge::transport::UnixTransport;
use mcp_edge::{Output, Provider, Runtime, Tool};

struct TempSensor;

impl Provider for TempSensor {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "temp_read", description: "Read temperature in Celsius" }]
    }

    fn call(&self, _tool: &str, _args: &[u8], out: &mut Output) -> Result<(), &'static str> {
        write!(out, "22.5").unwrap();
        Ok(())
    }
}

struct HumiditySensor;

impl Provider for HumiditySensor {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "humidity_read", description: "Read relative humidity (%)" }]
    }

    fn call(&self, _tool: &str, _args: &[u8], out: &mut Output) -> Result<(), &'static str> {
        write!(out, "45.0").unwrap();
        Ok(())
    }
}

fn main() {
    let temp = TempSensor;
    let humidity = HumiditySensor;

    let mut rt: Runtime<'_, 2> = Runtime::new();
    match std::env::var("TOOL").ok().as_deref() {
        Some("temp")     => rt.register(&temp).unwrap(),
        Some("humidity") => rt.register(&humidity).unwrap(),
        _ => {
            rt.register(&temp).unwrap();
            rt.register(&humidity).unwrap();
        }
    }

    let path = std::env::var("SOCK").unwrap_or_else(|_| "/tmp/mcp-edge.sock".into());
    UnixTransport::new(&path).serve(|msg, out| rt.handle(msg, out));
}
