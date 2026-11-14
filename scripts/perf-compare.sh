#!/usr/bin/env bash
#
# perf-compare.sh: does the observability layer change the relay's performance?
#
# Builds the relay twice in release mode, without the `observability` feature
# (the baseline) and with it, then runs each scenario of the scenario file against
# both builds, alternating, several times. The load comes from our own client:
# publishers stamp each object with its send time (--timestamp) and subscriber
# load runs (--sessions, --report) measure throughput and end-to-end latency on
# the client side, so the relay never grades itself. While the observability
# build runs, the script also polls its API the way Prometheus and a dashboard
# would.
#
# It prints the median and spread of every measure per build, and checks the
# pass criteria: throughput and latency within the baseline's spread, and relay
# CPU time at most 2% higher. All raw reports stay in the output folder.
#
# Usage: scripts/perf-compare.sh [options]
#   --runs N          Runs per build and scenario (default: 5)
#   --duration S      Seconds each run lasts (default: 20)
#   --scenarios FILE  Scenario file (default: scripts/perf-scenarios.conf)
#   --only NAME       Run only the scenario with this name
#   --out DIR         Output folder (default: /tmp/moqtail-perf-compare-<time>)
#   --skip-build      Reuse the two relay builds from the previous run
#   --port N          Relay port (default: 4500); the API uses port + 80
#   --cert PATH / --key PATH
#                     Relay TLS certificate and key (default: apps/relay/cert/*.pem;
#                     see apps/relay/cert/README.md to create them)
#
# Publishers, subscribers and the relay all run on this machine, so latency is
# measured on one clock.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
RUNS=5
DURATION=20
SCENARIOS="$ROOT/scripts/perf-scenarios.conf"
ONLY=""
OUT="/tmp/moqtail-perf-compare-$(date +%Y%m%d-%H%M%S)"
SKIP_BUILD=0
PORT=4500
CERT="$ROOT/apps/relay/cert/cert.pem"
KEY="$ROOT/apps/relay/cert/key.pem"

while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs) RUNS="$2"; shift 2 ;;
    --duration) DURATION="$2"; shift 2 ;;
    --scenarios) SCENARIOS="$2"; shift 2 ;;
    --only) ONLY="$2"; shift 2 ;;
    --out) OUT="$2"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    --port) PORT="$2"; shift 2 ;;
    --cert) CERT="$2"; shift 2 ;;
    --key) KEY="$2"; shift 2 ;;
    -h|--help) sed -n '2,32p' "$0"; exit 0 ;;
    *) echo "unknown option: $1" >&2; exit 2 ;;
  esac
done

BIN="$ROOT/target/perf-compare"
API_PORT=$((PORT + 80))
for f in "$CERT" "$KEY"; do
  [[ -r "$f" ]] || { echo "cannot read $f; pass --cert/--key or see apps/relay/cert/README.md" >&2; exit 1; }
done
mkdir -p "$OUT" "$BIN"
echo "Output: $OUT"

# ── Builds ──────────────────────────────────────────────────────────────────────
if [[ $SKIP_BUILD -eq 0 ]]; then
  echo "Building the baseline relay (no observability)..."
  cargo build --release -q -p relay --no-default-features --manifest-path "$ROOT/Cargo.toml"
  cp "$ROOT/target/release/relay" "$BIN/relay-baseline"
  echo "Building the relay with observability..."
  cargo build --release -q -p relay --manifest-path "$ROOT/Cargo.toml"
  cp "$ROOT/target/release/relay" "$BIN/relay-observability"
  echo "Building the client..."
  cargo build --release -q -p client --manifest-path "$ROOT/Cargo.toml"
  cp "$ROOT/target/release/client" "$BIN/client"
fi
for b in relay-baseline relay-observability client; do
  [[ -x "$BIN/$b" ]] || { echo "missing $BIN/$b; run without --skip-build" >&2; exit 1; }
done

# ── Relay configurations ────────────────────────────────────────────────────────
cat > "$OUT/baseline.yaml" <<EOF
bind: '127.0.0.1:$PORT'
cert_file: $CERT
key_file: $KEY
EOF
cat > "$OUT/observability.yaml" <<EOF
bind: '127.0.0.1:$PORT'
cert_file: $CERT
key_file: $KEY
api:
  enable: true
  bind: 127.0.0.1:$API_PORT
  features: [health, state, metrics, latency, qlog, events]
EOF

PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}
trap cleanup EXIT

# Samples a process's CPU time and resident memory once a second until it exits.
# Each line: <cpu time as ps prints it> <rss in KiB>
sample() {
  local pid=$1 file=$2
  while kill -0 "$pid" 2>/dev/null; do
    ps -o time=,rss= -p "$pid" >> "$file" 2>/dev/null || true
    sleep 1
  done
}

