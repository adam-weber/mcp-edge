//! Example: a gateway that aggregates two leaf devices.
//!
//! Start two leaves first (in separate terminals):
//!   cargo run --example sensor
//!   SOCK=/tmp/mcp-edge2.sock cargo run --example sensor  # TODO: make path configurable
//!
//! Then run the gateway:
//!   cargo run --example gateway --features gateway
//!
//! The gateway exposes all tools from all leaves under a single socket.
//! Agents connect only to the gateway; leaves are invisible to them.

use mcp_edge::transport::UnixTransport;
use mcp_edge::Gateway;

fn main() {
    let mut gw: Gateway<2, 16> = Gateway::new();

    // Connect to leaves and discover their tools.
    // add_leaf blocks until the leaf responds.
    gw.add_leaf("/tmp/leaf1.sock").expect("leaf1 not running — start it first");
    gw.add_leaf("/tmp/leaf2.sock").expect("leaf2 not running — start it first");

    println!("gateway ready");
    UnixTransport::new("/tmp/gateway.sock").serve(|msg, out| gw.handle(msg, out));
}
