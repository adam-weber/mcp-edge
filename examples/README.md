# mcp-edge examples

Three runnable examples, each covering one usage shape.

## sensor — Unix socket leaf

A device exposing tools over a local Unix socket. Good for local development, scripted testing with `nc -U`, or as a leaf process behind a `Gateway`.

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

## gateway — multi-leaf aggregator

Aggregates two leaves behind one socket. Requires the `gateway` feature.

```bash
# After two `sensor` instances are running on /tmp/leaf1.sock and /tmp/leaf2.sock:
cargo run --example gateway --features gateway

# in another terminal:
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' \
  | nc -U /tmp/gateway.sock
```

Override leaf paths with `LEAF1`, `LEAF2` env vars. See [`gateway.rs`](gateway.rs) for the source.

## stdio — subprocess MCP server

The shape MCP hosts use for local servers — they spawn the binary and pipe JSON-RPC over stdin/stdout. No socket, no port.

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

Drop that block into your host's MCP config file, restart the host, and ask your agent *"what's the temperature?"* — it discovers `temp_read`, calls it, and answers with the result.

CLI-based hosts typically expose an equivalent `add` command that takes `--transport stdio` and the binary path; check your host's docs for the exact syntax.
