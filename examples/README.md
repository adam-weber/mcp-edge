# mcp-edge examples

Five runnable examples, each covering one usage shape, plus a benchmark.

> **First-time question:** *"I already have a sensor outputting humidity (or whatever). Does mcp-edge wrap it, or do I rewrite my sensor?"*
>
> You write a small Rust wrapper — typically 5–10 lines per tool — that reads from wherever your existing source lives (a sysfs file, a CLI tool, a daemon socket, an HTTP endpoint, an MQTT topic, a hardware register) and writes the value into `out`. mcp-edge is the protocol surface, not the sensor; your driver code stays where it is. See [`wrap.rs`](wrap.rs) for two concrete patterns.

## wrap — wrapping an existing source

Two backing patterns side-by-side: reading a sysfs file (Linux) and shelling out to an existing CLI tool (`hostname`, portable). The body of each `Provider::call` is ~5 lines — use it as a template for your own source.

```bash
cargo run --example wrap

# in another terminal:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/mcp-edge.sock

# call the portable one (works everywhere):
echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"hostname","arguments":{}}}' \
  | nc -U /tmp/mcp-edge.sock
```

The `cpu_temp` tool reads from `/sys/class/thermal/thermal_zone0/temp`, which is Linux-only. On macOS or Windows it will return `"sysfs read failed"` — that's expected; the pattern is identical for any file-based sensor on Linux/embedded.

## sensor — Unix socket leaf

A device exposing tools over a local Unix socket with stub values. Good for scripted testing with `nc -U`, or as a leaf process behind a `Gateway`.

```bash
cargo run --example sensor

# in another terminal:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/mcp-edge.sock
```

The same example, configured per process via env vars, supplies two distinctly-tooled leaves to a gateway:

```bash
TOOL=temp     SOCK=/tmp/leaf1.sock cargo run --example sensor
TOOL=humidity SOCK=/tmp/leaf2.sock cargo run --example sensor
```

## stdio — subprocess MCP server

The shape MCP hosts use for local servers — they spawn the binary and pipe JSON-RPC over stdin/stdout. No socket, no port. This is what you build when you want an agent to be able to call your tools.

### Build and verify

```bash
cargo build --release --example stdio

# Confirm it speaks the protocol with no host involved:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | ./target/release/examples/stdio
# → {"jsonrpc":"2.0","id":1,"result":{"tools":[...]}}

echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"temp_read","arguments":{}}}' \
  | ./target/release/examples/stdio
# → {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"22.5"}]}}
```

### Register with an MCP host

Most MCP hosts (whatever your agent stack uses — desktop apps, IDE extensions, custom SLM frontends, etc.) accept an `mcpServers` JSON config block:

```json
{
  "mcpServers": {
    "mcp-edge-demo": {
      "command": "/abs/path/to/target/release/examples/stdio"
    }
  }
}
```

Substitute the absolute path to your built binary. Drop the block into your host's MCP config file, restart the host, and ask your agent *"what's the temperature?"* — it discovers `temp_read`, calls it, and answers with the result.

CLI-based hosts typically expose an equivalent `add` command that takes `--transport stdio` and the binary path; check your host's docs for the exact syntax.

## gateway — multi-leaf aggregator

Aggregates multiple leaves behind one socket. Requires the `gateway` feature.

```bash
# After two `sensor` instances are running on /tmp/leaf1.sock and /tmp/leaf2.sock
# (see the env-var trick under `sensor` above):
cargo run --example gateway --features gateway

# in another terminal:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/gateway.sock
```

Three seconds after start it also tries `/tmp/leaf3.sock`; bring up `TOOL=pressure SOCK=/tmp/leaf3.sock cargo run --example sensor` beforehand and every connected client receives a `notifications/tools/list_changed` push. Override paths with the `LEAF1`, `LEAF2`, `LEAF3`, and `SOCK` env vars. See [`gateway.rs`](gateway.rs) for the source.

## client — the consuming side

Connects to any mcp-edge server or gateway, runs the handshake, lists what it found, and calls a tool. Dependency-free, and the shape to crib if you are writing your own client.

```bash
cargo run --example sensor &
cargo run --example client
# connecting to /tmp/mcp-edge.sock
# initialize -> {"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2024-11-05",...}}
# tools/list -> 2 tool(s): temp_read, humidity_read
# calling temp_read with {}
# tools/call -> {"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"22.5"}]}}
```

Arguments are `[address] [tool] [args-json]`, with `--watch` to hold the connection open and print `tools/list_changed` pushes from a `DynamicGateway`:

```bash
cargo run --example client -- /tmp/gateway.sock humidity_read '{}'
cargo run --example client -- tcp://10.0.0.5:9000
cargo run --example client -- /tmp/gateway.sock --watch
```

## bench — throughput and latency

Not a usage shape, a measurement harness: runtime dispatch, static vs dynamic gateway `handle`, full proxy round-trips over a real socket, mutation rate, and broadcast fan-out. Zero deps, hand-rolled percentiles.

```bash
cargo run --profile bench-fast --example bench --features gateway
```
