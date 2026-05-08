//! Example: a gateway that aggregates two leaf devices, then dynamically
//! adds a third leaf 3 seconds after start to demonstrate live mutation
//! and `notifications/tools/list_changed` push.
//!
//! Start two leaves first, each exposing a single distinct tool:
//!   TOOL=temp     SOCK=/tmp/leaf1.sock cargo run --example sensor
//!   TOOL=humidity SOCK=/tmp/leaf2.sock cargo run --example sensor
//!   (optionally) TOOL=pressure SOCK=/tmp/leaf3.sock cargo run --example sensor
//!
//! Then run the gateway:
//!   cargo run --example gateway --features gateway
//!
//! Connect a client to /tmp/gateway.sock, send `tools/list` immediately —
//! you'll see two tools. Wait ~3 seconds; you'll see a
//! `notifications/tools/list_changed` arrive on the same connection if
//! /tmp/leaf3.sock exists. Send `tools/list` again — three tools.

use std::sync::Arc;
use std::time::Duration;

use mcp_edge::DynamicGateway;

fn main() {
    let gw: Arc<DynamicGateway<4, 16>> = Arc::new(DynamicGateway::new());

    let leaf1 = std::env::var("LEAF1").unwrap_or_else(|_| "/tmp/leaf1.sock".into());
    let leaf2 = std::env::var("LEAF2").unwrap_or_else(|_| "/tmp/leaf2.sock".into());
    let leaf3 = std::env::var("LEAF3").unwrap_or_else(|_| "/tmp/leaf3.sock".into());

    // add_leaf blocks until the leaf responds; treat any error as fatal.
    gw.add_leaf(&leaf1).expect("leaf1 unavailable — start it first");
    gw.add_leaf(&leaf2).expect("leaf2 unavailable — start it first");

    // Demo: 3 seconds in, try to add a third leaf. If it's not running,
    // log and move on. If it succeeds, every connected client receives a
    // notifications/tools/list_changed automatically.
    {
        let gw = gw.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            match gw.add_leaf(&leaf3) {
                Ok(()) => eprintln!("dynamically added {leaf3}"),
                Err(e) => eprintln!("could not add {leaf3}: {e} (start it to see the push)"),
            }
        });
    }

    let path = std::env::var("SOCK").unwrap_or_else(|_| "/tmp/gateway.sock".into());
    println!("gateway listening on {path}");
    gw.serve_unix(&path);
}
