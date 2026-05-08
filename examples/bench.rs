//! Performance benchmark for `mcp-edge`. Run with:
//!
//!   cargo run --profile bench-fast --example bench --features gateway
//!
//! Measures:
//!   - Runtime in-process throughput (tools/list, tools/call)
//!   - Static `Gateway::handle` throughput (cached tools/list)
//!   - `DynamicGateway::handle` throughput (read-lock cost vs static)
//!   - Full proxy round-trip latency (Unix-socket leaf, real syscalls)
//!   - Mutation throughput (`add_leaf` / `remove_leaf` with no subscribers)
//!   - Broadcast fan-out latency (1, 4, 16 simultaneous subscribers)
//!   - Concurrent-client throughput over a real Unix socket
//!
//! Zero deps — pure `std::time::Instant` and hand-rolled percentiles. Each
//! bench warms up briefly, runs for a fixed budget, and prints
//! ops/sec + mean + P50 + P99 latency.

use core::fmt::Write as FmtWrite;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use mcp_edge::{DynamicGateway, Gateway, Output, Provider, Runtime, Tool};

// ===========================================================================
// Generic timing harness
// ===========================================================================

struct Stats {
    ops: u64,
    elapsed: Duration,
    samples: Vec<Duration>, // per-iteration latencies, sorted at print time
}

impl Stats {
    fn print(&self, label: &str) {
        // Mean and percentiles both come from `samples` so the numbers stay
        // self-consistent even when the outer loop has un-timed work
        // between samples (e.g. the re-add waits in the broadcast bench).
        // `ops/sec` is throughput against wall-clock elapsed and reflects
        // the loop's overall rate, not just the timed portion.
        let mut s = self.samples.clone();
        s.sort_unstable();
        let n = s.len().max(1);
        let mean_ns = s.iter().map(|d| d.as_nanos()).sum::<u128>() as f64 / n as f64;
        let p50 = s.get(s.len() / 2).copied().unwrap_or(Duration::ZERO);
        let p99 = s
            .get(s.len().saturating_sub(s.len() / 100).saturating_sub(1))
            .copied()
            .unwrap_or(Duration::ZERO);
        let ops_per_sec = (self.ops as f64) / self.elapsed.as_secs_f64();
        println!(
            "{:50} {:>10.0} ops/s    mean {:>7.2} µs   p50 {:>7.2} µs   p99 {:>7.2} µs",
            label,
            ops_per_sec,
            mean_ns / 1000.0,
            p50.as_nanos() as f64 / 1000.0,
            p99.as_nanos() as f64 / 1000.0,
        );
    }
}

/// Run `f()` in a tight loop for `budget`, recording per-iteration latency
/// for percentile reporting. Warms up for 50 ms first so caches are hot
/// before we start measuring.
fn time_loop<F: FnMut()>(budget: Duration, mut f: F) -> Stats {
    let warmup_end = Instant::now() + Duration::from_millis(50);
    while Instant::now() < warmup_end {
        f();
    }
    let mut samples = Vec::with_capacity(1 << 16);
    let start = Instant::now();
    let deadline = start + budget;
    while Instant::now() < deadline {
        let t0 = Instant::now();
        f();
        samples.push(t0.elapsed());
    }
    Stats {
        ops: samples.len() as u64,
        elapsed: start.elapsed(),
        samples,
    }
}

// ===========================================================================
// Test fixtures
// ===========================================================================

struct PingProvider;
impl Provider for PingProvider {
    fn tools(&self) -> &[Tool] {
        static T: &[Tool] = &[Tool {
            name: "ping",
            description: "responds with pong",
        }];
        T
    }
    fn call(&self, _t: &str, _a: &[u8], out: &mut Output) -> Result<(), &'static str> {
        write!(out, "pong").map_err(|_| "out full")
    }
}

const TOOLS_LIST_REQ: &[u8] = br#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;
const TOOLS_CALL_REQ: &[u8] =
    br#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"ping","arguments":{}}}"#;

