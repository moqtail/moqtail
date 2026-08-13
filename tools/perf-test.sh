#!/usr/bin/env bash
# perf-test.sh — MOQtail relay throughput/subscriber perf test
#
# Adapted from moqx's scripts/perf-test.sh: starts the MOQtail relay instead of
# moqx, and drives it with moxygen's moqtest_server (publisher) and
# moqperf_test_client (subscriber ramp).  Logs for all three processes are saved
# to /tmp/moqtail-perf-<timestamp>/.
#
# MOQtail speaks only moqt-18, so the publisher and subscriber are pinned to
# --versions=18.
#
# Usage: tools/perf-test.sh [options]
#   --relay PATH           Path to the MOQtail relay binary
#                          (default: target/release/relay)
#   --moqbin PATH          Path to the moxygen bin dir holding moqtest_server and
#                          moqperf_test_client. Required; $MOQBIN also works.
#   -s, --subscriber-max N Max total subscribers (default: 500)
#   --ramp N               Subscribers added per second (default: 100)
#   -d, --duration N       Test duration in seconds (default: 30)
#   --delivery-timeout N   Delivery timeout in ms (default: 500)
#   -t, --transport TYPE   quic or webtransport (default: quic)
#   --threads N            Number of perf client threads (default: 2)
#   --port N               Relay port (default: 4433)
#   -l, --relay-log SPEC   RUST_LOG spec for the relay (default: warn).
#                          Anything at info level logs per control message and
#                          will itself dominate the measurement.
#   --cert PATH / --key PATH
#                          Relay TLS cert/key (default: apps/relay/cert/*.pem)
#   --relay-args ARGS      Extra flags appended to the relay invocation
#                          e.g. --relay-args "--max-subscriber-lag 1000"
#   --client-args ARGS     Extra flags appended to moqperf_test_client
#                          e.g. --client-args "--first_object_size=5000"
#   --client-metrics PATH  Prometheus .prom output for the perf client
#                          (default: LOG_DIR/metrics.prom)
#   --client-timeout N     Kill the perf client if it outlives duration + N
#                          seconds (default: 45).  It does not always exit on
#                          its own once subscribers fail to connect, and the
#                          per-second stats it already printed are the result.
#   --remote-relay HOST    Run the relay on HOST over ssh instead of locally, with
#                          the publisher and subscriber staying here.  Sources are
#                          rsynced to --remote-path and rebuilt there, and the
#                          relay's log is copied back into LOG_DIR at the end.
#                          Measuring on loopback conflates the relay with the load
#                          generator, so this is the topology that gives a number
#                          about the relay.
#   --remote-path PATH     Checkout on HOST to sync into, relative to the remote
#                          home unless absolute (default: moqtail). $REMOTE_PATH
#                          also works.
#   --remote-skip-build    Use the binary already built on HOST
#   --remote-perf N        Profile the remote relay with perf for N seconds once
#                          the subscribers have ramped; the report is copied back
#                          to LOG_DIR/remote/. Linux relay only.

set -euo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

# ── Defaults ───────────────────────────────────────────────────────────────────
BINARY="${RELAY:-$REPO/target/release/relay}"
MOQBIN="${MOQBIN:-}"
SUBSCRIBER_MAX=500
RAMP=100
DURATION=30
DELIVERY_TIMEOUT=500
TRANSPORT="quic"
CLIENT_THREADS=2
RELAY_PORT=4433
RELAY_LOG_SPEC="warn"
CERT_FILE=""
KEY_FILE=""
RELAY_EXTRA_ARGS=()
CLIENT_EXTRA_ARGS=()
METRICS_OUT=""
CLIENT_GRACE=45
REMOTE_RELAY_HOST=""
REMOTE_PATH="${REMOTE_PATH:-moqtail}"
REMOTE_SKIP_BUILD=false
REMOTE_PERF=0

ENDPOINT="/moq-relay"
# MOQtail's only supported draft; all three parties are pinned to it.
DRAFT=18

