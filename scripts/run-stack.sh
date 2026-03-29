#!/usr/bin/env bash
#
# Run the full MOQtail stack: build Rust, start relay, publisher, and client-js.
# Logs are written to logs/ with one file per component.
#
# Usage:
#   ./scripts/run-stack.sh [VIDEO_PATH]   Start the full stack
#   ./scripts/run-stack.sh stop           Stop all running components
#
# Examples:
#   ./scripts/run-stack.sh                                  # uses default 1080p video
#   ./scripts/run-stack.sh data/video/smoking_test_480p.mp4 # custom video
#   ./scripts/run-stack.sh stop                             # kill all components

set -euo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
LOG_DIR="$ROOT_DIR/logs"
PID_FILE="$LOG_DIR/.stack.pids"

# --- Stop command ---
if [[ "${1:-}" == "stop" ]]; then
  if [[ ! -f "$PID_FILE" ]]; then
    echo "No running stack found."
    exit 0
  fi
  echo "Stopping MOQtail stack..."
  while IFS='=' read -r name pid; do
    if kill -0 "$pid" 2>/dev/null; then
      echo "  Stopping $name (PID $pid)..."
      kill "$pid" 2>/dev/null || true
    fi
  done < "$PID_FILE"
  rm -f "$PID_FILE"
  echo "All stopped."
  exit 0
fi

VIDEO_PATH="${1:-data/video/smoking_test_1080p.mp4}"

cleanup() {
  echo ""
  echo "Shutting down..."
  if [[ -f "$PID_FILE" ]]; then
    while IFS='=' read -r name pid; do
      if kill -0 "$pid" 2>/dev/null; then
        echo "  Stopping $name (PID $pid)..."
        kill "$pid" 2>/dev/null || true
      fi
    done < "$PID_FILE"
    rm -f "$PID_FILE"
  fi
  wait 2>/dev/null || true
  echo "All processes stopped. Logs are in $LOG_DIR/"
}

trap cleanup EXIT INT TERM

# --- Validate video path ---
if [[ ! -f "$ROOT_DIR/$VIDEO_PATH" && ! -f "$VIDEO_PATH" ]]; then
  echo "Error: Video file not found: $VIDEO_PATH"
  echo "Available videos in data/video/:"
  ls "$ROOT_DIR/data/video/" 2>/dev/null || echo "  (none)"
  exit 1
fi

# Resolve to absolute path if relative
if [[ -f "$ROOT_DIR/$VIDEO_PATH" ]]; then
  VIDEO_PATH="$ROOT_DIR/$VIDEO_PATH"
fi

# --- Create log directory ---
mkdir -p "$LOG_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)

echo "=== MOQtail Stack ==="
echo "  Video:  $VIDEO_PATH"
echo "  Logs:   $LOG_DIR/"
echo ""

# --- Build Rust components ---
echo "[build] Building Rust components..."
cargo build --release --manifest-path "$ROOT_DIR/Cargo.toml" 2>&1 | tee "$LOG_DIR/build_${TIMESTAMP}.log"
echo "[build] Done."
echo ""

# Reset PID file
> "$PID_FILE"

# --- Start relay ---
echo "[relay] Starting relay on port 4433..."
cargo run --release --manifest-path "$ROOT_DIR/Cargo.toml" --bin relay -- \
  --port 4433 \
  --cert-file "$ROOT_DIR/apps/relay/cert/cert.pem" \
  --key-file "$ROOT_DIR/apps/relay/cert/key.pem" \
  --log-folder "$LOG_DIR" \
  > "$LOG_DIR/relay_${TIMESTAMP}.log" 2>&1 &
RELAY_PID=$!
echo "relay=$RELAY_PID" >> "$PID_FILE"
echo "[relay] PID $RELAY_PID — log: relay_${TIMESTAMP}.log"

# Give relay a moment to bind
sleep 2

# --- Start publisher ---
echo "[publisher] Starting publisher with: $(basename "$VIDEO_PATH")"
cargo run --release --manifest-path "$ROOT_DIR/Cargo.toml" --bin publisher -- \
  --video-path "$VIDEO_PATH" \
  --max-variants 2 \
  > "$LOG_DIR/publisher_${TIMESTAMP}.log" 2>&1 &
PUB_PID=$!
echo "publisher=$PUB_PID" >> "$PID_FILE"
echo "[publisher] PID $PUB_PID — log: publisher_${TIMESTAMP}.log"

# Give publisher a moment to connect and start encoding
sleep 3

# --- Start client-js ---
echo "[client-js] Starting Vite dev server..."
npm run --prefix "$ROOT_DIR/apps/client-js" dev \
  > "$LOG_DIR/client-js_${TIMESTAMP}.log" 2>&1 &
CLIENT_PID=$!
echo "client-js=$CLIENT_PID" >> "$PID_FILE"
echo "[client-js] PID $CLIENT_PID — log: client-js_${TIMESTAMP}.log"

echo ""
echo "=== All running ==="
echo "  Relay:      https://127.0.0.1:4433"
echo "  Client-JS:  http://localhost:5173"
echo ""
echo "Stop with:  ./scripts/run-stack.sh stop  (from another terminal)"
echo "       or:  Ctrl+C"
echo ""

# Wait for all children — if any exits, cleanup triggers
wait