/// Spawn a deterministic Unix-socket leaf that exposes a single `ping` tool.
/// Replies to `initialize`, `tools/list`, and `tools/call` with hard-coded
/// JSON. Returns the path; the listener thread runs until the process exits.
fn spawn_fake_leaf(label: &str) -> String {
    let path = format!("/tmp/mcp-edge-bench-{label}-{}.sock", std::process::id());
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).expect("bind leaf");
    std::thread::Builder::new()
        .stack_size(64 * 1024)
        .spawn(move || {
            for conn in listener.incoming().flatten() {
                std::thread::Builder::new()
                    .stack_size(64 * 1024)
                    .spawn(move || handle_leaf_conn(conn))
                    .ok();
            }
        })
        .ok();
    // Tiny grace period for the listener to bind before callers connect.
    std::thread::sleep(Duration::from_millis(20));
    path
}

fn handle_leaf_conn(conn: UnixStream) {
    let mut writer = match conn.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let reader = BufReader::new(conn);
    for line in reader.lines().map_while(Result::ok) {
        let resp: &[u8] = if line.contains("\"initialize\"") {
            br#"{"jsonrpc":"2.0","id":0,"result":{"protocolVersion":"2024-11-05","capabilities":{"tools":{}}}}
"#
        } else if line.contains("\"tools/list\"") {
            br#"{"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"ping","description":"d"}]}}
"#
        } else if line.contains("\"tools/call\"") {
            br#"{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"pong"}]}}
"#
        } else {
            continue;
        };
        if writer.write_all(resp).is_err() {
            break;
        }
    }
}

// ===========================================================================
// In-process benchmarks (no syscalls)
// ===========================================================================

fn bench_runtime() {
    println!("\nRuntime — in-process (no syscalls)");
    let provider = PingProvider;
    let mut rt: Runtime<'_, 1, 256> = Runtime::new();
    rt.register(&provider).unwrap();

    let mut out = [0u8; 1024];
    time_loop(Duration::from_millis(500), || {
        let _ = rt.handle(TOOLS_LIST_REQ, &mut out);
    })
    .print("  tools/list");
    time_loop(Duration::from_millis(500), || {
        let _ = rt.handle(TOOLS_CALL_REQ, &mut out);
    })
    .print("  tools/call");
}

fn bench_gateways_in_process() {
    println!("\nGateway — in-process (cached tools/list, no leaf I/O)");
    let mut gw: Gateway = Gateway::new();
    // Seed via the inner — bypasses real network discovery so we're measuring
    // pure dispatch cost.
    {
        let _ = &mut gw; // visibility hint
    }
    {
        // Reach into module-private state for fixturing — works because the
        // example lives outside the crate, so we have to go through real
        // public API. Use a fake leaf for one round-trip to populate.
        let leaf = spawn_fake_leaf("static");
        gw.add_leaf(&leaf).expect("seed static gateway");
    }

    let mut out = [0u8; 1024];
    time_loop(Duration::from_millis(500), || {
        let _ = gw.handle(TOOLS_LIST_REQ, &mut out);
    })
    .print("  Gateway::handle tools/list (lock-free)");

    let dgw: DynamicGateway = DynamicGateway::new();
    let leaf2 = spawn_fake_leaf("dyn");
    dgw.add_leaf(&leaf2).expect("seed dynamic gateway");
    time_loop(Duration::from_millis(500), || {
        let _ = dgw.handle(TOOLS_LIST_REQ, &mut out);
    })
    .print("  DynamicGateway::handle tools/list (RwLock read)");
}

// ===========================================================================
// Full proxy round-trip via real Unix sockets
// ===========================================================================

