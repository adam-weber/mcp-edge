#!/bin/bash
# Simple test client for mcp-edge
# Usage: ./test_client.sh

SOCK="/tmp/mcp-edge.sock"

send() {
    echo "$1" | nc -U "$SOCK"
}

echo "=== MCP Edge Test Client ==="
echo ""

echo "1. Initialize:"
send '{"jsonrpc":"2.0","id":1,"method":"initialize"}'
echo ""

echo "2. List tools:"
send '{"jsonrpc":"2.0","id":2,"method":"tools/list"}'
echo ""

echo "3. Read temperature:"
send '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"temp_read","arguments":{}}}'
echo ""

echo "4. Read humidity:"
send '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"humidity_read","arguments":{}}}'
echo ""
