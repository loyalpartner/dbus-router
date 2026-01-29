#!/bin/bash
# Test script for dbus-router
set -e

# Paths
ROUTER_BIN="./target/debug/dbus-router"
PROXY_SOCK="/tmp/dbus-router-test.sock"
SANDBOX_SOCK="/tmp/sandbox-bus-test.sock"
CONFIG_FILE="/tmp/router-test.toml"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

cleanup() {
    echo -e "${YELLOW}Cleaning up...${NC}"
    [ -n "$ROUTER_PID" ] && kill $ROUTER_PID 2>/dev/null || true
    [ -n "$SANDBOX_PID" ] && kill $SANDBOX_PID 2>/dev/null || true
    rm -f "$PROXY_SOCK" "$SANDBOX_SOCK" "$CONFIG_FILE"
}
trap cleanup EXIT

echo -e "${YELLOW}=== D-Bus Router Test ===${NC}"

# Build
echo -e "${YELLOW}Building...${NC}"
cargo build --quiet

# Create test config
cat > "$CONFIG_FILE" << 'EOF'
[[host_routes]]
destination = "org.freedesktop.DBus"

[[host_routes]]
destination = "org.freedesktop.portal.*"
EOF
echo -e "${GREEN}Created config: $CONFIG_FILE${NC}"

# Start a sandbox dbus-daemon
echo -e "${YELLOW}Starting sandbox D-Bus daemon...${NC}"
dbus-daemon --session --address="unix:path=$SANDBOX_SOCK" --nofork --print-pid &
SANDBOX_PID=$!
sleep 0.5

if ! kill -0 $SANDBOX_PID 2>/dev/null; then
    echo -e "${RED}Failed to start sandbox dbus-daemon${NC}"
    exit 1
fi
echo -e "${GREEN}Sandbox bus running at: $SANDBOX_SOCK (PID: $SANDBOX_PID)${NC}"

# Get host bus address
HOST_BUS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=/run/user/$(id -u)/bus}"
echo -e "${GREEN}Host bus: $HOST_BUS${NC}"

# Start router
echo -e "${YELLOW}Starting dbus-router...${NC}"
RUST_LOG=debug "$ROUTER_BIN" \
    --listen "$PROXY_SOCK" \
    --host "$HOST_BUS" \
    --sandbox "unix:path=$SANDBOX_SOCK" \
    --config "$CONFIG_FILE" &
ROUTER_PID=$!
sleep 0.5

if ! kill -0 $ROUTER_PID 2>/dev/null; then
    echo -e "${RED}Failed to start dbus-router${NC}"
    exit 1
fi
echo -e "${GREEN}Router running at: $PROXY_SOCK (PID: $ROUTER_PID)${NC}"

echo ""
echo -e "${YELLOW}=== Test 1: Route to HOST bus (org.freedesktop.DBus) ===${NC}"
echo "Calling org.freedesktop.DBus.ListNames..."
RESULT=$(DBUS_SESSION_BUS_ADDRESS="unix:path=$PROXY_SOCK" \
    dbus-send --session --dest=org.freedesktop.DBus \
    --print-reply / org.freedesktop.DBus.ListNames 2>&1) || true

if echo "$RESULT" | grep -q "org.freedesktop.DBus"; then
    echo -e "${GREEN}✓ Got response from host bus${NC}"
    echo "$RESULT" | head -5
else
    echo -e "${RED}✗ Failed to get response${NC}"
    echo "$RESULT"
fi

echo ""
echo -e "${YELLOW}=== Test 2: Route to SANDBOX bus (org.example.Test) ===${NC}"
echo "Calling org.example.Test (should go to sandbox, will fail as no such service)..."
RESULT=$(DBUS_SESSION_BUS_ADDRESS="unix:path=$PROXY_SOCK" \
    dbus-send --session --dest=org.example.Test \
    --print-reply / org.freedesktop.DBus.Peer.Ping 2>&1) || true

if echo "$RESULT" | grep -q "org.freedesktop.DBus.Error"; then
    echo -e "${GREEN}✓ Correctly routed to sandbox (got expected error)${NC}"
    echo "$RESULT" | head -2
else
    echo -e "${YELLOW}? Unexpected response${NC}"
    echo "$RESULT"
fi

echo ""
echo -e "${YELLOW}=== Test 3: Verify host bus names are visible ===${NC}"
NAMES=$(DBUS_SESSION_BUS_ADDRESS="unix:path=$PROXY_SOCK" \
    dbus-send --session --dest=org.freedesktop.DBus \
    --print-reply / org.freedesktop.DBus.ListNames 2>&1 | grep string | head -10)
echo "$NAMES"

echo ""
echo -e "${GREEN}=== Tests completed ===${NC}"
