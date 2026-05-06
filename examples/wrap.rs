//! Example: wrapping an existing sensor.
//!
//! "I already have a humidity sensor / thermometer / something publishing
//! data somewhere. Does mcp-edge wrap it, or do I rewrite my sensor?"
//!
//! You write a small Rust wrapper — typically 5–10 lines per tool — that
//! reads from wherever your existing source lives and writes the value into
//! `out`. mcp-edge is the protocol surface, not the sensor. Your driver code
//! stays where it is; the `Provider` impl is the adapter.
//!
//! Common backing sources and the shape of the wrapper for each:
//!
//! - Linux sysfs file        → `std::fs::read_to_string(path)`
//! - Existing CLI tool       → `std::process::Command::new(bin).output()`
//! - Daemon on a Unix socket → `UnixStream::connect(path)` and read
//! - HTTP endpoint           → bring your own HTTP client crate
//! - MQTT topic              → subscribe in a background thread, cache the
//!   latest value, return it from `call()`
//! - Hardware register       → exactly your existing driver code
//!
//! This file shows the two most common patterns. Both are ~5 lines of body
//! per tool — most of the file is boilerplate you only write once.
//!
//! Note: the sysfs read (`cpu_temp` tool) is Linux-only; on macOS/Windows it
//! returns an error. The subprocess pattern (`hostname`) is portable.
//!
//! Run:  cargo run --example wrap
//! Test: echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | nc -U /tmp/mcp-edge.sock

use core::fmt::Write;
use mcp_edge::transport::UnixTransport;
use mcp_edge::{Output, Provider, Runtime, Tool};

/// Pattern 1 — read from a sysfs file.
///
/// Linux exposes most temperature, voltage, humidity, fan, and battery
/// sensors as files under `/sys/class/...`. The wrapper is a `read_to_string`
/// + a parse + a `write!`. Replace the path with whatever your sensor exposes.
struct CpuTemp;

impl Provider for CpuTemp {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "cpu_temp", description: "CPU temperature in Celsius" }]
    }
    fn call(&self, _: &str, _: &[u8], out: &mut Output) -> Result<(), &'static str> {
        // Linux-only path; on a Pi, also try /sys/class/thermal/thermal_zone*/temp.
        let raw = std::fs::read_to_string("/sys/class/thermal/thermal_zone0/temp")
            .map_err(|_| "sysfs read failed")?;
        let millideg: i32 = raw.trim().parse().map_err(|_| "parse failed")?;
        write!(out, "{:.1}", millideg as f32 / 1000.0).unwrap();
        Ok(())
    }
}

/// Pattern 2 — shell out to an existing command-line tool.
///
/// When your sensor is already a Python script, a vendor binary, or any tool
/// that prints a value, just spawn it. Output capture + parse + `write!`.
struct HostName;

impl Provider for HostName {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "hostname", description: "Machine hostname" }]
    }
    fn call(&self, _: &str, _: &[u8], out: &mut Output) -> Result<(), &'static str> {
        let output = std::process::Command::new("hostname")
            .output()
            .map_err(|_| "spawn failed")?;
        let s = core::str::from_utf8(&output.stdout).map_err(|_| "non-utf8 output")?;
        write!(out, "{}", s.trim()).unwrap();
        Ok(())
    }
}

fn main() {
    let cpu = CpuTemp;
    let host = HostName;
    let mut rt: Runtime<'_, 2> = Runtime::new();
    rt.register(&cpu).unwrap();
    rt.register(&host).unwrap();

    let path = std::env::var("SOCK").unwrap_or_else(|_| "/tmp/mcp-edge.sock".into());
    UnixTransport::new(&path).serve(|m, o| rt.handle(m, o));
}
