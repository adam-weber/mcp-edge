// mcp-edge: minimal implementation
// Single file, minimal dependencies, demonstrates core concepts

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::{Arc, Mutex};

// ============================================================================
// Core Types
// ============================================================================

/// JSON-RPC request (simplified)
#[derive(Debug)]
struct Request {
    id: u64,
    method: String,
    params: HashMap<String, String>,
}

/// JSON-RPC response (simplified)
#[derive(Debug)]
struct Response {
    id: u64,
    result: Option<String>,
    error: Option<String>,
}

/// Tool definition exposed to agents
#[derive(Debug, Clone)]
pub struct Tool {
    pub name: String,
    pub description: String,
}

/// Result of calling a tool
pub enum ToolResult {
    Ok(String),
    Err(String),
}

// ============================================================================
// Provider Trait
// ============================================================================

/// A provider exposes tools to the runtime.
/// Implement this for your sensors, actuators, APIs, etc.
pub trait Provider: Send + Sync {
    /// Unique name for this provider
    fn name(&self) -> &str;

    /// List of tools this provider exposes
    fn tools(&self) -> Vec<Tool>;

    /// Handle a tool call
    fn call(&self, tool: &str, params: &HashMap<String, String>) -> ToolResult;
}

// ============================================================================
// Transport Trait
// ============================================================================

/// A transport handles communication with agents.
/// Implement this for Unix sockets, TCP, MQTT, etc.
pub trait Transport {
    /// Start listening and call handler for each connection
    fn serve<F>(&self, handler: F)
    where
        F: Fn(&str) -> String + Send + Sync + Clone + 'static;
}

// ============================================================================
// Runtime
// ============================================================================