# ── Arg parsing ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --relay)             BINARY="$2";            shift 2 ;;
    --moqbin)            MOQBIN="$2";            shift 2 ;;
    -s|--subscriber-max) SUBSCRIBER_MAX="$2";    shift 2 ;;
    --ramp)              RAMP="$2";              shift 2 ;;
    -d|--duration)       DURATION="$2";          shift 2 ;;
    --delivery-timeout)  DELIVERY_TIMEOUT="$2";  shift 2 ;;
    -t|--transport)      TRANSPORT="$2";         shift 2 ;;
    --threads)           CLIENT_THREADS="$2";    shift 2 ;;
    --port)              RELAY_PORT="$2";        shift 2 ;;
    -l|--relay-log)      RELAY_LOG_SPEC="$2";    shift 2 ;;
    --cert)              CERT_FILE="$2";         shift 2 ;;
    --key)               KEY_FILE="$2";          shift 2 ;;
    --relay-args)        read -ra RELAY_EXTRA_ARGS <<< "$2";  shift 2 ;;
    --client-args)       read -ra CLIENT_EXTRA_ARGS <<< "$2"; shift 2 ;;
    --client-metrics)    METRICS_OUT="$2";       shift 2 ;;
    --client-timeout)    CLIENT_GRACE="$2";      shift 2 ;;
    --remote-relay)      REMOTE_RELAY_HOST="$2"; shift 2 ;;
    --remote-path)       REMOTE_PATH="$2";       shift 2 ;;
    --remote-skip-build) REMOTE_SKIP_BUILD=true; shift ;;
    --remote-perf)       REMOTE_PERF="$2";       shift 2 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

if [[ -z "$MOQBIN" ]]; then
  echo "ERROR: set --moqbin (or MOQBIN) to the moxygen bin directory" >&2; exit 1
fi
CERT_FILE="${CERT_FILE:-$REPO/apps/relay/cert/cert.pem}"
KEY_FILE="${KEY_FILE:-$REPO/apps/relay/cert/key.pem}"
MOQTEST_SERVER="$MOQBIN/moqtest_server"
MOQPERF_CLIENT="$MOQBIN/moqperf_test_client"

# ── Log directory ─────────────────────────────────────────────────────────────
LOG_DIR="/tmp/moqtail-perf-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$LOG_DIR"
RELAY_LOG="$LOG_DIR/relay.log"
SERVER_LOG="$LOG_DIR/server.log"
CLIENT_LOG="$LOG_DIR/client.log"
echo "Logs: $LOG_DIR"

METRICS_OUT="${METRICS_OUT:-$LOG_DIR/metrics.prom}"
echo "Metrics (.prom): $METRICS_OUT"

# ── Prereq checks ──────────────────────────────────────────────────────────────
CHECK_BINS=("$MOQTEST_SERVER" "$MOQPERF_CLIENT")
# With a remote relay the binary and the certs live on that host, not here.
[[ -z "$REMOTE_RELAY_HOST" ]] && CHECK_BINS+=("$BINARY")
for f in "${CHECK_BINS[@]}"; do
  if [[ ! -x "$f" ]]; then
    echo "ERROR: not found or not executable: $f" >&2; exit 1
  fi
done
if [[ -z "$REMOTE_RELAY_HOST" ]]; then
  for f in "$CERT_FILE" "$KEY_FILE"; do
    [[ -r "$f" ]] || { echo "ERROR: cannot read $f" >&2; exit 1; }
  done
fi

case "$TRANSPORT" in
  quic)         CLIENT_TRANSPORT="quic" ;;
  webtransport) CLIENT_TRANSPORT="h3wt" ;;
  *) echo "ERROR: --transport must be 'quic' or 'webtransport'" >&2; exit 1 ;;
esac
if [[ "$RAMP" -le 0 ]]; then
  echo "ERROR: --ramp must be > 0" >&2; exit 1
fi

