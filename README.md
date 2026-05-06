# mcp-edge

A minimal Rust runtime for the [Model Context Protocol](https://modelcontextprotocol.io). All you implement is this:

```rust
pub trait Provider {
    fn tools(&self) -> &[Tool];
    fn call(&self, tool: &str, args: &[u8], out: &mut Output) -> Result<(), &'static str>;
}
```

Two functions — list your tools, run one. mcp-edge does the rest of the protocol.

`no_std`-compatible core, zero heap, ~3.6 KB of resident state at default sizes. Small enough to run on a $5 microcontroller; composes into a multi-device gateway when one chip isn't enough.

## A complete server

```rust
use mcp_edge::{Output, Provider, Runtime, Tool, transport::UnixTransport};
use core::fmt::Write;

struct TempSensor;
impl Provider for TempSensor {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "temp_read", description: "Read temperature in Celsius" }]
    }
    fn call(&self, _: &str, _: &[u8], out: &mut Output) -> Result<(), &'static str> {
        write!(out, "{:.1}", read_sensor()).unwrap();
        Ok(())
    }
}

fn main() {
    let sensor = TempSensor;
    let mut rt: Runtime<'_, 1> = Runtime::new();
    rt.register(&sensor).unwrap();
    UnixTransport::new("/tmp/mcp-edge.sock").serve(|m, o| rt.handle(m, o));
}
```

That's the whole program. Cross-machine? Swap one line:

```rust
TcpTransport::new("0.0.0.0:9000").serve(|m, o| rt.handle(m, o));
```

## Try it

```bash
cargo add mcp-edge
cargo run --example sensor &

echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/mcp-edge.sock

echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"temp_read","arguments":{}}}' \
  | nc -U /tmp/mcp-edge.sock
```

## Aggregating multiple devices

When you outgrow one process — fault isolation across systemd services, multiple ECUs on a vehicle bus, sensors scattered across a workshop — the same primitives compose into a `Gateway`. The default `MultiConnector` dispatches each leaf by URL scheme, so one gateway can mix Unix, TCP, and (planned) TLS:

```rust
let mut gw: Gateway<3, 16> = Gateway::new();
gw.add_leaf("/run/leaves/climate.sock").unwrap();    // bare path → Unix
gw.add_leaf("tcp://10.0.0.5:9000").unwrap();         // remote, TCP
gw.add_leaf("tls://factory.local:9001").unwrap();    // requires `tls` feature
UnixTransport::new("/run/aggregator.sock").serve(|m, o| gw.handle(m, o));
```

Agents see one flat namespace. The gateway routes per tool name, opens a fresh stateless connection per call, and proxies the response back. See [`examples/gateway.rs`](examples/gateway.rs) for a runnable demo.

## Architecture

Three trait-shaped boundaries; nothing else is load-bearing.

- **`Provider`** — what the device exposes. Backed by whatever you can reach: GPIO, I2C, CAN, LIN, UDS, software state.
- **`Transport`** — how agents reach you. `UnixTransport`, `TcpTransport` today. TLS and `embedded-nal` (for bare-metal MCUs) layer on as features or sibling crates.
- **`Connector`** — how a `Gateway` reaches each leaf. `UnixConnector`, `TcpConnector`, URL-scheme-dispatching `MultiConnector` today. Future: `TlsConnector`, SOME/IP-SD for automotive zonal controllers.

Default connectors and transports are zero-sized — heavyweight integrations live behind feature flags so they cost nothing if you don't opt in.

## Resource budgets (defaults)

- `Runtime<'_, 8, 512>`: ~136 bytes resident; peak `OUT` bytes (= 512) of stack on `tools/call`, ~80 bytes elsewhere.
- `Gateway<4, 32, 2048>`: ~3.6 KB resident; 1.3 KB per-call stack peak.
- All sizes are const-generic. Exceeding them errors at startup — never a silent truncation at runtime.

## Where it doesn't fit

- **Cloud / production MCP servers** — use [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk) (official Anthropic SDK). Async, full feature surface, well-maintained.
- **Enterprise gateways** (auth, observability, Docker, K8s) — use MetaMCP, Kong AI Gateway, IBM ContextForge. mcp-edge is a library, not an orchestrator.
- **On-device LLM inference** — mcp-edge sits *below* the model. The smallest useful SLMs need ~600 MB RAM (Pi 5 territory). On the MCU tier, the LLM lives elsewhere; mcp-edge exposes tools to whatever calls in.

## Features

- `std` (default) — `UnixTransport` and `TcpTransport`
- `gateway` — `Gateway` for aggregating leaves (requires `std`)
- `tls` — reserves the `tls://` URL scheme in `MultiConnector` (rustls integration lands in a follow-up)

For `no_std` builds:

```toml
mcp-edge = { version = "0.1", default-features = false }
```
