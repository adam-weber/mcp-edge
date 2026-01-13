# mcp-edge

A minimal MCP runtime for resource-constrained devices.

## Getting Started

### Install

```bash
cargo add mcp-edge
```

### Minimal Example

```rust
use mcp_edge::{Runtime, Provider, Tool, ToolResult, UnixTransport};
use std::collections::HashMap;

// Define a provider for your hardware
struct TemperatureSensor;

impl Provider for TemperatureSensor {
    fn name(&self) -> &str { 
        "temperature" 
    }

    fn tools(&self) -> Vec {
        vec![Tool {
            name: "read_temperature".into(),
            description: "Read temperature in Celsius".into(),
        }]
    }

    fn call(&self, tool: &str, _params: &HashMap) -> ToolResult {
        match tool {
            "read_temperature" => {
                // Your actual sensor reading logic here
                let temp = 22.5; 
                ToolResult::Ok(format!("{:.1}", temp))
            }
            _ => ToolResult::Err("unknown tool".into())
        }
    }
}

fn main() {
    let mut runtime = Runtime::new();
    runtime.register(TemperatureSensor);
    runtime.serve(UnixTransport::new("/tmp/mcp-edge.sock"));
}
```

### Test It

```bash
# Terminal 1: Run the server
cargo run

# Terminal 2: Send MCP messages
echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | nc -U /tmp/mcp-edge.sock
# Returns: {"jsonrpc":"2.0","id":1,"result":{"tools":[{"name":"read_temperature",...}]}}

echo '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_temperature"}}' | nc -U /tmp/mcp-edge.sock
# Returns: {"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"22.5"}]}}
```

Any MCP-compatible agent can now discover and use your sensor.


## How It Works

mcp-edge is a library, not a framework. You build a small program that:

1. Starts the runtime
2. Registers providers (things that expose tools)
3. Listens on a transport (how agents connect)

The runtime handles MCP protocol details, tool discovery, and message routing.

```
┌─────────────────────────────────────────────────────────┐
│  Your device                                            │
│                                                         │
│  ┌────────────────────────────────────────────────────┐ │
│  │  mcp-edge runtime                                  │ │
│  │                                                    │ │
│  │  ┌─────────────┐  ┌─────────────┐                  │ │
│  │  │ Provider A  │  │ Provider B  │  (your code)     │ │
│  │  │ (sensors)   │  │ (actuators) │                  │ │
│  │  └─────────────┘  └─────────────┘                  │ │
│  │         │                │                         │ │
│  │         └───────┬────────┘                         │ │
│  │                 │                                  │ │
│  │         ┌───────▼────────┐                         │ │
│  │         │   Transport    │  (how agents connect)   │ │
│  │         │  (TCP, MQTT,   │                         │ │
│  │         │   Unix, etc)   │                         │ │
│  │         └───────┬────────┘                         │ │
│  └─────────────────│──────────────────────────────────┘ │
│                    │                                    │
└────────────────────│────────────────────────────────────┘
                     │
                     ▼
                  Agent
```

An agent connects, asks "what tools do you have?", gets a list, and starts using them.

## Deployment Patterns

### Leaf

A single device exposing its own capabilities.

*Example: A Raspberry Pi with sensors in a greenhouse. An agent connects directly and reads temperature/humidity.*

### Gateway

A device that aggregates multiple leaves. Provides a single point of contact for agents, routes requests to the right leaf.

*Example: A home server that knows about all the smart devices on your network. An agent asks the gateway, gateway routes to the right device.*

### Standalone

Like a leaf, but with optional local inference. Can handle queries even when disconnected from the cloud.

*Example: An industrial gateway that can classify simple commands locally, only reaching out to the cloud for complex reasoning.*

```
Leaf           Gateway                    Standalone
────           ───────                    ──────────

┌─────┐        ┌─────────┐               ┌───────────┐
│ mcp │        │   mcp   │               │    mcp    │
│edge │        │  edge   │               │   edge    │
└──┬──┘        └────┬────┘               │           │
   │                │                    │ + optional│
   │           ┌────┼────┐               │ inference │
   ▼           ▼    ▼    ▼               └─────┬─────┘
 Agent       leaf leaf leaf                    │
                                               ▼
                                             Agent
```

---

## Local Inference (Optional)

Most of the time, queries go to a cloud-based agent (Claude, GPT, etc). The edge device just provides tools.

But sometimes you want local intelligence:
- Network is down
- Latency matters
- Privacy requires it

mcp-edge supports three tiers:

| Tier | What It Does | Memory Cost |
|------|--------------|-------------|
| **Pass-through** | All queries go to cloud | None |
| **Router** | Classifies intent locally, picks tools | ~10MB |
| **SLM** | Full local reasoning (small model) | ~1-2GB |

The tier is a runtime choice, not a build choice. Same binary can operate differently based on connectivity and configuration.

## Resource Philosophy

Edge devices have constraints. mcp-edge respects them.

**Budgets**: You declare limits (memory, connections, tools). The runtime enforces them.

**Streaming**: Messages are parsed on the wire, not loaded into memory. A device with 16MB of RAM can handle large tool responses.

**Lazy loading**: Providers initialize when first used, not at startup. Inference models load on demand, unload when idle. Memory is borrowed, not owned.

**Graceful degradation**: If a provider is unavailable, the tool disappears from the list, but the runtime keeps running. One broken sensor doesn't crash the system.
