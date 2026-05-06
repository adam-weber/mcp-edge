# mcp-edge

A minimal MCP runtime for resource-constrained devices. `no_std`-compatible core, zero heap allocation, fits in tens of KB of RAM.

## Getting Started

### Install

```bash
cargo add mcp-edge
```

Optional features:

- `std` (default) — enables `UnixTransport`
- `gateway` — enables `Gateway` for aggregating multiple leaves under one socket (requires `std`)

### Minimal Example

```rust
use core::fmt::Write;
use mcp_edge::transport::UnixTransport;
use mcp_edge::{Output, Provider, Runtime, Tool};

struct TempSensor;

impl Provider for TempSensor {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "temp_read", description: "Read temperature in Celsius" }]
    }

    fn call(&self, _tool: &str, _args: &[u8], out: &mut Output) -> Result<(), &'static str> {
        // Replace with your actual sensor read.
        write!(out, "22.5").unwrap();
        Ok(())
    }
}

fn main() {
    let sensor = TempSensor;
    let mut rt: Runtime<'_, 1> = Runtime::new();
    rt.register(&sensor).unwrap();
    UnixTransport::new("/tmp/mcp-edge.sock").serve(|msg, out| rt.handle(msg, out));
}
```

### Test It

```bash
# Terminal 1: run the example sensor
cargo run --example sensor

# Terminal 2: list tools, then call one
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/mcp-edge.sock

echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"temp_read","arguments":{}}}' \
  | nc -U /tmp/mcp-edge.sock
```

Any MCP-compatible agent can now discover and use your sensor.

## How It Works

mcp-edge is a library, not a framework. Your program:

1. Constructs a `Runtime<N, OUT>` — `N` is the max provider count, `OUT` is the max bytes a single tool result may produce.
2. Registers providers (each exposes one or more tools).
3. Calls `UnixTransport::serve(|msg, out| rt.handle(msg, out))` to accept connections.

The runtime parses JSON-RPC, dispatches `initialize`, `tools/list`, and `tools/call`, and writes replies into a caller-provided buffer. All sizes are const-generic, so the entire runtime lives on the stack — no allocator required.

```
┌───────────────────────────────────────────────┐
│  Your device                                  │
│                                               │
│  ┌───────────────────────────────────────┐    │
│  │  mcp-edge runtime                     │    │
│  │                                       │    │
│  │  ┌─────────────┐  ┌─────────────┐     │    │
│  │  │ Provider A  │  │ Provider B  │     │    │
│  │  │ (sensors)   │  │ (actuators) │     │    │
│  │  └──────┬──────┘  └──────┬──────┘     │    │
│  │         │                │            │    │
│  │         └────────┬───────┘            │    │
│  │                  │                    │    │
│  │         ┌────────▼────────┐           │    │
│  │         │  UnixTransport  │           │    │
│  │         └────────┬────────┘           │    │
│  └──────────────────│────────────────────┘    │
│                     │                         │
└─────────────────────│─────────────────────────┘
                      │
                      ▼
                    Agent
```

## Deployment Patterns

### Leaf

A single device exposing its own capabilities. An agent connects directly.

*Example: a Raspberry Pi with sensors in a greenhouse — agents read temperature and humidity over its socket.*

### Gateway

A device that aggregates multiple leaves behind one socket. Routes each `tools/call` to the owning leaf and proxies the response back.

```rust
use mcp_edge::transport::UnixTransport;
use mcp_edge::Gateway;

let mut gw: Gateway<2, 16> = Gateway::new();
gw.add_leaf("/tmp/leaf1.sock").unwrap();
gw.add_leaf("/tmp/leaf2.sock").unwrap();
UnixTransport::new("/tmp/gateway.sock").serve(|msg, out| gw.handle(msg, out));
```

Run the demo (three terminals):

```bash
TOOL=temp     SOCK=/tmp/leaf1.sock cargo run --example sensor
TOOL=humidity SOCK=/tmp/leaf2.sock cargo run --example sensor
cargo run --example gateway --features gateway
```

The gateway opens a fresh connection per call (stateless). The MCP `initialize` handshake runs once per leaf at startup, never per call — so each `tools/call` is exactly one round trip.

## Resource Philosophy

Edge devices have constraints. mcp-edge respects them.

- **Zero heap.** No `Vec`, `String`, `Box`, or `Arc`. All storage is in fixed arrays sized by const generics.
- **You declare the limits.** Provider count `N`, tool-result size `OUT`, leaf count `L`, route table size `T`, gateway tools-list buffer `B` — all compile-time. Exceeding them returns an error at startup, not a silent truncation at runtime.
- **Stack budgets.** A default `Runtime<'_, 8, 512>` peaks around `OUT` bytes of stack on the `tools/call` path and ~80 bytes on every other path. A default `Gateway<4, 32, 2048>` is ~3.6 KB resident with a ~1.3 KB per-call stack peak.
- **`no_std` compatible.** The core runtime builds without `std`. Disable default features for `no_std` targets:
  ```toml
  mcp-edge = { version = "0.1", default-features = false }
  ```