fn bench_full_proxy() {
    println!("\nFull proxy round-trip (real Unix sockets)");
    let leaf = spawn_fake_leaf("proxy");
    let mut gw: Gateway = Gateway::new();
    gw.add_leaf(&leaf).expect("add_leaf for proxy bench");

    let mut out = [0u8; 1024];
    time_loop(Duration::from_millis(500), || {
        // Each call opens a fresh UnixStream to the leaf, sends the
        // tools/call request, reads the response, and writes the framed
        // reply into out. Includes connect + write + read + parse syscalls.
        let _ = gw.handle(TOOLS_CALL_REQ, &mut out);
    })
    .print("  tools/call (Gateway → leaf)");
}

// ===========================================================================
// Mutation + broadcast fan-out
// ===========================================================================

fn bench_mutation_throughput() {
    println!("\nMutation throughput (DynamicGateway, no subscribers)");
    let leaf = spawn_fake_leaf("mut");
    let dgw: DynamicGateway = DynamicGateway::new();

    // add → remove → add → remove ... — measures the round-trip cost.
    time_loop(Duration::from_millis(500), || {
        dgw.add_leaf(&leaf).expect("add");
        dgw.remove_leaf(&leaf).expect("remove");
    })
    .print("  add_leaf + remove_leaf (one cycle)");
}

fn bench_broadcast_fanout() {
    println!("\nBroadcast fan-out (DynamicGateway → N subscribers)");
    for &n in &[1usize, 4, 16] {
        let leaf = spawn_fake_leaf(&format!("fan{n}"));
        let dgw: Arc<DynamicGateway> = Arc::new(DynamicGateway::new());
        dgw.add_leaf(&leaf).expect("seed");

        // Spin up the gateway listener.
        let gw_path = format!(
            "/tmp/mcp-edge-bench-fan{n}-gw-{}.sock",
            std::process::id()
        );
        {
            let dgw = dgw.clone();
            let path = gw_path.clone();
            std::thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || dgw.serve_unix(&path))
                .ok();
            std::thread::sleep(Duration::from_millis(20));
        }

        // Connect N subscribers; each just reads bytes off its socket and
        // counts list_changed notifications.
        let counter = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let s = UnixStream::connect(&gw_path).expect("client connect");
            s.set_read_timeout(Some(Duration::from_secs(2))).ok();
            let counter = counter.clone();
            let stop = stop.clone();
            handles.push(
                std::thread::Builder::new()
                    .stack_size(64 * 1024)
                    .spawn(move || {
                        let mut r = BufReader::new(s);
                        let mut buf = String::new();
                        while !stop.load(Ordering::Acquire) {
                            buf.clear();
                            match r.read_line(&mut buf) {
                                Ok(0) | Err(_) => return,
                                Ok(_) => {
                                    if buf.contains("notifications/tools/list_changed") {
                                        counter.fetch_add(1, Ordering::Release);
                                    }
                                }
                            }
                        }
                    })
                    .unwrap(),
            );
        }
        // Brief delay so each subscriber's connection-thread is in its read loop.
        std::thread::sleep(Duration::from_millis(50));

        // The gateway is pre-seeded with `leaf`. Each iteration: remove
        // (measured — the broadcast we time) then re-add (un-measured but
        // we still wait for its broadcast so the next iteration can remove
        // again). Spin on the counter so we measure delivery, not OS
        // wake-up scheduling.
        let mut samples = Vec::new();
        let start = Instant::now();
        let mut iters = 0u64;
        while start.elapsed() < Duration::from_millis(300) {
            counter.store(0, Ordering::Release);
            let t0 = Instant::now();
            dgw.remove_leaf(&leaf).expect("remove");
            while counter.load(Ordering::Acquire) < n {
                std::hint::spin_loop();
            }
            samples.push(t0.elapsed());
            iters += 1;
            counter.store(0, Ordering::Release);
            dgw.add_leaf(&leaf).expect("re-add");
            while counter.load(Ordering::Acquire) < n {
                std::hint::spin_loop();
            }
        }
        let elapsed = start.elapsed();
        let stats = Stats {
            ops: iters,
            elapsed,
            samples,
        };
        let mut label = String::new();
        let _ = write!(label, "  add_leaf → all {n} clients see push");
        stats.print(&label);

        // Tear down subscribers.
        stop.store(true, Ordering::Release);
        for h in handles {
            // Closing our side will EOF the subscriber thread.
            let _ = h.join();
        }
        let _ = std::fs::remove_file(&gw_path);
    }
}

