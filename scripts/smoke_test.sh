#!/usr/bin/env bash
# Runs a server and one client headless for a few seconds and reports what each logged.
set -u
cd "$(dirname "$0")/.."

PORT="${1:-5070}"
SECONDS_TO_RUN="${2:-45}"
export XDG_RUNTIME_DIR=/tmp/xdg
mkdir -p "$XDG_RUNTIME_DIR"

./target/debug/deepcore_server --bind "127.0.0.1:$PORT" --seed 7 > /tmp/deepcore_server.log 2>&1 &
SERVER_PID=$!
sleep 4

xvfb-run -a -s "-screen 0 1280x720x24" \
  ./target/debug/deepcore_client --connect "127.0.0.1:$PORT" --name Tester \
  > /tmp/deepcore_client.log 2>&1 &
CLIENT_PID=$!

sleep "$SECONDS_TO_RUN"

kill "$CLIENT_PID" 2>/dev/null
kill "$SERVER_PID" 2>/dev/null
wait 2>/dev/null

echo "===================== SERVER ====================="
grep -vE "ALSA|XDG_RUNTIME" /tmp/deepcore_server.log | tail -25
echo "===================== CLIENT ====================="
grep -vE "ALSA|XDG_RUNTIME" /tmp/deepcore_client.log | tail -30
