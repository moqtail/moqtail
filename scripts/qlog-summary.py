#!/usr/bin/env python3
# Copyright 2026 The MOQtail Authors
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

"""Summarizes moqtail qlog traces, one block per connection.

    scripts/qlog-summary.py <folder or .sqlog files>... [--raw]

For each MoQT trace: the peer's SETUP, every request with its responses, data
streams per track, and control bytes that failed to parse. `<-` is from the peer,
`->` is to the peer. --raw adds each control message's bytes in hex. QUIC packet
traces (*_quic.sqlog) get one line each.
"""

import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

REQUESTS = {
    "subscribe", "publish", "fetch", "track_status", "publish_namespace",
    "subscribe_namespace", "subscribe_tracks",
}


def records(path):
    for chunk in path.read_bytes().split(b"\x1e"):
        chunk = chunk.strip()
        if chunk:
            try:
                yield json.loads(chunk)
            except json.JSONDecodeError:
                # The last record of a file still being written may be cut short.
                continue


def text(field):
    if "value" in field:
        return str(field["value"])
    return "0x" + field.get("value_bytes", "")


def tuple_text(fields):
    return "/".join(text(f) for f in fields or [])


def value_text(v):
    if isinstance(v, dict) and set(v) == {"group", "object"}:
        return f"{v['group']}:{v['object']}"
    if isinstance(v, dict):
        return ",".join(f"{k}={value_text(x)}" for k, x in v.items())
    if isinstance(v, list):
        return tuple_text(v)
    return v


def param_text(p):
    name = p.get("name", "?")
    if name == "unknown":
        name = f"0x{p.get('type', 0):x}"
    if "value_bytes" in p and "value" not in p:
        return f"{name}=0x{p['value_bytes'].get('data', '')}"
    return f"{name}={value_text(p.get('value'))}"


def describe(m):
    """One line for a control message."""
    parts = [m["type"].upper()]
    for key in ("track_namespace", "track_namespace_prefix", "track_namespace_suffix"):
        if key in m:
            ns = tuple_text(m[key])
            if "track_name" in m:
                ns += " : " + text(m["track_name"])
            parts.append(ns)
    if "standalone_fetch" in m:
        sf = m["standalone_fetch"]
        parts.append(tuple_text(sf["track_namespace"]) + " : " + text(sf["track_name"]))
    for key in ("request_id", "track_alias", "error_code", "status_code", "stream_count", "timeout"):
        if key in m:
            parts.append(f"{key}={m[key]}")
    if m.get("reason_phrase"):
        parts.append(repr(m["reason_phrase"]))
    if "end_location" in m:
        parts.append("end={group}:{object}".format(**m["end_location"]))
    if m.get("parameters"):
        parts.append("params[" + ", ".join(param_text(p) for p in m["parameters"]) + "]")
    return " ".join(parts)


def setup_text(m):
    opts = []
    for o in m.get("setup_options", []):
        name = o["name"] if o["name"] != "unknown" else f"0x{o.get('type', 0):x}"
        v = o.get("value", o.get("value_bytes", {}).get("data"))
        opts.append(f"{name}={v}")
    return ", ".join(opts) or "(no options)"


def summarize_moqt(path, events, raw):
    header = events[0] if events and "file_schema" in events[0] else {}
    title = header.get("title", path.name)
    events = [e for e in events if "time" in e]
    if not events:
        print(f"== {title}  [{path.name}, no events]")
        return
    start = events[0]["time"]
    span = (events[-1]["time"] - start) / 1000
    print(f"== {title}  [{path.name}, {len(events)} events, {span:.1f}s]")

    by_stream = defaultdict(list)
    subgroups = defaultdict(Counter)
    groups = defaultdict(set)
    fetches = Counter()
    errors = []
    for e in events:
        name, d = e["name"], e["data"]
        arrow = "<-" if name.endswith("_parsed") else "->"
        at = (e["time"] - start) / 1000
        if name.startswith("moqt:control_message"):
            m = d["message"]
            if m["type"] == "setup":
                if arrow == "<-":
                    print(f"  peer SETUP: {setup_text(m)}")
                continue
            by_stream[d["stream_id"]].append((at, arrow, m, d.get("raw", {}).get("data", "")))
        elif name.startswith("moqt:subgroup_header"):
            subgroups[(arrow, d["track_alias"])]["streams"] += 1
            groups[(arrow, d["track_alias"])].add(d["group_id"])
        elif name.startswith("moqt:fetch_header"):
            fetches[(arrow, d["request_id"])] += 1
        elif name == "loglevel:error":
            errors.append((at, d))

    # A request stream carries one request and its responses, which are indented
    # under it. Messages on the control stream stand alone.
    for stream_id, msgs in sorted(by_stream.items(), key=lambda kv: kv[1][0][0]):
        is_request = msgs[0][2]["type"] in REQUESTS
        for i, (at, arrow, m, hexdata) in enumerate(msgs):
            lead = "      " if is_request and i > 0 else "  "
            print(f"{lead}{at:7.3f}s {arrow} {describe(m)}")
            if raw:
                print(f"{lead}          {hexdata}")

    for (arrow, alias), c in sorted(subgroups.items()):
        g = sorted(groups[(arrow, alias)])
        who = "from peer" if arrow == "<-" else "to peer  "
        print(f"  data {who}: track_alias={alias}: {c['streams']} subgroup streams, "
              f"groups {g[0]}..{g[-1]} ({len(g)} distinct)")
    for (arrow, req), n in sorted(fetches.items()):
        who = "from peer" if arrow == "<-" else "to peer  "
        print(f"  data {who}: fetch request_id={req}: {n} stream(s)")
    for at, d in errors:
        r = d.get("raw", {})
        print(f"  {at:7.3f}s !! {d.get('message')} ({r.get('length')} bytes): {r.get('data')}")


def summarize_quic(path, events):
    c = Counter(e.get("name") for e in events if "time" in e)
    metrics = [e["data"] for e in events if e.get("name") == "recovery:metrics_updated"]
    rtt = next((m["smoothed_rtt"] for m in reversed(metrics) if "smoothed_rtt" in m), None)
    title = events[0].get("title", path.name) if events else path.name
    # quinn writes RTT in seconds, not the milliseconds qlog asks for.
    rtt_text = f", smoothed_rtt={rtt * 1000:.2f}ms" if rtt is not None else ""
    print(f"== {title}  [{path.name}]")
    print(f"  QUIC: {c['transport:packet_sent']} sent, {c['transport:packet_received']} received, "
          f"{c['recovery:packet_lost']} lost{rtt_text}")


def main(argv):
    raw = "--raw" in argv
    paths = []
    for a in (a for a in argv if a != "--raw"):
        p = Path(a)
        paths += sorted(p.glob("*.sqlog")) if p.is_dir() else [p]
    # MoQT traces first, QUIC packet traces after them.
    paths.sort(key=lambda p: p.name.endswith("_quic.sqlog"))
    if not paths:
        print(__doc__.strip())
        return 1
    for path in paths:
        events = list(records(path))
        if path.name.endswith("_quic.sqlog"):
            summarize_quic(path, events)
        else:
            summarize_moqt(path, events, raw)
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
