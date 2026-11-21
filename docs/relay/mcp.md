---
description: Let an AI agent such as Claude Code read the relay's state, and optionally act on it, through the relay's MCP server.
---

# Connect an AI agent

The relay's HTTP API includes a [Model Context Protocol](https://modelcontextprotocol.io) server at `/api/v1/mcp`. Connect an agent to it to ask about the relay in plain words: who is connected, why a subscription failed, which tracks are slow. With write access, the agent can also move or close connections.

::: warning Privacy
The agent sees namespaces, track names and whatever peers send in control messages. All of it goes to the agent's model provider.
:::

## What the agent can use

The server offers a tool only when the API feature behind it is on:

| Tools                                                                                                                             | Needs                            |
| --------------------------------------------------------------------------------------------------------------------------------- | -------------------------------- |
| `relay_overview`, `list_connections`, `get_connection`, `list_tracks`, `get_track`, `namespaces`, `upstreams`, `zones`, `cluster` | `state`                          |
| `latency_report`, `slowlog`                                                                                                       | `latency`                        |
| `connection_timeline`, the control messages of one connection                                                                     | `qlog`, and `qlog_folder` set    |
| `send_goaway`, `close_connection`, `drain_relay`, `set_log_filter`, `set_redirect_uri`                                            | `actions`, and `mcp_write: true` |

The first three rows only read. The last row changes the relay. [HTTP API](/relay/reference/http-api) describes each tool.

## 1. Make a token

The MCP server takes a token of its own. Make one with the relay binary:

```sh
relay api-token
```

It prints the token and the line that goes in the config file:

```
Token (shown once; give it to the MCP client): mcp_640c75c5...
Under api: in the relay's configuration:
  mcp_token_hash: "sha256d$c36d3d22...$5e77569e..."
```

With the container image, run `docker run --rm ghcr.io/moqtail/relay:latest api-token` instead.

The relay stores only the salted hash, so keep the token: it cannot be shown again. If you lose it, make a new one. To hash a token you already have, pipe it into `relay api-token --hash`.

The MCP token works only at `/api/v1/mcp`, and the read and write tokens do not work there.

## 2. Turn on the MCP server

Add `mcp` to the API's features, and paste the `mcp_token_hash` line under `api`:

```yaml
api:
  enable: true
  features: [health, state, metrics, latency, qlog, mcp]
  mcp_token_hash: 'sha256d$c36d3d22...$5e77569e...'
  mcp_name: relay-eu-1
```

`features` replaces the default list, so it names the defaults too. `mcp_name` is how the relay introduces itself to the agent, so an agent connected to several relays can tell them apart. It defaults to the machine's host name.

Restart the relay. Without `mcp_token_hash`, the relay refuses to start:

```
Error: the `mcp` API feature needs `api.mcp_token_hash`; make one with `relay api-token`
```

## 3. Connect the agent

The server uses MCP's Streamable HTTP transport, at `http://127.0.0.1:4480/api/v1/mcp`, with the token in an `Authorization: Bearer` header. It does not use OAuth.

For Claude Code:

```sh
claude mcp add --transport http moqtail-relay http://127.0.0.1:4480/api/v1/mcp \
  --header "Authorization: Bearer mcp_640c75c5..."
```

In Claude Code, `/mcp` then lists the server and its tools. Ask, for example, "Which connections are subscribed to the slowest track?".

Other clients take the same three things: the transport, the URL and the header. A client that reads a JSON file, such as a project's `.mcp.json` in Claude Code, takes:

```json
{
  "mcpServers": {
    "moqtail-relay": {
      "type": "http",
      "url": "http://127.0.0.1:4480/api/v1/mcp",
      "headers": { "Authorization": "Bearer mcp_640c75c5..." }
    }
  }
}
```

If the client cannot connect, try the token with curl:

```sh
curl http://127.0.0.1:4480/api/v1/mcp \
  -H "Authorization: Bearer $MCP_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
```

A working token gets an answer that names the relay, `"title":"MOQtail relay relay-eu-1"`. A missing token gets `401 token_required`, a wrong one `403 token_rejected`.

## Reach a relay on another machine

The API listens on loopback by default, which is the safest place for it. To reach it from your machine, forward the port over SSH and connect the agent to `http://127.0.0.1:4480/api/v1/mcp` as above:

```sh
ssh -N -L 4480:127.0.0.1:4480 relay.example.com
```

On a loopback address, the MCP server also checks the `Host` header, and accepts only `localhost`, `127.0.0.1` and `::1`.

You can instead bind the API to a network address, as [Monitor the relay](/relay/observability#open-the-api-to-the-network) shows. `allow` and the token then decide who gets in. The API speaks plain HTTP, so the token crosses the network in the clear unless a reverse proxy adds TLS.

## Browser origins

A web page could reach an MCP server on your machine through DNS rebinding. To stop that, the MCP server refuses every request whose `Origin` header is not in `cors_origins`, with `403 origin_not_allowed`. Command-line agents send no `Origin`, so this does not affect them. To use an agent that runs in a browser, add its origin:

```yaml
api:
  cors_origins: [https://agent.example.com]
```

## Let the agent change the relay

The write tools send GOAWAY to a connection, close one, drain the relay, change the log filter and change the redirect URI. They need the `actions` feature, which needs a `write_token`, and `mcp_write`:

```yaml
api:
  enable: true
  features: [health, state, metrics, latency, qlog, actions, mcp]
  write_token: '<a long random string>'
  mcp_token_hash: 'sha256d$c36d3d22...$5e77569e...'
  mcp_write: true
```

The agent still uses its MCP token, not the write token. `mcp_write` without `actions` has no effect, and the relay warns about it at startup.

The write tools are marked destructive, so clients that honor the mark ask you before running one. A drain lasts until the relay restarts. The relay logs every change an agent makes, such as `API action from MCP: closed connection 7`.

## More information

- [HTTP API](/relay/reference/http-api), with every MCP tool
- [Monitor the relay](/relay/observability)
- [Run the relay in production](/relay/production), on GOAWAY and draining
- [Configuration file](/relay/reference/config)
