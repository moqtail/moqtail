---
'relay': minor
'relay-api': minor
---

An MCP server at `/api/v1/mcp` (`mcp` feature) so AI agents can query the relay, including a connection's message timeline, and with `api.mcp_write` use the operator actions. It speaks the 2026-07-28 and 2025 protocol revisions. It has its own token, which the relay keeps only as a salted, double SHA-256 hash made with `relay api-token`.
