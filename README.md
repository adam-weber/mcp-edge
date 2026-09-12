# mcp-edge

A minimal Rust runtime for the [Model Context Protocol](https://modelcontextprotocol.io). All you implement is this:

```rust
pub trait Provider {
    fn tools(&self) -> &[Tool];
    fn call(&self, tool: &str, args: &[u8], out: &mut Output) -> Result<(), &'static str>;
}
```

Two functions: list your tools, run one. mcp-edge does the rest of the protocol.

`no_std`-compatible core, zero heap, 128 bytes of resident state for a default `Runtime`. Small enough to run on a $5 microcontroller, and it composes into a multi-device gateway (3.3 KB, still no allocator) when one chip isn't enough.

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

That's the whole program. Different transport, swap one line, same handler closure:

```rust
TcpTransport::new("0.0.0.0:9000").serve(|m, o| rt.handle(m, o));     // cross-machine
StdioTransport::serve(|m, o| rt.handle(m, o));                       // subprocess MCP server
```

Runnable scenarios, including how to register the stdio binary with an MCP host, are in [`examples/`](examples/).

## Try it

From a checkout of this repo:

```bash
git clone https://github.com/adam-weber/mcp-edge && cd mcp-edge
cargo run --example sensor &

echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/mcp-edge.sock

echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"temp_read","arguments":{}}}' \
  | nc -U /tmp/mcp-edge.sock
```

To use it in your own crate, `cargo add mcp-edge` and start from the server above.

## Connecting an agent

Everything above is the server side. Three ways to consume it.

### From an MCP host (Claude, ChatGPT, an IDE, your own)

Hosts spawn MCP servers as subprocesses and speak JSON-RPC over stdin/stdout. Build a stdio binary and register it:

```bash
cargo build --release --example stdio
```

```json
{
  "mcpServers": {
    "mcp-edge-demo": {
      "command": "/abs/path/to/mcp-edge/target/release/examples/stdio"
    }
  }
}
```

That block is the common denominator across hosts; `args` and `env` are accepted too, so one binary can serve several devices from different config entries. Drop it into your host's MCP config, restart the host, and ask *"what's the temperature?"*. The agent discovers `temp_read`, calls it, and answers from the result. Claude Desktop keeps that JSON in `claude_desktop_config.json`; Claude Code takes the same thing as `claude mcp add mcp-edge-demo -- /abs/path/to/stdio`; IDE extensions each have their own config path but the same shape.

### From a socket

Hosts speak stdio, not Unix sockets, so a server already listening on a socket needs a bridge. The wire format is newline-delimited JSON in both directions, which means the bridge is a byte pipe with nothing to translate:

```json
{
  "mcpServers": {
    "workshop": {
      "command": "socat",
      "args": ["STDIO", "UNIX-CONNECT:/run/aggregator.sock"]
    }
  }
}
```

Swap `UNIX-CONNECT:` for `TCP:host:port` to reach a `TcpTransport` server.

### From your own code

[`examples/client.rs`](examples/client.rs) is a dependency-free consumer: connect, `initialize`, `tools/list`, `tools/call`, and optionally hold the connection open printing `tools/list_changed` pushes.

```bash
cargo run --example sensor &
cargo run --example client
# connecting to /tmp/mcp-edge.sock
# initialize -> {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05",...}}
# tools/list -> 2 tool(s): temp_read, humidity_read
# calling temp_read with {}
# tools/call -> {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"22.5"}]}}
```

It takes an address, a tool name, and an arguments object, so it drives a gateway just as well: `cargo run --example client -- /tmp/gateway.sock humidity_read '{}'`.

## Aggregating multiple devices

One `Runtime` is one process. When you outgrow that, because you want fault isolation across systemd services, or you have multiple ECUs on a vehicle bus, or sensors scattered across a workshop, the same primitives compose into a gateway: leaf processes keep running unchanged, and the gateway presents their tools to agents as one flat namespace.

A gateway does three things. At startup it runs the MCP handshake against each leaf, reads its `tools/list`, and records which leaf owns which tool name. On `tools/list` it serves a JSON blob it built once at registration time, so the read path touches no leaf and no allocator. On `tools/call` it looks up the owning leaf, opens a fresh stateless connection, forwards the call verbatim, and splices the leaf's `result` (or `error`) into a reply carrying the client's own request id. Duplicate tool names across leaves are rejected at `add_leaf` time rather than resolved by some silent precedence rule.

### Two flavors

|                           | `Gateway`                     | `DynamicGateway`                          |
|---------------------------|-------------------------------|-------------------------------------------|
| Leaves added at runtime   | startup only (`&mut self`)    | any time, any thread (`&self`)            |
| Leaves removed at runtime  | not supported                | `remove_leaf(addr)`                       |
| `tools/list_changed` push | not sent                      | broadcast on every mutation               |
| Clients                   | one connection at a time      | thread per connection                     |
| Heap allocation           | none                          | `Vec`/`Arc`/`Box` for the subscriber list |
| Synchronization           | none                          | `RwLock` + `Mutex` + `AtomicU64`          |
| Typical use               | embedded device, fixed leaves | Linux gateway, dynamic discovery          |

Both speak the same wire protocol. Pick `Gateway` unless you need runtime mutation.

### Static: `Gateway`

Leaves are registered before serving starts. After that the borrow checker does the work a lock would otherwise do: `handle` takes `&self`, so nothing can mutate the routing table while the serve closure holds a borrow. No locks, no atomics, no allocator.

```rust
use mcp_edge::{Gateway, transport::UnixTransport};

let mut gw: Gateway<4, 16> = Gateway::new();
gw.add_leaf("/run/leaves/climate.sock").unwrap();    // bare path resolves to Unix
gw.add_leaf("unix:///run/leaves/power.sock").unwrap();
gw.add_leaf("tcp://10.0.0.5:9000").unwrap();         // remote, TCP
gw.add_leaf("udp://10.0.0.6:9000").unwrap();         // requires `udp` feature

UnixTransport::new("/run/aggregator.sock").serve(|m, o| gw.handle(m, o));
```

The default `MultiConnector` dispatches each leaf on its URL scheme, so one gateway can mix transports. `tls://` is parsed and reserved but not yet wired to rustls; `add_leaf` returns `Err` for it today. If every leaf shares one transport you can name the connector and drop the dispatch enum entirely: `Gateway<4, 16, 2048, UnixConnector>`.

### Dynamic: `DynamicGateway`

Same routing, but leaves come and go while clients are connected, and every connected client is told when they do. `add_leaf` and `remove_leaf` take `&self` and are callable from any thread: a discovery loop, a hotplug watcher, a control socket. Each mutation bumps a monotonic `epoch()` and broadcasts `notifications/tools/list_changed` to every live subscriber.

```rust
use std::sync::Arc;
use mcp_edge::DynamicGateway;

let gw: Arc<DynamicGateway<8, 64>> = Arc::new(DynamicGateway::new());
gw.add_leaf("/run/leaves/climate.sock").unwrap();

// A discovery thread mutating the live gateway.
{
    let gw = gw.clone();
    std::thread::spawn(move || {
        gw.add_leaf("tcp://10.0.0.9:9000").unwrap();       // clients get list_changed
        gw.remove_leaf("/run/leaves/climate.sock").unwrap(); // and again here
    });
}

gw.serve_unix("/run/aggregator.sock");   // thread per client; serve_tcp is the TCP twin
```

`serve_unix` and `serve_tcp` accept and spawn for you. `serve_conn` takes a connection you accepted yourself, so you can do your own listener setup, TLS termination, or accept policy and still get subscriber registration and framing.

Some details that matter once leaves are mutable:

- Leaf discovery runs outside the lock. Connecting to a wedged leaf can take seconds; it does not block in-flight tool calls.
- `tools/call` copies the leaf address out under a read lock and releases it before proxying, so a slow leaf never blocks a mutator.
- Every write to a client goes through one per-connection mutex, so a broadcast cannot splice its bytes into the middle of a response frame.
- A subscriber whose write times out (2s) is marked dead and skipped, never stalling the thread doing the mutation.
- Connection threads spawn with a 16 KiB stack instead of the 2 MiB default, so 100 concurrent clients cost 1.6 MiB of address space.
- Removing a leaf reclaims its bytes in the name arena, so long-running add/remove churn doesn't drift into the `A` cap.

### Putting a gateway behind an agent

An agent reaches a gateway exactly the way it reaches a single device. Serve the gateway over stdio and the host sees one MCP server whose tool list happens to span every leaf:

```rust
use mcp_edge::{Gateway, transport::StdioTransport};

fn main() {
    let mut gw: Gateway<4, 32> = Gateway::new();
    gw.add_leaf("/run/leaves/climate.sock").unwrap();
    gw.add_leaf("tcp://10.0.0.5:9000").unwrap();
    StdioTransport::serve(|m, o| gw.handle(m, o));
}
```

```json
{
  "mcpServers": {
    "workshop": { "command": "/abs/path/to/your-gateway-binary" }
  }
}
```

One caveat: stdio is a single pipe pair with no subscriber registration, so a `DynamicGateway` served that way still routes and still mutates, but its `tools/list_changed` pushes reach nobody. If you want the host to hear about leaves appearing and disappearing, run `serve_unix` and point the host at it through the socat bridge above.

### Sizing a gateway

```rust
Gateway<L, T, B, C, A>
```

- `L`: max leaves (default 4)
- `T`: max tools across all leaves (default 32)
- `B`: byte budget for the cached tools-list JSON (default 2048)
- `C`: connector (default `MultiConnector`; all built-in connectors are zero-sized)
- `A`: byte budget for the packed arena holding leaf addresses, tool names, and tool descriptions (default 1024)

`DynamicGateway` takes the same five. Exceeding `L`, `T`, `B`, or `A` makes `add_leaf` return `Err` at startup instead of truncating at runtime.

[`examples/gateway.rs`](examples/gateway.rs) is a runnable demo: two leaves at startup, a third added three seconds in, with the push arriving on an already-connected client. [`tests/dynamic.rs`](tests/dynamic.rs) is the same flow as an end-to-end assertion.

## Running on a microcontroller

The core is `no_std` with no allocator, so it builds for bare metal as-is. Verified building clean (warning-free) for:

| Target | Boards |
|---|---|
| `thumbv7em-none-eabihf` | Arduino Uno R4 (RA4M1), Nano 33 BLE (nRF52840) |
| `thumbv6m-none-eabi` | Arduino Nano RP2040 Connect |
| `riscv32imc-unknown-none-elf` | ESP32-C3 class |

Measured on Cortex-M4, for a one-tool server including framing: **3,862 bytes of flash**, 4 bytes of RAM per provider slot, and whatever you size the buffers to. That leaves room on parts where the vendor SDK alone would not fit.

### Framing without an OS

There is no `std::io` on these targets. Bare-metal triples ship `core` and `alloc` only, so there is no `Read`/`Write` to implement against and no sockets to accept. `Framer` inverts the relationship instead: you own the peripheral, and push bytes in as they arrive.

```rust
use mcp_edge::{Framer, Runtime};

let mut rt: Runtime<'_, 2, 64> = Runtime::new();
rt.register(&sensor).unwrap();

// RX caps one inbound message, TX one outbound response.
let mut framer: Framer<256, 256> = Framer::new();

loop {
    let byte = uart.read_byte();
    framer.feed(
        &[byte],
        |msg, out| rt.handle(msg, out),
        |resp| uart.write_all(resp),
    ).ok();
}
```

`feed` takes any chunk size, so a UART interrupt handing over one byte and a USB-CDC endpoint handing over 64 both work. Where you'd rather not pay for the copy, `framer.rx_space()` hands you the unfilled tail of the buffer to read or DMA into directly, and `framer.commit(n, ..)` dispatches what that completed. The `std` transports in this crate are thin adapters over exactly that pair, so socket and UART paths share one framing implementation rather than two.

A message longer than `RX` is dropped and the framer resynchronizes on the next newline, so a truncated frame is never parsed as a fresh request. It reports `FrameError::Overflow` once, not on every subsequent byte.

### Sizing it for the part

Everything is a const generic, so RAM is a number you choose rather than one you discover:

- `Runtime<'_, N, OUT>`: `N` pointer-sized slots resident, `OUT` bytes of stack on the `tools/call` path only.
- `Framer<RX, TX>`: exactly `RX + TX` bytes plus two words.

A 64 KB-SRAM part runs the defaults comfortably. On something tighter, `Runtime<'_, 2, 64>` plus `Framer<256, 256>` is 584 bytes of buffers total.

### What still needs writing

The crate gives you framing and protocol, not board support. Wiring `uart.read_byte()` to your HAL is yours, and it is the loop above rather than anything larger.

Two caveats worth knowing before you pick a board. Classic 8-bit AVR (Uno, Nano, ATmega328P) is a poor fit: Rust's AVR target is nightly-only tier 3 with known codegen bugs, and 2 KB of SRAM is tight once buffers are accounted for. And `core::fmt`'s float formatting is expensive on small parts, so prefer integer math in `Provider::call` (`write!(out, "{}.{}", c / 100, c % 100 / 10)`) over `{:.1}` on an `f32`.

## Architecture

Three trait-shaped boundaries; nothing else is load-bearing.

- **`Provider`**: what the device exposes. Backed by whatever you can reach, including GPIO, I2C, CAN, LIN, UDS, and software state.
- **`Transport`**: how agents reach you. `UnixTransport`, `TcpTransport`, `StdioTransport`, and `UdpTransport` under `std`; on bare metal, `Framer` gives you the same framing with the peripheral left in your hands. TLS layers on as a feature.
- **`Connector`**: how a gateway reaches each leaf. `UnixConnector`, `TcpConnector`, `UdpConnector`, and the URL-scheme-dispatching `MultiConnector` today. Future: `TlsConnector`, SOME/IP-SD for automotive zonal controllers.

Default connectors and transports are zero-sized. Heavyweight integrations live behind feature flags so they cost nothing if you don't opt in.

## Resource budgets (defaults)

- `Runtime<'_, 8, 512>`: 128 bytes resident on 64-bit, 64 on 32-bit (one fat pointer per provider slot). Peak stack is `OUT` bytes (512) on the `tools/call` path, roughly 80 bytes elsewhere.
- `Gateway<4, 32, 2048>` with `A = 1024`: 3,350 bytes resident. Fixed arrays only, so the number is identical on 32- and 64-bit.
- `DynamicGateway<4, 32, 2048>`: 3,408 bytes on 64-bit, plus heap for the subscriber list (one `Arc` + one `Box<dyn Write>` per connected client).
- Per-call gateway stack: a 1 KiB buffer in `proxy_call`, shared by the outbound request and the inbound reply. `add_leaf` uses a separate 4 KiB discovery buffer, and `remove_leaf` puts `A` bytes on the mutator's stack while compacting. Neither lands on a connection thread.

## Where it doesn't fit

- **Cloud / production MCP servers**: use [`rmcp`](https://github.com/modelcontextprotocol/rust-sdk), the official MCP Rust SDK. Async, full feature surface, well-maintained.
- **Enterprise gateways** (auth, observability, Docker, K8s): use MetaMCP, Kong AI Gateway, IBM ContextForge. mcp-edge is a library, not an orchestrator.
- **On-device LLM inference**: mcp-edge sits *below* the model. The smallest useful SLMs need ~600 MB RAM (Pi 5 territory). On the MCU tier the LLM lives elsewhere, and mcp-edge exposes tools to whatever calls in.

## Features

- `std` (default): `UnixTransport`, `TcpTransport`, `StdioTransport`. Without it you still get `Runtime`, `Provider`, and `Framer`, which is the whole bare-metal surface.
- `gateway`: `Gateway` and `DynamicGateway` for aggregating leaves (requires `std`)
- `udp`: `UdpTransport` and `UdpConnector` (datagram framing, one request to one reply)
- `tls`: reserves the `tls://` URL scheme in `MultiConnector` (rustls integration lands in a follow-up)

For `no_std` builds:

```toml
mcp-edge = { version = "0.1", default-features = false }
```
