//! Example: a gateway that aggregates two leaf devices.
//!
//! Start two leaves first, each exposing a single distinct tool:
//!   TOOL=temp     SOCK=/tmp/leaf1.sock cargo run --example sensor
//!   TOOL=humidity SOCK=/tmp/leaf2.sock cargo run --example sensor
//!
//! Then run the gateway:
//!   cargo run --example gateway --features gateway
//!
//! Override sockets with env vars: LEAF1, LEAF2, SOCK.
//!
//! The gateway exposes all tools from all leaves under a single socket.
//! Agents connect only to the gateway; leaves are invisible to them.

use mcp_edge::transport::UnixTransport;
use mcp_edge::Gateway;

fn main() {
    let mut gw: Gateway<2, 16> = Gateway::new();

    let leaf1 = std::env::var("LEAF1").unwrap_or_else(|_| "/tmp/leaf1.sock".into());
    let leaf2 = std::env::var("LEAF2").unwrap_or_else(|_| "/tmp/leaf2.sock".into());

    // add_leaf blocks until the leaf responds; treat any error as fatal.
    gw.add_leaf(&leaf1).expect("leaf1 unavailable — start it first");
    gw.add_leaf(&leaf2).expect("leaf2 unavailable — start it first");

    let path = std::env::var("SOCK").unwrap_or_else(|_| "/tmp/gateway.sock".into());
    println!("gateway listening on {path}");
    UnixTransport::new(&path).serve(|msg, out| gw.handle(msg, out));
}
