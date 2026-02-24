#!/usr/bin/env bash
#
# Test commands for the MOQtail client app.
# Usage: ./scripts/test-client.sh <subcommand> [--server <url>]
#
# Set SERVER env var or use --server flag to override the default relay address.

set -euo pipefail

DEFAULT_SERVER="https://127.0.0.1:4433"
SERVER="${SERVER:-$DEFAULT_SERVER}"
NAMESPACE="moqtail"
TRACK="demo"

# Parse global --server / -s flag before the subcommand
ARGS=("$@")
SUBCMD=""



for i in "${!ARGS[@]}"; do
  case "${ARGS[$i]}" in
    --server|-s)
      SERVER="${ARGS[$((i+1))]}"
      unset 'ARGS[$i]' 'ARGS[$((i+1))]'
      ;;
  esac
done

# Re-index after deletions
if [ ${#ARGS[@]} -eq 0 ]; then
  SUBCMD="help"
else
  SUBCMD="${ARGS[0]:-help}"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
BIN="$ROOT_DIR/target/debug/client"

COMMON="--server $SERVER --namespace $NAMESPACE --track-name $TRACK --no-cert-validation"

cleanup() {
  # Kill background jobs on exit
  jobs -p 2>/dev/null | xargs -r kill 2>/dev/null || true
}
trap cleanup EXIT

build() {
  echo "Building client..."
  cargo build -p client --manifest-path "$ROOT_DIR/Cargo.toml"
}

run_client() {
  $BIN $COMMON "$@"
}

case "$SUBCMD" in

  ## ── Publish ──────────────────────────────────────────────

  publish-subgroup)
    build
    echo "Publishing via subgroup streams (default settings)"
    run_client -c publish --forwarding-preference subgroup
    ;;

  publish-datagram)
    build
    echo "Publishing via datagrams"
    run_client -c publish --forwarding-preference datagram
    ;;

  publish-fast)
    build
    echo "Publishing fast: 10ms interval, 5 objects/group, 50 groups"
    run_client -c publish --interval 10 --objects-per-group 5 --group-count 50
    ;;

  publish-large)
    build
    echo "Publishing with large payloads (65536 bytes)"
    run_client -c publish --payload-size 65536 --group-count 20 --objects-per-group 5
    ;;

  publish-long)
    build
    echo "Publishing long run: 1000 groups, 20 objects/group"
    run_client -c publish --group-count 1000 --objects-per-group 20 --interval 100
    ;;

  publish-namespace)
    build
    echo "Publishing namespace (auto-respond to subscribes)"
    run_client -c publish-namespace
    ;;

  ## ── Subscribe ────────────────────────────────────────────

  subscribe-subgroup)
    build
    echo "Subscribing via subgroup streams"
    run_client -c subscribe --forwarding-preference subgroup
    ;;

  subscribe-datagram)
    build
    echo "Subscribing via datagrams"
    run_client -c subscribe --forwarding-preference datagram
    ;;

  subscribe-timed)
    build
    echo "Subscribing for 30 seconds"
    run_client -c subscribe --duration 30
    ;;

  ## ── Fetch ────────────────────────────────────────────────

  fetch)
    build
    echo "Fetching groups 1-5, objects 0-3"
    run_client -c fetch --start-group 1 --start-object 0 --end-group 5 --end-object 3
    ;;

  fetch-cancel)
    build
    echo "Fetching with cancel after 5 objects"
    run_client -c fetch --start-group 1 --start-object 0 --end-group 10 --end-object 9 --cancel-after 5
    ;;

  ## ── Combined pub+sub ─────────────────────────────────────

  pub-sub)
    build
    echo "Running publisher + subscriber (subgroup)"
    run_client -c publish --forwarding-preference subgroup &
    sleep 1
    run_client -c subscribe --forwarding-preference subgroup --duration 15
    ;;

  pub-sub-datagram)
    build
    echo "Running publisher + subscriber (datagram)"
    run_client -c publish --forwarding-preference datagram &
    sleep 1
    run_client -c subscribe --forwarding-preference datagram --duration 15
    ;;

  ## ── Help ─────────────────────────────────────────────────

  help|--help|-h|"")
    cat <<'HELP'
MOQtail Client Test Commands

Usage: ./scripts/test-client.sh <subcommand> [--server <url>]

Environment: SERVER=<url>  (default: https://127.0.0.1:4433)

Publish:
  publish-subgroup      Publish via subgroup streams (default settings)
  publish-datagram      Publish via datagrams
  publish-fast          Publish fast (10ms interval, small groups)
  publish-large         Publish with large payloads (64KB)
  publish-long          Publish many groups (1000 groups, 20 obj/group)
  publish-namespace     Publish namespace (auto-respond to subscribes)

Subscribe:
  subscribe-subgroup    Subscribe to subgroup-based track
  subscribe-datagram    Subscribe to datagram-based track
  subscribe-timed       Subscribe with 30-second time limit

Fetch:
  fetch                 Fetch object range (groups 1-5, objects 0-3)
  fetch-cancel          Fetch with cancel after 5 objects

Combined:
  pub-sub               Publisher + subscriber in parallel (subgroup)
  pub-sub-datagram      Publisher + subscriber in parallel (datagram)

HELP
    ;;

  *)
    echo "Unknown subcommand: $SUBCMD"
    echo "Run './scripts/test-client.sh help' for available commands."
    exit 1
    ;;
esac
