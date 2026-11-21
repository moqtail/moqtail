---
description: The relay's command-line flags, the api-token subcommand, its environment variables, signals and exit codes.
---

# Relay CLI

## Synopsis

```
relay [--config <file>] [--bind <address>] [--cert-file <file>] [--key-file <file>]
relay api-token [--hash]
relay --help
relay --version
```

## Flags

| Flag                 | Default                    | Description                                                                                                |
| -------------------- | -------------------------- | ---------------------------------------------------------------------------------------------------------- |
| `--config <file>`    | none                       | The YAML [configuration file](/relay/reference/config). Without it the relay runs on the defaults.         |
| `--bind <address>`   | `[::]:4433`                | The UDP address for WebTransport and raw QUIC, an IP address and a port. `[::]` takes IPv4 and IPv6 alike. |
| `--cert-file <file>` | `apps/relay/cert/cert.pem` | The TLS certificate chain, in PEM.                                                                         |
| `--key-file <file>`  | `apps/relay/cert/key.pem`  | The TLS private key, in PEM.                                                                               |
| `-h`, `--help`       |                            | Prints the flags and exits.                                                                                |
| `-V`, `--version`    |                            | Prints `relay` and the crate version, such as `relay 0.19.0`, and exits.                                   |

- `--bind`, `--cert-file` and `--key-file` override the keys of the same name in the configuration file. Every other setting is only in the file.
- `--bind` takes an IP address, not a host name: `0.0.0.0:4433`, `[::1]:4433`.
- Relative paths resolve against the directory the relay starts in. The default certificate paths are relative to the repository root.

## `api-token`

Makes a token for the [MCP server](/relay/mcp). Only builds with the HTTP API have it, which the default Cargo feature `observability` includes.

| Command                  | Reads                      | Prints                                                    |
| ------------------------ | -------------------------- | --------------------------------------------------------- |
| `relay api-token`        | nothing                    | A new token, then the `mcp_token_hash` line that holds it |
| `relay api-token --hash` | one line on standard input | The `mcp_token_hash` line for that token                  |

```
$ relay api-token
Token (shown once; give it to the MCP client): mcp_3c9dea56…
Under api: in the relay's configuration:
  mcp_token_hash: "sha256d$87808241…$fd926dc2…"
```

A token is `mcp_` and 64 hexadecimal digits. The hash is salted with random bytes, so hashing one token twice gives two different lines, and either one works. `--hash` trims the line it reads and fails when it is empty.

Both commands exit without starting the relay. The command line has to be exactly one of the two; any other arguments go to the flag parser, which rejects them.

## Environment variables

| Variable   | Description                                                                                                                                                                                                                                                          |
| ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `RUST_LOG` | The log filter, as [`EnvFilter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html) directives: `debug`, `info,relay=trace`. Unset, the level is `info`. A directive that does not parse is skipped, with a note at startup. |
| `NO_COLOR` | Set to any non-empty value, log lines have no ANSI colors.                                                                                                                                                                                                           |

The relay writes its log to standard output. Errors that stop it at startup go to standard error.

With the HTTP API's `actions` feature on, `PUT /api/v1/logging` replaces the filter while the relay runs. See [HTTP API](/relay/reference/http-api#actions).

## Signals

| Signal                    | What the relay does                                                                                                                                                                               |
| ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `SIGINT` (Ctrl-C)         | Exits at once. Clients get no GOAWAY.                                                                                                                                                             |
| `SIGTERM`                 | Drains, then exits after 10 seconds. It sends every client a GOAWAY with a 10000 ms timeout and the `redirect_uri`, and refuses new requests with `GOING_AWAY`. It still accepts new connections. |
| `SIGINT` during the drain | Ends the drain and exits at once.                                                                                                                                                                 |

While the relay drains, `GET /api/v1/ready` answers 503 with `"reason": "draining"`. The 10-second drain is fixed. To drain without exiting, use `POST /api/v1/drain`.

## Exit codes

| Code | When                                                                                                                                                                                                            |
| ---- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 0    | `--help`, `--version` and `api-token`, and a normal exit after `SIGINT` or the `SIGTERM` drain.                                                                                                                 |
| 1    | The relay stopped on an error, printed as `Error: …`: a certificate it cannot read, a port in use, an [`api` setting it refuses](/relay/reference/http-api#startup-refusals), `api-token --hash` with no token. |
| 2    | A command line or configuration file it cannot use, printed as `error: …`: an unknown flag, a missing file, an unknown key, a value out of range.                                                               |