# Polls the API like Prometheus (every second) and a dashboard (every 2 seconds),
# and keeps a live event stream open, until the relay exits.
api_load() {
  local relay_pid=$1 base="http://127.0.0.1:$API_PORT/api/v1"
  curl -sN "$base/events" > /dev/null 2>&1 &
  local events_pid=$!
  local tick=0
  while kill -0 "$relay_pid" 2>/dev/null; do
    curl -s -o /dev/null "$base/metrics" || true
    if (( tick % 2 == 0 )); then
      for page in overview connections latency; do curl -s -o /dev/null "$base/$page" || true; done
    fi
    tick=$((tick + 1))
    sleep 1
  done
  kill "$events_pid" 2>/dev/null || true
}

# One run: relay, publishers, subscriber load processes; waits for the load to end.
# Arguments: build scenario run name publishers subscribers ramp interval payload
#            objects_per_group delivery transport
run_once() {
  local build=$1 run=$2 name=$3 publishers=$4 subscribers=$5 ramp=$6 interval=$7
  local payload=$8 per_group=$9 delivery=${10} transport=${11}
  local dir="$OUT/$name/$build/run$run"
  mkdir -p "$dir"
  PIDS=()

  RUST_LOG=warn "$BIN/relay-$build" --config "$OUT/$build.yaml" > "$dir/relay.log" 2>&1 &
  local relay=$!
  PIDS+=("$relay")
  sleep 1
  if ! kill -0 "$relay" 2>/dev/null; then
    echo "the $build relay did not start; its log:" >&2
    tail -5 "$dir/relay.log" >&2
    exit 1
  fi
  sample "$relay" "$dir/relay.samples" &
  [[ $build == observability ]] && { api_load "$relay" & }

  # Enough groups that publishing outlasts the run.
  local groups=$(( (DURATION + 10) * 1000 / (interval * per_group) + 1 ))
  for p in $(seq 1 "$publishers"); do
    RUST_LOG=warn "$BIN/client" -c publish -s "moqt://127.0.0.1:$PORT" --transport "$transport" \
      --no-cert-validation -T "track$p" --timestamp --delivery-mode "$delivery" \
      --group-count "$groups" --objects-per-group "$per_group" -i "$interval" \
      --payload-size "$payload" > "$dir/publisher$p.log" 2>&1 &
    PIDS+=("$!")
  done
  # Subscribers ask only once their track exists.
  sleep 2

  local subs=()
  for p in $(seq 1 "$publishers"); do
    RUST_LOG=warn "$BIN/client" -c subscribe -s "moqt://127.0.0.1:$PORT" --transport "$transport" \
      --no-cert-validation -T "track$p" --delivery-mode "$delivery" \
      --sessions "$subscribers" --ramp "$ramp" --duration "$DURATION" \
      --report "$dir/subscriber$p.json" > "$dir/subscriber$p.log" 2>&1 &
    subs+=("$!")
    sample "$!" "$dir/subscriber$p.samples" &
  done
  for pid in "${subs[@]}"; do wait "$pid" || true; done

  for pid in "${PIDS[@]}"; do kill "$pid" 2>/dev/null || true; done
  wait 2>/dev/null || true
}