if [[ -n "$REMOTE_RELAY_HOST" ]]; then
  # The URL must carry an address the publisher and subscriber can reach, so it comes
  # from ssh's own resolution of the host rather than the alias.
  RELAY_ADDR="$(ssh -G "$REMOTE_RELAY_HOST" 2>/dev/null | awk '/^hostname /{print $2; exit}')"
  [[ -n "$RELAY_ADDR" ]] || { echo "ERROR: cannot resolve ssh host $REMOTE_RELAY_HOST" >&2; exit 1; }
  ssh -o ConnectTimeout=10 "$REMOTE_RELAY_HOST" true \
    || { echo "ERROR: cannot ssh to $REMOTE_RELAY_HOST" >&2; exit 1; }
  if ssh "$REMOTE_RELAY_HOST" "ss -lun 2>/dev/null | grep -q ':$RELAY_PORT '"; then
    echo "ERROR: UDP port $RELAY_PORT already in use on $REMOTE_RELAY_HOST" >&2; exit 1
  fi
else
  RELAY_ADDR="127.0.0.1"
  # The relay listens on UDP only (QUIC), so the TCP-oriented listener checks used
  # for HTTP servers do not apply.
  if lsof -nP -iUDP:"$RELAY_PORT" 2>/dev/null | grep -q .; then
    echo "ERROR: UDP port $RELAY_PORT already in use" >&2; exit 1
  fi
fi

RELAY_URL="https://${RELAY_ADDR}:${RELAY_PORT}${ENDPOINT}"

# ── Cleanup ───────────────────────────────────────────────────────────────────
PIDS=()
RELAY_PID=""
REMOTE_RELAY_PID=""
REMOTE_LOG_DIR="/tmp/moqtail-perf-remote-$(date +%Y%m%d-%H%M%S)"
cleanup() {
  exec 2>/dev/null   # the relay/publisher are killed here; skip the job-control noise
  if [[ -n "$REMOTE_RELAY_HOST" && -n "$REMOTE_RELAY_PID" ]]; then
    # Read the CPU counters before the kill, while the process still exists.
    if [[ -n "$REMOTE_CPU_START" ]]; then
      local cpu_end elapsed ticks
      cpu_end="$(ssh "$REMOTE_RELAY_HOST" \
        "awk '{print \$14+\$15}' /proc/$REMOTE_RELAY_PID/stat 2>/dev/null")"
      elapsed=$(( $(date +%s) - REMOTE_CPU_T0 ))
      if [[ -n "$cpu_end" && "$elapsed" -gt 0 ]]; then
        ticks=$(( cpu_end - REMOTE_CPU_START ))
        {
          echo "relay CPU over ${elapsed}s of run:"
          echo "  cpu_seconds:  $(awk -v t="$ticks" 'BEGIN{printf "%.1f", t/100}')"
          echo "  cores_busy:   $(awk -v t="$ticks" -v e="$elapsed" 'BEGIN{printf "%.2f", t/100/e}')"
          echo "  cores_total:  ${REMOTE_CORES:-unknown}"
        } | tee "$LOG_DIR/relay-cpu.txt"
      fi
    fi
    # SIGKILL rather than SIGTERM: the relay drains for 10s on TERM, and there is
    # nothing left to drain to.
    ssh "$REMOTE_RELAY_HOST" "kill -9 $REMOTE_RELAY_PID 2>/dev/null; true"
    mkdir -p "$LOG_DIR/remote"
    rsync -az "$REMOTE_RELAY_HOST:$REMOTE_LOG_DIR/" "$LOG_DIR/remote/" 2>/dev/null || true
    ssh "$REMOTE_RELAY_HOST" "rm -rf $REMOTE_LOG_DIR" 2>/dev/null || true
  fi
  [[ -n "$RELAY_PID" ]] && kill "$RELAY_PID" 2>/dev/null || true
  for pid in ${PIDS[@]+"${PIDS[@]}"}; do kill "$pid" 2>/dev/null || true; done
  local deadline=$(( $(date +%s) + 5 ))
  for pid in ${PIDS[@]+"${PIDS[@]}"} ${RELAY_PID:+$RELAY_PID}; do
    while kill -0 "$pid" 2>/dev/null; do
      (( $(date +%s) >= deadline )) && { kill -KILL "$pid" 2>/dev/null || true; break; }
      sleep 0.1
    done
  done
  wait ${PIDS[@]+"${PIDS[@]}"} 2>/dev/null || true
  [[ -n "$RELAY_PID" ]] && wait "$RELAY_PID" 2>/dev/null || true
  echo "Logs saved to $LOG_DIR"
}
trap cleanup EXIT

