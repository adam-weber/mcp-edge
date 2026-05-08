//! End-to-end integration test for the dynamic gateway.
//!
//! Stands up a fake leaf on a Unix socket (single-threaded, hand-rolled to
//! avoid pulling in extra deps), an `Arc<Gateway>` serving its own Unix
//! socket via `serve_unix`, and a client socket that exercises:
//!
//! - `initialize` declares `tools.listChanged: true`.
//! - `tools/list` reflects leaves added via `gw.add_leaf` after the server
//!   started.
//! - Each `add_leaf` / `remove_leaf` from the test thread produces a
//!   `notifications/tools/list_changed` frame on the client socket within
//!   one second.

#![cfg(feature = "gateway")]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mcp_edge::DynamicGateway;

/// Spin up a one-tool fake leaf on `path`. Returns once the listener is
/// bound. The leaf thread runs forever until the test process exits.
fn spawn_fake_leaf(path: &str, tool_name: &str) {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path).expect("leaf bind");
    let tool_name = tool_name.to_string();
    std::thread::spawn(move || {
        for conn in listener.incoming().flatten() {
            let tool_name = tool_name.clone();
            std::thread::spawn(move || handle_leaf_conn(conn, &tool_name));
        }
    });
}

fn handle_leaf_conn(conn: UnixStream, tool_name: &str) {
    let mut writer = conn.try_clone().expect("leaf clone");
    let reader = BufReader::new(conn);
    for line in reader.lines().map_while(Result::ok) {
        if line.contains("\"initialize\"") {
            let _ = writeln!(
                writer,
                r#"{{"jsonrpc":"2.0","id":0,"result":{{"protocolVersion":"2024-11-05","capabilities":{{"tools":{{}}}}}}}}"#
            );
        } else if line.contains("\"tools/list\"") {
            let _ = writeln!(
                writer,
                r#"{{"jsonrpc":"2.0","id":1,"result":{{"tools":[{{"name":"{}","description":"d"}}]}}}}"#,
                tool_name
            );
        } else if line.contains("\"tools/call\"") {
            let _ = writeln!(
                writer,
                r#"{{"jsonrpc":"2.0","id":2,"result":{{"content":[{{"type":"text","text":"ok"}}]}}}}"#
            );
        }
    }
}

/// Read lines from a connected client socket with a deadline. Returns the
/// first matching line (or None on timeout).
fn read_until<F: Fn(&str) -> bool>(
    reader: &mut BufReader<UnixStream>,
    deadline: Instant,
    matches: F,
) -> Option<String> {
    while Instant::now() < deadline {
        let timeout = deadline.saturating_duration_since(Instant::now());
        let _ = reader.get_mut().set_read_timeout(Some(timeout.max(Duration::from_millis(10))));
        let mut buf = String::new();
        match reader.read_line(&mut buf) {
            Ok(0) => return None,
            Ok(_) => {
                if matches(&buf) {
                    return Some(buf);
                }
            }
            Err(_) => continue,
        }
    }
    None
}

#[test]
fn dynamic_add_and_remove_with_push_notifications() {
    // Pin to PID so parallel test runs don't collide on socket paths.
    let pid = std::process::id();
    let gw_sock = format!("/tmp/mcp-edge-test-gw-{pid}.sock");
    let leaf_a = format!("/tmp/mcp-edge-test-leafa-{pid}.sock");
    let leaf_b = format!("/tmp/mcp-edge-test-leafb-{pid}.sock");

    spawn_fake_leaf(&leaf_a, "tool_a");
    spawn_fake_leaf(&leaf_b, "tool_b");
    // Tiny grace period for both listeners to bind.
    std::thread::sleep(Duration::from_millis(20));

    let gw: Arc<DynamicGateway<4, 8, 2048>> = Arc::new(DynamicGateway::new());
    gw.add_leaf(&leaf_a).expect("add leaf-a at startup");

    // Spawn the gateway's own listener.
    {
        let gw = gw.clone();
        let path = gw_sock.clone();
        std::thread::spawn(move || gw.serve_unix(&path));
    }
    std::thread::sleep(Duration::from_millis(20));

    // Client connects.
    let conn = UnixStream::connect(&gw_sock).expect("client connect");
    conn.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut writer = conn;

    // initialize → expect listChanged: true.
    writeln!(
        writer,
        r#"{{"jsonrpc":"2.0","id":1,"method":"initialize"}}"#
    )
    .unwrap();
    let init_resp = read_until(
        &mut reader,
        Instant::now() + Duration::from_secs(2),
        |line| line.contains("\"id\":1"),
    )
    .expect("initialize response");
    assert!(
        init_resp.contains(r#""listChanged":true"#),
        "expected listChanged:true, got {init_resp}"
    );

    // tools/list → only tool_a so far.
    writeln!(
        writer,
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#
    )
    .unwrap();
    let list1 = read_until(
        &mut reader,
        Instant::now() + Duration::from_secs(2),
        |line| line.contains("\"id\":2"),
    )
    .expect("first tools/list response");
    assert!(list1.contains("tool_a"), "{list1}");
    assert!(!list1.contains("tool_b"), "{list1}");

    // From the test thread: dynamically add leaf_b. The push notification
    // should arrive on the client socket without us asking for it.
    gw.add_leaf(&leaf_b).expect("dynamic add leaf-b");
    let notify_add = read_until(
        &mut reader,
        Instant::now() + Duration::from_secs(2),
        |line| line.contains("notifications/tools/list_changed"),
    )
    .expect("expected list_changed notification on add");
    assert!(notify_add.contains("\"jsonrpc\":\"2.0\""), "{notify_add}");

    // tools/list → both tools now.
    writeln!(
        writer,
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/list"}}"#
    )
    .unwrap();
    let list2 = read_until(
        &mut reader,
        Instant::now() + Duration::from_secs(2),
        |line| line.contains("\"id\":3"),
    )
    .expect("second tools/list response");
    assert!(list2.contains("tool_a"), "{list2}");
    assert!(list2.contains("tool_b"), "{list2}");

    // Now remove leaf_a — expect another push notification.
    gw.remove_leaf(&leaf_a).expect("dynamic remove leaf-a");
    let notify_remove = read_until(
        &mut reader,
        Instant::now() + Duration::from_secs(2),
        |line| line.contains("notifications/tools/list_changed"),
    )
    .expect("expected list_changed notification on remove");
    assert!(notify_remove.contains("\"jsonrpc\":\"2.0\""), "{notify_remove}");

    // tools/list → only tool_b remains.
    writeln!(
        writer,
        r#"{{"jsonrpc":"2.0","id":4,"method":"tools/list"}}"#
    )
    .unwrap();
    let list3 = read_until(
        &mut reader,
        Instant::now() + Duration::from_secs(2),
        |line| line.contains("\"id\":4"),
    )
    .expect("third tools/list response");
    assert!(!list3.contains("tool_a"), "{list3}");
    assert!(list3.contains("tool_b"), "{list3}");

    // Cleanup.
    let _ = std::fs::remove_file(&gw_sock);
    let _ = std::fs::remove_file(&leaf_a);
    let _ = std::fs::remove_file(&leaf_b);
}
