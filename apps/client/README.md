# MOQtail Test Client

A unified test client for the MOQtail relay, supporting publish, subscribe, and fetch operations over MoQ Transport (draft-14). Supports both subgroup (unidirectional streams) and datagram object forwarding preferences.

## Usage

```
cargo run --bin client -- --command <COMMAND> [OPTIONS]
```

## Commands

| Command     | Description                               |
| ----------- | ----------------------------------------- |
| `publish`   | Publish objects to a track                |
| `subscribe` | Subscribe to a track and receive objects  |
| `fetch`     | Fetch specific object ranges from a track |

## Global Options

| Option                    | Short | Default                  | Description                                             |
| ------------------------- | ----- | ------------------------ | ------------------------------------------------------- |
| `--command`               | `-c`  | _(required)_             | Command to run                                          |
| `--server`                | `-s`  | `https://127.0.0.1:4433` | Server address                                          |
| `--namespace`             | `-n`  | `moqtail`                | Track namespace                                         |
| `--track-name`            | `-T`  | `demo`                   | Track name                                              |
| `--no-cert-validation`    |       | `false`                  | Skip certificate validation                             |
| `--forwarding-preference` |       | `subgroup`               | Object forwarding preference (`subgroup` or `datagram`) |

## Publish Options

| Option                | Short | Default     | Description                              |
| --------------------- | ----- | ----------- | ---------------------------------------- |
| `--publish-mode`      |       | `proactive` | `proactive` or `reactive` (see below)    |
| `--group-count`       |       | `100`       | Number of groups to send                 |
| `--interval`          | `-i`  | `1000`      | Interval between objects in milliseconds |
| `--objects-per-group` |       | `10`        | Number of objects per group              |
| `--payload-size`      |       | `1200`      | Payload size in bytes                    |
| `--track-alias`       |       | _(random)_  | Track alias (random if not specified)    |

### Publish Modes

- **Proactive**: Sends a `Publish` message to the relay, waits for `PublishOk`, then immediately starts sending data.
- **Reactive**: Sends a `PublishNamespace` message, then waits for a `Subscribe` from the relay. Responds with `SubscribeOk` and starts sending data.

## Subscribe Options

| Option       | Short | Default | Description                                    |
| ------------ | ----- | ------- | ---------------------------------------------- |
| `--duration` | `-d`  | `0`     | Duration to listen in seconds (0 = indefinite) |

## Fetch Options

| Option           | Default | Description     |
| ---------------- | ------- | --------------- |
| `--start-group`  | `1`     | Start group ID  |
| `--start-object` | `0`     | Start object ID |
| `--end-group`    | `5`     | End group ID    |
| `--end-object`   | `3`     | End object ID   |

## Examples

Publish 50 groups of datagrams at 500ms intervals:

```
cargo run --bin client -- -c publish --forwarding-preference datagram --group-count 50 --interval 500
```

Publish via subgroup streams in reactive mode:

```
cargo run --bin client -- -c publish --forwarding-preference subgroup --publish-mode reactive
```

Subscribe to datagrams for 30 seconds:

```
cargo run --bin client -- -c subscribe --forwarding-preference datagram --duration 30
```

Subscribe to subgroup streams indefinitely:

```
cargo run --bin client -- -c subscribe
```

Fetch objects from groups 0-10:

```
cargo run --bin client -- -c fetch --start-group 0 --end-group 10
```

Connect to a remote server with certificate validation disabled:

```
cargo run --bin client -- -c publish --server https://relay.example.com:4433 --no-cert-validation
```