# ── Runs ────────────────────────────────────────────────────────────────────────
while read -r name publishers subscribers ramp interval payload per_group delivery transport; do
  [[ -z "$name" || "$name" == \#* ]] && continue
  [[ -n "$ONLY" && "$name" != "$ONLY" ]] && continue
  echo "Scenario $name: $publishers publisher(s), $subscribers subscriber(s) each, $transport, $delivery"
  for run in $(seq 1 "$RUNS"); do
    # Alternate which build goes first, so drift over time hits both equally.
    if (( run % 2 == 1 )); then order=(baseline observability); else order=(observability baseline); fi
    for build in "${order[@]}"; do
      echo "  run $run/$RUNS: $build"
      run_once "$build" "$run" "$name" "$publishers" "$subscribers" "$ramp" "$interval" \
        "$payload" "$per_group" "$delivery" "$transport"
    done
  done
done < "$SCENARIOS"

# ── Summary ─────────────────────────────────────────────────────────────────────
python3 - "$OUT" "$DURATION" <<'PY'
import json, os, statistics, sys

out, duration = sys.argv[1], int(sys.argv[2])
cores = os.cpu_count() or 1

def cpu_seconds(text):
    # ps prints [[dd-]hh:]mm:ss[.cc]
    days = 0
    if "-" in text:
        d, text = text.split("-", 1)
        days = int(d)
    parts = [float(p) for p in text.split(":")]
    seconds = 0.0
    for p in parts:
        seconds = seconds * 60 + p
    return days * 86400 + seconds

def last_sample(path):
    try:
        lines = [l.split() for l in open(path) if l.strip()]
    except FileNotFoundError:
        return None, None
    if not lines:
        return None, None
    return cpu_seconds(lines[-1][0]), max(int(l[1]) for l in lines)

def run_measures(run_dir):
    reports = [json.load(open(os.path.join(run_dir, f)))
               for f in sorted(os.listdir(run_dir)) if f.startswith("subscriber") and f.endswith(".json")]
    if not reports:
        return None
    cpu, rss = last_sample(os.path.join(run_dir, "relay.samples"))
    client_cpu = [last_sample(os.path.join(run_dir, f))[0] or 0
                  for f in os.listdir(run_dir) if f.startswith("subscriber") and f.endswith(".samples")]
    latency = [r["latency_us"] for r in reports if r["latency_us"]["count"]]
    return {
        "objects_per_s": sum(r["objects_per_s"] for r in reports),
        "sessions_ok": sum(r["sessions"]["connected"] - r["sessions"]["ended_with_error"] for r in reports),
        "p50_us": statistics.mean(l["p50"] for l in latency) if latency else None,
        "p99_us": max(l["p99"] for l in latency) if latency else None,
        "relay_cpu_s": cpu,
        "relay_rss_kib": rss,
        # A loaded client distorts what it measures, and a run in which no session
        # received anything measured nothing; neither is counted.
        "client_valid": all(c / duration < 0.7 * cores for c in client_cpu)
                        and sum(r["objects"] for r in reports) > 0,
    }

def spread(values):
    values = [v for v in values if v is not None]
    if not values:
        return None
    return {"median": statistics.median(values), "min": min(values), "max": max(values)}

summary, failed = {}, False
for scenario in sorted(os.listdir(out)):
    sdir = os.path.join(out, scenario)
    if not os.path.isdir(sdir):
        continue
    builds = {}
    for build in ("baseline", "observability"):
        bdir = os.path.join(sdir, build)
        runs = [m for m in (run_measures(os.path.join(bdir, r)) for r in sorted(os.listdir(bdir))) if m]
        invalid = sum(1 for m in runs if not m["client_valid"])
        runs = [m for m in runs if m["client_valid"]]
        builds[build] = {k: spread([m[k] for m in runs])
                         for k in ("objects_per_s", "sessions_ok", "p50_us", "p99_us", "relay_cpu_s", "relay_rss_kib")}
        builds[build]["runs"], builds[build]["invalid_runs"] = len(runs), invalid
    b, o = builds["baseline"], builds["observability"]
    checks = {}
    def within(key, higher_is_worse):
        if not b[key] or not o[key]:
            return None
        bad = o[key]["median"] > b[key]["max"] if higher_is_worse else o[key]["median"] < b[key]["min"]
        return not bad
    checks["throughput"] = None if not (b["objects_per_s"] and o["objects_per_s"]) else bool(
        within("objects_per_s", False) or
        o["objects_per_s"]["median"] >= 0.99 * b["objects_per_s"]["median"])
    checks["latency_p50"] = within("p50_us", True)
    checks["latency_p99"] = within("p99_us", True)
    checks["relay_cpu"] = None if not (b["relay_cpu_s"] and o["relay_cpu_s"]) else (
        o["relay_cpu_s"]["median"] <= 1.02 * b["relay_cpu_s"]["median"])
    summary[scenario] = {"builds": builds, "checks": checks}

    print(f"\n{scenario}  (runs: baseline {b['runs']}, observability {o['runs']}; "
          f"invalid: {b['invalid_runs']} / {o['invalid_runs']})")
    print(f"  {'measure':<16}{'baseline median [min..max]':>34}{'observability median [min..max]':>38}")
    for key in ("objects_per_s", "sessions_ok", "p50_us", "p99_us", "relay_cpu_s", "relay_rss_kib"):
        def fmt(s):
            return "-" if not s else f"{s['median']:.1f} [{s['min']:.1f}..{s['max']:.1f}]"
        print(f"  {key:<16}{fmt(b[key]):>34}{fmt(o[key]):>38}")
    for name, ok in checks.items():
        print(f"  {name:<16}{'PASS' if ok else ('no data' if ok is None else 'FAIL')}")
        # A check without data is a failure too: the comparison did not happen.
        failed |= not ok

json.dump(summary, open(os.path.join(out, "summary.json"), "w"), indent=2)
print(f"\nSummary: {os.path.join(out, 'summary.json')}")
sys.exit(1 if failed else 0)
PY