// ===========================================================================
// Concurrent-client throughput
// ===========================================================================

fn bench_concurrent_clients() {
    println!("\nConcurrent-client throughput (DynamicGateway via Unix socket)");
    for &n in &[1usize, 4, 16] {
        let leaf = spawn_fake_leaf(&format!("conc{n}"));
        let dgw: Arc<DynamicGateway> = Arc::new(DynamicGateway::new());
        dgw.add_leaf(&leaf).expect("seed");
        let gw_path = format!(
            "/tmp/mcp-edge-bench-conc{n}-gw-{}.sock",
            std::process::id()
        );
        {
            let dgw = dgw.clone();
            let path = gw_path.clone();
            std::thread::Builder::new()
                .stack_size(64 * 1024)
                .spawn(move || dgw.serve_unix(&path))
                .ok();
            std::thread::sleep(Duration::from_millis(20));
        }

        // N client threads, each hammering tools/list as fast as possible.
        // tools/call would also exercise the proxy path but adds the leaf's
        // serialization to each measurement; tools/list isolates the gateway
        // dispatch + read-lock + transport framing.
        let stop = Arc::new(AtomicBool::new(false));
        let total = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(n);
        for _ in 0..n {
            let stop = stop.clone();
            let total = total.clone();
            let path = gw_path.clone();
            handles.push(
                std::thread::Builder::new()
                    .stack_size(64 * 1024)
                    .spawn(move || {
                        let mut s = UnixStream::connect(&path).expect("conn");
                        s.set_read_timeout(Some(Duration::from_secs(2))).ok();
                        let mut buf = [0u8; 1024];
                        let mut count = 0usize;
                        while !stop.load(Ordering::Acquire) {
                            if s.write_all(TOOLS_LIST_REQ).is_err() {
                                break;
                            }
                            if s.write_all(b"\n").is_err() {
                                break;
                            }
                            // Read until newline. The gateway always appends one.
                            loop {
                                let n = match s.read(&mut buf) {
                                    Ok(0) | Err(_) => return,
                                    Ok(n) => n,
                                };
                                if buf[..n].contains(&b'\n') {
                                    break;
                                }
                            }
                            count += 1;
                        }
                        total.fetch_add(count, Ordering::Release);
                    })
                    .unwrap(),
            );
        }

        let start = Instant::now();
        std::thread::sleep(Duration::from_millis(500));
        stop.store(true, Ordering::Release);
        for h in handles {
            let _ = h.join();
        }
        let elapsed = start.elapsed();
        let total_ops = total.load(Ordering::Acquire) as u64;
        let ops_per_sec = (total_ops as f64) / elapsed.as_secs_f64();
        println!(
            "  {n:>2} clients hammering tools/list   {ops_per_sec:>10.0} req/s aggregate"
        );
        let _ = std::fs::remove_file(&gw_path);
    }
}

// ===========================================================================
// Entry point
// ===========================================================================

fn main() {
    println!("mcp-edge benchmark");
    println!(
        "  build profile: {}",
        if cfg!(debug_assertions) {
            "debug (rerun with --release for real numbers!)"
        } else {
            "release"
        }
    );
    println!(
        "  arch: {} / {}",
        std::env::consts::OS,
        std::env::consts::ARCH
    );

    bench_runtime();
    bench_gateways_in_process();
    bench_full_proxy();
    bench_mutation_throughput();
    bench_broadcast_fanout();
    bench_concurrent_clients();
}
