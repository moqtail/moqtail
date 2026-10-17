#!/usr/bin/env bash
# conformance-test.sh — run moxygen's MoQTest conformance suite against a relay
#
# Topology:  moqtest_server --(publish)--> relay <--(subscribe)-- moqtest_client
#
# The suite drives 41 cases through the relay (forwarding preferences, object
# and group counts, object sizes, group/object increments, end-of-group markers,
# extensions, delivery timeouts) and verifies every object the subscriber gets
# against what the publisher generated.  The relay under test is either started
# by this script or reached over the network with --relay-url.
#
# All three parties are pinned to moqt-18, MOQtail's only supported version:
# left to negotiate, the moxygen binaries would offer older drafts too and the
# publisher and subscriber could settle on different ones, which the relay
# cannot bridge.
#
# Prerequisites: see docs/interop-testing.md (moxygen binaries + macOS dylibs).
#
# Usage: tools/conformance-test.sh [options]
#   -t, --transport TYPE  quic or webtransport (default: quic)
#   --relay-url URL       Test a running relay (e.g. https://relay18.moqtail.dev/moq)
#                         instead of starting a local one
#   --relay PATH          Path to the MOQtail relay binary
#                         (default: target/debug/relay)
#   --moqbin PATH         Path to the moxygen bin dir
#                         (default: $MOQBIN or the newest
#                          ~/.cache/moqx/moxygen-*/bin)
#   --conformance PATH    Path to moxygen's conformance_test.sh
#                         (default: $CONFORMANCE_SCRIPT or the newest
#                          ~/.cache/moqx/cpm/moxygen/*/moxygen/moqtest/conformance_test.sh)
#   --port N              Local relay port (default: 4433)
#   -l, --relay-log SPEC  RUST_LOG spec for the local relay (default: info)
#   --cert PATH / --key PATH
#                         Relay TLS cert/key (default: apps/relay/cert/*.pem)
#
# Exits non-zero if any case fails.  Logs and the suite's own report go to
# /tmp/moqtail-conformance-<timestamp>/.

set -uo pipefail

REPO="$(cd "$(dirname "$0")/.." && pwd)"

# ── Defaults ───────────────────────────────────────────────────────────────────
BINARY="${RELAY:-$REPO/target/debug/relay}"

