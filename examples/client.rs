//! Example: the *consuming* side. Connects to an mcp-edge server or gateway,
//! runs the MCP handshake, lists the tools it found, and calls one.
//!
//! This is what an MCP host does for you when it spawns a stdio server. Here
//! it's spelled out so you can see the whole exchange, drive a socket-based
//! server (which hosts generally can't reach directly), or crib the framing
//! for your own client.
//!
//! Run a server first, then point this at it:
//!
//!   cargo run --example sensor &
//!   cargo run --example client
//!
//!   # explicit address; `tcp://host:port` also works
//!   cargo run --example client -- /tmp/gateway.sock
//!
//!   # call a specific tool with arguments
//!   cargo run --example client -- /tmp/mcp-edge.sock temp_read '{}'
//!
//!   # stay connected and print tools/list_changed pushes from a DynamicGateway
//!   cargo run --example client -- /tmp/gateway.sock --watch
//!
//! Framing is one JSON-RPC object per line, in both directions. That's the
//! whole wire protocol; `initialize` first, then anything else.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

/// Unix or TCP, chosen by the address prefix, same rule the gateway's
/// `MultiConnector` uses: `tcp://host:port` is TCP, anything else is a path.
enum Conn {
    Unix(UnixStream),
    Tcp(TcpStream),
}

impl Conn {
    fn connect(addr: &str) -> std::io::Result<Self> {
        match addr.strip_prefix("tcp://") {
            Some(hp) => Ok(Conn::Tcp(TcpStream::connect(hp)?)),
            None => {
                let path = addr.strip_prefix("unix://").unwrap_or(addr);
                Ok(Conn::Unix(UnixStream::connect(path)?))
            }
        }
    }

    fn try_clone(&self) -> std::io::Result<Self> {
        match self {
            Conn::Unix(s) => s.try_clone().map(Conn::Unix),
            Conn::Tcp(s) => s.try_clone().map(Conn::Tcp),
        }
    }

    fn set_read_timeout(&self, d: Option<Duration>) -> std::io::Result<()> {
        match self {
            Conn::Unix(s) => s.set_read_timeout(d),
            Conn::Tcp(s) => s.set_read_timeout(d),
        }
    }
}

impl Read for Conn {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Conn::Unix(s) => s.read(buf),
            Conn::Tcp(s) => s.read(buf),
        }
    }
}

impl Write for Conn {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Conn::Unix(s) => s.write(buf),
            Conn::Tcp(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Conn::Unix(s) => s.flush(),
            Conn::Tcp(s) => s.flush(),
        }
    }
}

/// Send one request line, then read lines until one carries a `result` or an
/// `error`. Notifications (`tools/list_changed` and friends) can arrive at any
/// time on a `DynamicGateway` connection, including in the middle of a
/// request/response pair, so a client has to skip past them rather than
/// assume the next line is its answer.
fn request(
    writer: &mut Conn,
    reader: &mut BufReader<Conn>,
    line: &str,
) -> std::io::Result<String> {
    writeln!(writer, "{line}")?;
    writer.flush()?;
    loop {
        let mut buf = String::new();
        if reader.read_line(&mut buf)? == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "server closed the connection",
            ));
        }
        if buf.contains("\"method\":\"notifications/") {
            println!("  <- push: {}", buf.trim());
            continue;
        }
        return Ok(buf.trim().to_string());
    }
}

/// Pull every `"name":"..."` out of a tools/list response. Demo-grade: good
/// enough for the short identifiers MCP tool names are, not a JSON parser.
/// A production client should use one.
fn tool_names(resp: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = resp;
    while let Some(i) = rest.find("\"name\":\"") {
        rest = &rest[i + 8..];
        match rest.find('"') {
            Some(end) => {
                out.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => break,
        }
    }
    out
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let watch = args.iter().any(|a| a == "--watch");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    let addr = positional
        .first()
        .map(|s| s.as_str())
        .unwrap_or("/tmp/mcp-edge.sock");
    let want_tool = positional.get(1).map(|s| s.as_str());
    let call_args = positional.get(2).map(|s| s.as_str()).unwrap_or("{}");

    println!("connecting to {addr}");
    let mut writer = Conn::connect(addr)?;
    let mut reader = BufReader::new(writer.try_clone()?);
    // Socket options live on the socket, not the fd, so this bounds the read
    // side too. Without it a wedged server hangs the client forever.
    reader.get_ref().set_read_timeout(Some(Duration::from_secs(5)))?;

    // 1. initialize. Every MCP session opens with this; the reply carries the
    //    protocol version and the server's capabilities.
    let init = request(
        &mut writer,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
    )?;
    println!("initialize -> {init}");

    // 2. tools/list. On a gateway this is the aggregated namespace: every
    //    leaf's tools, flattened, with no hint of which device owns what.
    let listed = request(
        &mut writer,
        &mut reader,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    )?;
    let names = tool_names(&listed);
    println!("tools/list -> {} tool(s): {}", names.len(), names.join(", "));

    // 3. tools/call. Named on the command line, or the first one discovered.
    //    The gateway routes by tool name, opens a connection to the owning
    //    leaf, and proxies the result back under this request's id.
    let target = want_tool.map(str::to_string).or_else(|| names.first().cloned());
    match target {
        Some(tool) => {
            let req = format!(
                r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"{tool}","arguments":{call_args}}}}}"#
            );
            println!("calling {tool} with {call_args}");
            println!("tools/call -> {}", request(&mut writer, &mut reader, &req)?);
        }
        None => println!("no tools to call"),
    }

    // 4. Optional: sit on the connection and print pushes. A DynamicGateway
    //    sends notifications/tools/list_changed whenever a leaf is added or
    //    removed, so a client can re-list instead of polling.
    if watch {
        println!("watching for notifications (ctrl-c to stop)");
        // Block indefinitely now, and keep the same reader so anything already
        // buffered from the exchange above isn't dropped on the floor.
        reader.get_ref().set_read_timeout(None)?;
        loop {
            let mut buf = String::new();
            if reader.read_line(&mut buf)? == 0 {
                println!("server closed the connection");
                break;
            }
            println!("  <- {}", buf.trim());
        }
    }

    Ok(())
}