/// The core runtime. Registers providers, routes tool calls.
pub struct Runtime {
    providers: Vec<Arc<dyn Provider>>,
    tool_map: HashMap<String, usize>, // tool name -> provider index
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
            tool_map: HashMap::new(),
        }
    }

    /// Register a provider and its tools
    pub fn register<P: Provider + 'static>(&mut self, provider: P) {
        let idx = self.providers.len();
        for tool in provider.tools() {
            self.tool_map.insert(tool.name.clone(), idx);
        }
        self.providers.push(Arc::new(provider));
    }

    /// Get all available tools
    pub fn list_tools(&self) -> Vec<Tool> {
        self.providers.iter().flat_map(|p| p.tools()).collect()
    }

    /// Call a tool by name
    pub fn call_tool(&self, name: &str, params: &HashMap<String, String>) -> ToolResult {
        match self.tool_map.get(name) {
            Some(&idx) => self.providers[idx].call(name, params),
            None => ToolResult::Err(format!("unknown tool: {}", name)),
        }
    }

    /// Handle a raw JSON-RPC message
    pub fn handle_message(&self, msg: &str) -> String {
        let req = match parse_request(msg) {
            Some(r) => r,
            None => return error_response(0, "parse error"),
        };

        match req.method.as_str() {
            "initialize" => {
                // MCP handshake
                success_response(req.id, r#"{"protocolVersion":"2024-11-05","capabilities":{"tools":{}}}"#)
            }
            "tools/list" => {
                // List available tools
                let tools: Vec<String> = self
                    .list_tools()
                    .iter()
                    .map(|t| format!(r#"{{"name":"{}","description":"{}"}}"#, t.name, t.description))
                    .collect();
                success_response(req.id, &format!(r#"{{"tools":[{}]}}"#, tools.join(",")))
            }
            "tools/call" => {
                // Call a tool
                let tool_name = req.params.get("name").map(|s| s.as_str()).unwrap_or("");
                match self.call_tool(tool_name, &req.params) {
                    ToolResult::Ok(result) => {
                        success_response(req.id, &format!(r#"{{"content":[{{"type":"text","text":"{}"}}]}}"#, escape_json(&result)))
                    }
                    ToolResult::Err(e) => error_response(req.id, &e),
                }
            }
            _ => error_response(req.id, "method not found"),
        }
    }

    /// Start serving on a transport
    pub fn serve<T: Transport>(self, transport: T) {
        let runtime = Arc::new(self);
        transport.serve(move |msg| runtime.handle_message(msg));
    }
}

// ============================================================================
// Unix Socket Transport
// ============================================================================

pub struct UnixTransport {
    path: String,
}

impl UnixTransport {
    pub fn new(path: &str) -> Self {
        // Clean up old socket
        let _ = std::fs::remove_file(path);
        Self { path: path.to_string() }
    }
}

impl Transport for UnixTransport {
    fn serve<F>(&self, handler: F)
    where
        F: Fn(&str) -> String + Send + Sync + Clone + 'static,
    {
        let listener = UnixListener::bind(&self.path).expect("failed to bind socket");
        println!("listening on {}", self.path);

        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let handler = handler.clone();
                    std::thread::spawn(move || handle_connection(stream, handler));
                }
                Err(e) => eprintln!("connection error: {}", e),
            }
        }
    }
}

fn handle_connection<F>(stream: UnixStream, handler: F)
where
    F: Fn(&str) -> String,
{
    let reader = BufReader::new(stream.try_clone().unwrap());
    let mut writer = stream;

    for line in reader.lines() {
        match line {
            Ok(msg) if !msg.is_empty() => {
                let response = handler(&msg);
                let _ = writeln!(writer, "{}", response);
            }
            Ok(_) => {}
            Err(_) => break,
        }
    }
}

// ============================================================================
// JSON Helpers (minimal, no serde)
// ============================================================================

fn parse_request(msg: &str) -> Option<Request> {
    // Extremely minimal JSON parsing - production would use serde
    let id = extract_number(msg, "\"id\":")?;
    let method = extract_string(msg, "\"method\":")?;
    
    let mut params = HashMap::new();
    if let Some(name) = extract_string(msg, "\"name\":") {
        params.insert("name".to_string(), name);
    }
    
    Some(Request { id, method, params })
}

fn extract_string(json: &str, key: &str) -> Option<String> {
    let start = json.find(key)? + key.len();
    let rest = &json[start..];
    let start = rest.find('"')? + 1;
    let rest = &rest[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn extract_number(json: &str, key: &str) -> Option<u64> {
    let start = json.find(key)? + key.len();
    let rest = json[start..].trim_start();
    let end = rest.find(|c: char| !c.is_numeric()).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn success_response(id: u64, result: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{},"result":{}}}"#, id, result)
}

fn error_response(id: u64, message: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":{},"error":{{"code":-1,"message":"{}"}}}}"#, id, escape_json(message))
}

fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

// ============================================================================
// Example: Mock Sensor Provider
// ============================================================================

pub struct MockSensor {
    name: String,
    value: Mutex<f32>,
}

impl MockSensor {
    pub fn new(name: &str, initial: f32) -> Self {
        Self {
            name: name.to_string(),
            value: Mutex::new(initial),
        }
    }
}

impl Provider for MockSensor {
    fn name(&self) -> &str {
        &self.name
    }

    fn tools(&self) -> Vec<Tool> {
        vec![
            Tool {
                name: format!("{}_read", self.name),
                description: format!("Read current value from {} sensor", self.name),
            },
            Tool {
                name: format!("{}_set", self.name),
                description: format!("Set value for {} sensor (mock)", self.name),
            },
        ]
    }

    fn call(&self, tool: &str, params: &HashMap<String, String>) -> ToolResult {
        if tool == format!("{}_read", self.name) {
            let val = self.value.lock().unwrap();
            ToolResult::Ok(format!("{:.1}", *val))
        } else if tool == format!("{}_set", self.name) {
            if let Some(v) = params.get("value").and_then(|s| s.parse::<f32>().ok()) {
                *self.value.lock().unwrap() = v;
                ToolResult::Ok("ok".to_string())
            } else {
                ToolResult::Err("missing or invalid 'value' param".to_string())
            }
        } else {
            ToolResult::Err("unknown tool".to_string())
        }
    }
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    println!("mcp-edge minimal runtime");
    println!("========================\n");

    // Create runtime
    let mut runtime = Runtime::new();

    // Register providers
    runtime.register(MockSensor::new("temperature", 22.5));
    runtime.register(MockSensor::new("humidity", 45.0));

    // Show registered tools
    println!("registered tools:");
    for tool in runtime.list_tools() {
        println!("  - {}: {}", tool.name, tool.description);
    }
    println!();

    // Start serving
    runtime.serve(UnixTransport::new("/tmp/mcp-edge.sock"));
}

// ============================================================================
// Test Client (run with: cargo test)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_runtime() {
        let mut runtime = Runtime::new();
        runtime.register(MockSensor::new("temp", 20.0));

        // List tools
        let tools = runtime.list_tools();
        assert_eq!(tools.len(), 2);

        // Call tool
        let result = runtime.call_tool("temp_read", &HashMap::new());
        match result {
            ToolResult::Ok(v) => assert_eq!(v, "20.0"),
            ToolResult::Err(e) => panic!("unexpected error: {}", e),
        }
    }

    #[test]
    fn test_mcp_messages() {
        let mut runtime = Runtime::new();
        runtime.register(MockSensor::new("temp", 25.0));

        // Initialize
        let resp = runtime.handle_message(r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
        assert!(resp.contains("protocolVersion"));

        // List tools
        let resp = runtime.handle_message(r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        assert!(resp.contains("temp_read"));

        // Call tool
        let resp = runtime.handle_message(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"temp_read"}}"#);
        assert!(resp.contains("25.0"));
    }
}