# moqx resolves moxygen through CPM into a cache outside its repo, keyed by the
# pinned revision, so neither path is fixed: take the newest of each. Both are
# still overridable, by flag or by env, when a specific one is wanted.
newest() { ls -dt $1 2>/dev/null | head -1; }
MOQBIN="${MOQBIN:-$(newest "$HOME/.cache/moqx/moxygen-*/bin")}"
CONFORMANCE="${CONFORMANCE_SCRIPT:-$(newest "$HOME/.cache/moqx/cpm/moxygen/*/moxygen/moqtest/conformance_test.sh")}"
TRANSPORT="quic"
RELAY_URL=""
RELAY_PORT=4433
RELAY_LOG_SPEC="info"
CERT_FILE=""
KEY_FILE=""

ENDPOINT="/moq-relay"
DRAFT=18

# ── Arg parsing ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    -t|--transport)  TRANSPORT="$2";      shift 2 ;;
    --relay-url)     RELAY_URL="$2";      shift 2 ;;
    --relay)         BINARY="$2";         shift 2 ;;
    --moqbin)        MOQBIN="$2";         shift 2 ;;
    --conformance)   CONFORMANCE="$2";    shift 2 ;;
    --port)          RELAY_PORT="$2";     shift 2 ;;
    -l|--relay-log)  RELAY_LOG_SPEC="$2"; shift 2 ;;
    --cert)          CERT_FILE="$2";      shift 2 ;;
    --key)           KEY_FILE="$2";       shift 2 ;;
    -h|--help)       sed -n '2,38p' "$0" | sed 's/^#//; s/^ //'; exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

CERT_FILE="${CERT_FILE:-$REPO/apps/relay/cert/cert.pem}"
KEY_FILE="${KEY_FILE:-$REPO/apps/relay/cert/key.pem}"
MOQTEST_SERVER="$MOQBIN/moqtest_server"
MOQTEST_CLIENT="$MOQBIN/moqtest_client"

case "$TRANSPORT" in
  # The suite takes the subscriber's transport as a positional letter; the
  # publisher takes a flag.  Both must match, or the two sessions land on
  # different transports and the relay has nothing to bridge.
  quic)         SERVER_TRANSPORT=(--transport=quic); SUITE_TRANSPORT=(Q) ;;
  webtransport) SERVER_TRANSPORT=(--transport=h3wt); SUITE_TRANSPORT=() ;;
  *) echo "ERROR: --transport must be 'quic' or 'webtransport'" >&2; exit 1 ;;
esac

# ── Prereq checks ──────────────────────────────────────────────────────────────
for f in "$MOQTEST_SERVER" "$MOQTEST_CLIENT" "$CONFORMANCE"; do
  [[ -x "$f" ]] || { echo "ERROR: not found or not executable: $f" >&2; exit 1; }
done
if [[ -z "$RELAY_URL" ]]; then
  [[ -x "$BINARY" ]] || { echo "ERROR: not found or not executable: $BINARY" >&2; exit 1; }
  for f in "$CERT_FILE" "$KEY_FILE"; do
    [[ -r "$f" ]] || { echo "ERROR: cannot read $f" >&2; exit 1; }
  done
fi

# ── Log directory ─────────────────────────────────────────────────────────────
LOG_DIR="/tmp/moqtail-conformance-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$LOG_DIR"
RELAY_LOG="$LOG_DIR/relay.log"
SERVER_LOG="$LOG_DIR/publisher.log"
SUITE_LOG="$LOG_DIR/conformance.log"
echo "Logs: $LOG_DIR"

# ── Cleanup ───────────────────────────────────────────────────────────────────
RELAY_PID=""
SERVER_PID=""
SHIM=""
cleanup() {
  exec 2>/dev/null
  for pid in $SERVER_PID $RELAY_PID; do kill "$pid" 2>/dev/null; done
  local deadline=$(( $(date +%s) + 5 ))
  for pid in $SERVER_PID $RELAY_PID; do
    while kill -0 "$pid" 2>/dev/null; do
      (( $(date +%s) >= deadline )) && { kill -KILL "$pid" 2>/dev/null; break; }
      sleep 0.1
    done
  done
  wait $SERVER_PID $RELAY_PID 2>/dev/null
  [[ -n "$SHIM" ]] && rm -rf "$SHIM"
  echo "Logs saved to $LOG_DIR"
}
trap cleanup EXIT

# ── Start the relay (unless testing a remote one) ─────────────────────────────
if [[ -n "$RELAY_URL" ]]; then
  echo "Testing relay at $RELAY_URL (not started by this script)"
else
  # A relay that has been SIGTERMed keeps the UDP port while it drains, and a new
  # one that cannot bind exits without a message, so check the port up front.
  if lsof -nP -iUDP:"$RELAY_PORT" 2>/dev/null | grep -q .; then
    echo "ERROR: UDP port $RELAY_PORT already in use (relay still draining?)" >&2
    lsof -nP -iUDP:"$RELAY_PORT" >&2
    exit 1
  fi

  RELAY_URL="https://127.0.0.1:${RELAY_PORT}${ENDPOINT}"
  echo "Starting MOQtail relay on port $RELAY_PORT ..."
  RUST_LOG="$RELAY_LOG_SPEC" "$BINARY" \
    --port "$RELAY_PORT" \
    --host 127.0.0.1 \
    --cert-file "$CERT_FILE" \
    --key-file "$KEY_FILE" \
    >"$RELAY_LOG" 2>&1 &
  RELAY_PID=$!

  deadline=$(( $(date +%s) + 10 ))
  until lsof -nP -iUDP:"$RELAY_PORT" -a -p "$RELAY_PID" 2>/dev/null | grep -q .; do
    kill -0 "$RELAY_PID" 2>/dev/null || { echo "ERROR: relay exited during startup; see $RELAY_LOG" >&2; exit 1; }
    (( $(date +%s) >= deadline )) && { echo "ERROR: relay not listening after 10s" >&2; exit 1; }
    sleep 0.1
  done
  echo "Relay ready (pid $RELAY_PID)"
fi

# ── Start moqtest_server (publisher) ─────────────────────────────────────────
echo "Starting moqtest_server ($TRANSPORT) -> $RELAY_URL ..."
"$MOQTEST_SERVER" \
  --relay_url="$RELAY_URL" \
  "${SERVER_TRANSPORT[@]}" \
  --versions="$DRAFT" \
  --logtostderr \
  >"$SERVER_LOG" 2>&1 &
SERVER_PID=$!

# moqtest_server says nothing on success — it logs only when it fails — so there
# is no line to wait for in its own log. A local relay's log names the namespace
# it stored, which is the real signal; otherwise all that can be done is give it
# a moment and check the publisher is still alive and unbroken.
PUBLISH_FAILED="Relay setup failed|Failed to establish relay session|PublishNamespaceError"
settle=$(( $(date +%s) + 3 ))
while :; do
  kill -0 "$SERVER_PID" 2>/dev/null \
    || { echo "ERROR: publisher exited; see $SERVER_LOG" >&2; tail -5 "$SERVER_LOG" >&2; exit 1; }
  if grep -qE "$PUBLISH_FAILED" "$SERVER_LOG" 2>/dev/null; then
    echo "ERROR: publisher could not publish its namespace; see $SERVER_LOG" >&2
    tail -5 "$SERVER_LOG" >&2
    exit 1
  fi
  if [[ -n "$RELAY_PID" ]] && grep -q "Stored announcement for namespace" "$RELAY_LOG" 2>/dev/null; then
    echo "Publisher connected: relay stored the moq-test-00 announcement"
    break
  fi
  if (( $(date +%s) >= settle )); then
    echo "Publisher connected (no relay log to confirm against)"
    break
  fi
  sleep 0.2
done

# ── Run the suite ─────────────────────────────────────────────────────────────
# conformance_test.sh looks for the subscriber at
# $MOXYGEN_DIR/moxygen/moqtest/moqtest_client, which is the build-tree layout;
# a symlink tree points it at the installed binary instead.
SHIM=$(mktemp -d)
mkdir -p "$SHIM/moxygen/moqtest"
ln -sf "$MOQTEST_CLIENT" "$SHIM/moxygen/moqtest/moqtest_client"
export MOXYGEN_DIR="$SHIM"

echo "Running conformance suite ($TRANSPORT, moqt-$DRAFT) ..."
echo "---"
# Run from LOG_DIR so the suite's own timestamped report lands there, not in the repo.
( cd "$LOG_DIR" && bash "$CONFORMANCE" "$RELAY_URL" ${SUITE_TRANSPORT[@]+"${SUITE_TRANSPORT[@]}"} "$DRAFT" ) 2>&1 | tee "$SUITE_LOG"

# tee masks the suite's status, so read the verdict back out of its output.
if grep -q "ALL TESTS PASSED" "$SUITE_LOG"; then
  exit 0
fi
echo "FAILED cases:"
grep "✗ FAILED" "$SUITE_LOG" | sed 's/\x1b\[[0-9;]*m//g' | head -20
exit 1
