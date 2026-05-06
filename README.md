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

Try the gateway aggregating two single-tool leaves:

```bash
# Terminals 1 & 2: two leaves, each exposing one tool
TOOL=temp     SOCK=/tmp/leaf1.sock cargo run --example sensor
TOOL=humidity SOCK=/tmp/leaf2.sock cargo run --example sensor

# Terminal 3: gateway
cargo run --example gateway --features gateway

# Terminal 4: list both tools through the gateway
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/gateway.sock
```

Any MCP-compatible agent can now discover and use these tools.

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

**Example: greenhouse monitor on a Raspberry Pi.** A Pi reads a DHT22 (temperature + humidity) and a capacitive soil-moisture probe. An agent on a phone or laptop reaches it over Tailscale and answers questions like *"should I water the tomatoes today?"* without you writing any prompt-handling code.

```rust
use core::fmt::Write;
use mcp_edge::transport::UnixTransport;
use mcp_edge::{Output, Provider, Runtime, Tool};

struct Climate;
impl Provider for Climate {
    fn tools(&self) -> &[Tool] {
        &[
            Tool { name: "temp_c",   description: "Air temperature, °C" },
            Tool { name: "humidity", description: "Relative humidity, %" },
        ]
    }
    fn call(&self, tool: &str, _args: &[u8], out: &mut Output) -> Result<(), &'static str> {
        let (t, h) = read_dht22().map_err(|_| "sensor read failed")?;
        match tool {
            "temp_c"   => write!(out, "{:.1}", t).unwrap(),
            "humidity" => write!(out, "{:.0}", h).unwrap(),
            _ => return Err("unknown tool"),
        }
        Ok(())
    }
}

struct Soil;
impl Provider for Soil {
    fn tools(&self) -> &[Tool] {
        &[Tool { name: "soil_moisture", description: "Soil moisture, % (0=dry, 100=wet)" }]
    }
    fn call(&self, _tool: &str, _args: &[u8], out: &mut Output) -> Result<(), &'static str> {
        let m = read_moisture_adc().map_err(|_| "ADC read failed")?;
        write!(out, "{:.0}", m).unwrap();
        Ok(())
    }
}

fn main() {
    let climate = Climate;
    let soil = Soil;
    let mut rt: Runtime<'_, 2> = Runtime::new();
    rt.register(&climate).unwrap();
    rt.register(&soil).unwrap();
    UnixTransport::new("/run/greenhouse.sock").serve(|m, o| rt.handle(m, o));
}
```

`read_dht22` and `read_moisture_adc` are your hardware drivers — mcp-edge ships only the MCP protocol surface, not sensor code.

### Gateway

A device that aggregates multiple leaves behind one socket. Routes each `tools/call` to the owning leaf and proxies the response back.

**Example: workshop Pi with fault-isolated providers.** Running every provider in one process means a glitch in one (e.g., a relay driver that hangs on a serial timeout) takes down sensor reads. Splitting concerns into separate processes — each owning its own bus or actuator on its own Unix socket — contains failures, and systemd restarts each independently. A gateway recombines them so agents see one device.

A workshop Pi runs four systemd services:

| Service                    | Socket                     | Provides                            |
|----------------------------|----------------------------|-------------------------------------|
| `workshop-climate.service` | `/run/leaves/climate.sock` | `temp_c`, `humidity`, `dust_ppm`    |
| `workshop-power.service`   | `/run/leaves/power.sock`   | `mains_w`, `solar_w`, `battery_pct` |
| `workshop-cnc.service`     | `/run/leaves/cnc.sock`     | `cnc_state`, `spindle_rpm`          |
| `workshop-gateway.service` | `/run/workshop.sock`       | (aggregates the three above)        |

The gateway:

```rust
use mcp_edge::transport::UnixTransport;
use mcp_edge::Gateway;

fn main() {
    let mut gw: Gateway<3, 16> = Gateway::new();
    gw.add_leaf("/run/leaves/climate.sock").expect("climate leaf unavailable");
    gw.add_leaf("/run/leaves/power.sock").expect("power leaf unavailable");
    gw.add_leaf("/run/leaves/cnc.sock").expect("cnc leaf unavailable");
    UnixTransport::new("/run/workshop.sock").serve(|m, o| gw.handle(m, o));
}
```

An agent connects only to `/run/workshop.sock` and sees eight tools as one flat namespace. Asked *"is the CNC running and how much solar power are we generating?"* it calls `cnc_state` and `solar_w` — the gateway routes each to the right leaf and proxies the answer back. Agents never see the topology.

If the CNC service crashes, calls to its tools return `{"error":{"code":-1,"message":"leaf connect failed"}}` and the other tools keep working. Tool discovery happens once at gateway startup, so when systemd restarts the CNC service, restart the gateway too if the tool list changed.

The gateway opens a fresh connection per call (stateless). The MCP `initialize` handshake runs once per leaf at startup, never per call — every `tools/call` is exactly one round trip.

## Resource Philosophy

Edge devices have constraints. mcp-edge respects them.

- **Zero heap.** No `Vec`, `String`, `Box`, or `Arc`. All storage is in fixed arrays sized by const generics.
- **You declare the limits.** Provider count `N`, tool-result size `OUT`, leaf count `L`, route table size `T`, gateway tools-list buffer `B` — all compile-time. Exceeding them returns an error at startup, not a silent truncation at runtime.
- **Stack budgets.** A default `Runtime<'_, 8, 512>` peaks around `OUT` bytes of stack on the `tools/call` path and ~80 bytes on every other path. A default `Gateway<4, 32, 2048>` is ~3.6 KB resident with a ~1.3 KB per-call stack peak.
- **`no_std` compatible.** The core runtime builds without `std`. Disable default features for `no_std` targets:
  ```toml
  mcp-edge = { version = "0.1", default-features = false }
  ```