# ── Run params ────────────────────────────────────────────────────────────────
{
  echo "date:             $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "moqtail_git:      $(git -C "$REPO" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  if [[ -n "$REMOTE_RELAY_HOST" ]]; then
    echo "relay_binary:     $REMOTE_RELAY_HOST:$REMOTE_PATH/target/release/relay"
    echo "relay_build:      ${REMOTE_BUILD_ID:-unknown}"
  else
    echo "relay_binary:     $BINARY"
  fi
  echo "moqbin:           $MOQBIN"
  echo "relay_url:        $RELAY_URL"
  echo "transport:        $TRANSPORT"
  echo "draft:            $DRAFT"
  echo "subscriber_max:   $SUBSCRIBER_MAX"
  echo "ramp:             $RAMP"
  echo "duration:         $DURATION"
  echo "delivery_timeout: $DELIVERY_TIMEOUT"
  echo "client_threads:   $CLIENT_THREADS"
  echo "relay_log:        $RELAY_LOG_SPEC"
  echo "relay_extra_args: ${RELAY_EXTRA_ARGS[*]-}"
  echo "client_extra_args:${CLIENT_EXTRA_ARGS[*]-}"
} | tee "$LOG_DIR/run_params.txt"
echo ""

# ── Start the MOQtail relay ───────────────────────────────────────────────────
ulimit -n 65536 2>/dev/null || true

if [[ -n "$REMOTE_RELAY_HOST" ]]; then
  echo "Syncing relay sources to $REMOTE_RELAY_HOST:$REMOTE_PATH ..."
  # Every workspace member has to be present for `cargo build -p relay` to resolve,
  # so all three are synced rather than leaning on whatever the remote already has.
  # Trailing slashes on both sides matter: without them rsync would nest the source
  # directory's basename under the destination.
  for member in apps/relay apps/client libs/moqtail-rs; do
    rsync -az --exclude target "$REPO/$member/" "$REMOTE_RELAY_HOST:$REMOTE_PATH/$member/" \
      || { echo "ERROR: rsync of $member to $REMOTE_RELAY_HOST failed" >&2; exit 1; }
  done
  rsync -az "$REPO/Cargo.toml" "$REPO/Cargo.lock" "$REMOTE_RELAY_HOST:$REMOTE_PATH/" \
    || { echo "ERROR: rsync of the workspace manifest failed" >&2; exit 1; }

  # Profiling a release build without frame pointers yields stacks that stop at the
  # first frame, so a profiled run is built with them (and with debug info, for
  # symbol names). Costs a percent or so of the thing being measured, which is worth
  # far less than knowing which caller took the lock.
  REMOTE_BUILD_ENV=""
  if [[ "$REMOTE_PERF" -gt 0 ]]; then
    REMOTE_BUILD_ENV='RUSTFLAGS="-C force-frame-pointers=yes" CARGO_PROFILE_RELEASE_DEBUG=1'
  fi

  if [[ "$REMOTE_SKIP_BUILD" != true ]]; then
    echo "Building the relay on $REMOTE_RELAY_HOST ..."
    # A non-interactive ssh shell reads none of the profile scripts that put cargo on
    # PATH, so it is put there here. The output goes to a file rather than through a
    # pipe: piping into `tail` would make the pipeline's status tail's, and a failed
    # build would sail past unnoticed and be measured as if it were the new code.
    if ! ssh "$REMOTE_RELAY_HOST" "
      [ -f \"\$HOME/.cargo/env\" ] && . \"\$HOME/.cargo/env\"
      command -v cargo >/dev/null 2>&1 || export PATH=\"\$HOME/.cargo/bin:\$PATH\"
      command -v cargo >/dev/null 2>&1 || { echo 'cargo not found on PATH'; exit 127; }
      cd '$REMOTE_PATH' && $REMOTE_BUILD_ENV cargo build --release -p relay" \
      >"$LOG_DIR/remote-build.log" 2>&1; then
      echo "ERROR: remote build failed:" >&2
      tail -20 "$LOG_DIR/remote-build.log" >&2
      exit 1
    fi
    tail -2 "$LOG_DIR/remote-build.log"
  fi

  # A sync that silently misses tells the same story as one that works, so the run
  # records which binary it actually got.
  REMOTE_BUILD_ID="$(ssh "$REMOTE_RELAY_HOST" \
    "stat -c '%y %s bytes' '$REMOTE_PATH/target/release/relay' 2>/dev/null | cut -c1-19,21-")"

  echo "Starting MOQtail relay on $REMOTE_RELAY_HOST:$RELAY_PORT (transport=$TRANSPORT)..."
  # ulimit and the UDP socket buffers are raised in the same shell as the relay:
  # a single QUIC socket serving hundreds of subscribers outgrows the defaults.
  REMOTE_RELAY_PID=$(ssh "$REMOTE_RELAY_HOST" "
    mkdir -p '$REMOTE_LOG_DIR'
    cd '$REMOTE_PATH'
    ulimit -n 65536 2>/dev/null || true
    RUST_LOG='$RELAY_LOG_SPEC' nohup ./target/release/relay \
      --port $RELAY_PORT \
      --host 0.0.0.0 \
      --cert-file '$REMOTE_PATH/apps/relay/cert/cert.pem' \
      --key-file '$REMOTE_PATH/apps/relay/cert/key.pem' \
      --log-folder '$REMOTE_LOG_DIR' \
      ${RELAY_EXTRA_ARGS[*]-} \
      > '$REMOTE_LOG_DIR/stdout.log' 2>&1 &
    echo \$!")
  [[ -n "$REMOTE_RELAY_PID" ]] || { echo "ERROR: could not start the remote relay" >&2; exit 1; }

  deadline=$(( $(date +%s) + 30 ))
  until ssh "$REMOTE_RELAY_HOST" "ss -lun | grep -q ':$RELAY_PORT '"; do
    ssh "$REMOTE_RELAY_HOST" "kill -0 $REMOTE_RELAY_PID 2>/dev/null" \
      || { echo "ERROR: remote relay exited during startup:" >&2
           ssh "$REMOTE_RELAY_HOST" "tail -20 '$REMOTE_LOG_DIR/stdout.log'" >&2; exit 1; }
    (( $(date +%s) >= deadline )) && { echo "ERROR: remote relay not listening after 30s" >&2; exit 1; }
    sleep 0.5
  done
  echo "Relay ready on $RELAY_ADDR:$RELAY_PORT (remote pid $REMOTE_RELAY_PID)"

  # CPU accounting straight from /proc, which needs none of the privileges perf does.
  # utime+stime in clock ticks, sampled here and again before the relay is killed.
  REMOTE_CPU_START="$(ssh "$REMOTE_RELAY_HOST" \
    "awk '{print \$14+\$15}' /proc/$REMOTE_RELAY_PID/stat 2>/dev/null")"
  REMOTE_CPU_T0=$(date +%s)
  REMOTE_CORES="$(ssh "$REMOTE_RELAY_HOST" nproc 2>/dev/null)"
  echo "relay_build:      ${REMOTE_BUILD_ID:-unknown}" >> "$LOG_DIR/run_params.txt"
  echo "remote_cores:     ${REMOTE_CORES:-unknown}" >> "$LOG_DIR/run_params.txt"
else
  echo "Starting MOQtail relay on port $RELAY_PORT (transport=$TRANSPORT)..."

  # --log-folder gets its own rolling relay.log; the stdout copy is what this
  # script tails for readiness.
  RUST_LOG="$RELAY_LOG_SPEC" "$BINARY" \
    --port "$RELAY_PORT" \
    --host 127.0.0.1 \
    --cert-file "$CERT_FILE" \
    --key-file "$KEY_FILE" \
    --log-folder "$LOG_DIR" \
    ${RELAY_EXTRA_ARGS[@]+"${RELAY_EXTRA_ARGS[@]}"} \
    >"$RELAY_LOG" 2>&1 &
  RELAY_PID=$!

  deadline=$(( $(date +%s) + 10 ))
  until lsof -nP -iUDP:"$RELAY_PORT" -a -p "$RELAY_PID" 2>/dev/null | grep -q .; do
    kill -0 "$RELAY_PID" 2>/dev/null || { echo "ERROR: relay exited during startup; see $RELAY_LOG" >&2; exit 1; }
    (( $(date +%s) >= deadline )) && { echo "ERROR: relay not listening after 10s" >&2; exit 1; }
    sleep 0.1
  done
  echo "Relay ready on port $RELAY_PORT (pid $RELAY_PID)"
fi

# ── Start moqtest_server (publisher) ─────────────────────────────────────────
echo "Starting moqtest_server -> $RELAY_URL ..."
"$MOQTEST_SERVER" \
  --relay_url="$RELAY_URL" \
  --transport="$CLIENT_TRANSPORT" \
  --versions="$DRAFT" \
  --include_timestamp_extension=true \
  >"$SERVER_LOG" 2>&1 &
PIDS+=($!)

deadline=$(( $(date +%s) + 10 ))
until grep -q "Successfully published namespace" "$SERVER_LOG" 2>/dev/null; do
  (( $(date +%s) >= deadline )) && { echo "ERROR: moqtest_server did not publish its namespace after 10s; see $SERVER_LOG" >&2; exit 1; }
  sleep 0.1
done
echo "Publisher connected: $(grep -m1 -o "namespace '[^']*'" "$SERVER_LOG")"

# ── Profile the remote relay (optional) ───────────────────────────────────────
if [[ -n "$REMOTE_RELAY_HOST" && "$REMOTE_PERF" -gt 0 ]]; then
  # Start after the ramp, so the profile describes steady state rather than a
  # few hundred handshakes.
  perf_delay=$(( SUBSCRIBER_MAX / RAMP + 5 ))
  echo "perf: starts in ${perf_delay}s, records ${REMOTE_PERF}s on the remote relay"
  (
    sleep "$perf_delay"
    ssh "$REMOTE_RELAY_HOST" "
      cd '$REMOTE_LOG_DIR'
      perf record -F 499 --call-graph fp -p $REMOTE_RELAY_PID -o perf.data -- sleep $REMOTE_PERF \
        > perf-record.log 2>&1
      perf report -i perf.data --stdio --no-children -g graph,0.5,caller 2>/dev/null | head -120 > perf-report.txt
      perf report -i perf.data --stdio --no-children --sort symbol 2>/dev/null | head -40 > perf-flat.txt
      perf report -i perf.data --stdio --sort dso 2>/dev/null | head -30 > perf-by-dso.txt
    " || echo "WARNING: remote perf failed (see $LOG_DIR/remote/perf-record.log)" >&2
  ) &
  PIDS+=($!)
fi

# ── Run moqperf_test_client ───────────────────────────────────────────────────
echo "Running perf client: subscriber_max=$SUBSCRIBER_MAX ramp=$RAMP duration=${DURATION}s delivery_timeout=${DELIVERY_TIMEOUT}ms threads=$CLIENT_THREADS"
echo "---"
# The client keeps retrying failed subscribers and does not always exit once the
# run is over, so it is killed after the run plus a grace period. Whatever it
# printed by then is the result, and it is already in CLIENT_LOG.
( sleep $(( DURATION + CLIENT_GRACE )) && pkill -KILL -f moqperf_test_client ) >/dev/null 2>&1 &
PIDS+=($!)

"$MOQPERF_CLIENT" \
  --relay_url="$RELAY_URL" \
  --transport="$CLIENT_TRANSPORT" \
  --versions="$DRAFT" \
  --subscriber_max="$SUBSCRIBER_MAX" \
  --subscriber_ramp="$RAMP" \
  --duration="$DURATION" \
  --delivery_timeout="$DELIVERY_TIMEOUT" \
  --num_threads="$CLIENT_THREADS" \
  --metrics_out="$METRICS_OUT" \
  ${CLIENT_EXTRA_ARGS[@]+"${CLIENT_EXTRA_ARGS[@]}"} \
  2>&1 | tee "$CLIENT_LOG"
