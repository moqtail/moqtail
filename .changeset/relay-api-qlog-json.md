---
'relay-api': minor
---

`GET /api/v1/qlog/{conn_id}` and `/quic` return the trace as one qlog JSON document (`application/qlog+json`, the events in one array) when `Accept` asks for `application/json` or `application/qlog+json`. Anything else still gets JSON-SEQ, which `?follow=true` needs.
