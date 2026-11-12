#!/usr/bin/env bash
# Runs a server and a client, waits for the world to stream in, and captures a screenshot.
set -u
cd "$(dirname "$0")/.."

PORT="${1:-5090}"
WAIT="${2:-50}"
export XDG_RUNTIME_DIR=/tmp/xdg
mkdir -p "$XDG_RUNTIME_DIR"

./target/debug/deepcore_server --bind "127.0.0.1:$PORT" --seed 7 > /tmp/shot_server.log 2>&1 &
SERVER_PID=$!
sleep 4

xvfb-run -a -s "-screen 0 1280x720x24" \
  ./target/debug/deepcore_client --connect "127.0.0.1:$PORT" --name Screenshot \
  > /tmp/shot_client.log 2>&1 &
CLIENT_PID=$!

sleep "$WAIT"

DISPLAY_NUM=$(ls /tmp/.X11-unix/ 2>/dev/null | head -1 | tr -d 'X')
if [ -n "${DISPLAY_NUM:-}" ] && command -v import >/dev/null 2>&1; then
  DISPLAY=":$DISPLAY_NUM" import -window root /tmp/deepcore_shot.png 2>/dev/null
fi

kill "$CLIENT_PID" 2>/dev/null
kill "$SERVER_PID" 2>/dev/null
wait 2>/dev/null
echo "screenshot: $(ls -la /tmp/deepcore_shot.png 2>/dev/null || echo none)"
