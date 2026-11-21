---
description: Run the relay for real traffic, with logs, graceful shutdown, moving clients to another relay and UDP socket tuning.
---

# Run the relay in production

Use this page once the relay works and you are putting it in front of real viewers. It assumes you run the [container image](/relay/install) with a trusted [certificate](/relay/certificates).

## Run the container

Run it in the background, with a restart policy and the files mounted read-only:

```sh
docker run -d --name moqtail-relay \
  --restart unless-stopped \
  --stop-timeout 15 \
  -p 4433:4433/udp \
  -e NO_COLOR=1 \
  -v /etc/moqtail/cert.pem:/certs/cert.pem:ro \
  -v /etc/moqtail/key.pem:/certs/key.pem:ro \
  -v /etc/moqtail/relay.yaml:/relay.yaml:ro \
  ghcr.io/moqtail/relay:0.19.0 \
  --config /relay.yaml --cert-file /certs/cert.pem --key-file /certs/key.pem
```

- `--restart unless-stopped` brings the relay back after a crash or a reboot.
- `--stop-timeout 15` gives the relay time to drain. It exits 10 seconds after `docker stop`, and Docker kills a container that is still running after 10 seconds by default.
- A release tag such as `0.19.0`, rather than `latest`, keeps the version fixed until you change it.

The relay only reads its config file and certificate, so `:ro` mounts are enough. Settings that write files, such as `qlog_folder`, need a writable mount for that folder.

The relay reads its configuration and certificate once, at startup. After you change the file or renew the certificate, restart it with `docker restart moqtail-relay`, which drains it first.

## Read the logs

The relay writes its log to standard output, one line per event, at level `INFO` and above. Errors that stop it at startup, such as an invalid config file, go to standard error. Read both with:

```sh
docker logs -f moqtail-relay
```

`RUST_LOG` sets the levels, for the whole relay or per module. It takes the directives of Rust's `tracing` `EnvFilter`. For example, add this to `docker run` to see the mesh in detail:

```sh
-e RUST_LOG=info,relay::server::cluster=debug
```

The lines are colored. `NO_COLOR=1`, as in the command above, turns the colors off for log collectors.

With the HTTP API's `actions` feature, you can change the levels without a restart. The `actions` feature needs a `write_token`; [Monitor the relay](/relay/observability) shows how to turn it on:

```sh
curl -X PUT http://127.0.0.1:4480/api/v1/logging \
  -H "Authorization: Bearer $WRITE_TOKEN" \
  -H 'Content-Type: application/json' \
  -d '{"filter": "info,relay::server::cluster=debug"}'
```

## Shut down gracefully

Stop the relay with SIGTERM, which is what `docker stop`, systemd and Kubernetes send. The relay then:

1. Refuses new requests with the error `GOING_AWAY`. `/api/v1/ready` answers `503` with `{"ready":false,"reason":"draining"}`, so a load balancer stops sending clients.
2. Sends GOAWAY to every session, clients and mesh peers alike, with a timeout of 10 seconds and the `redirect_uri` if one is set.
3. Exits after 10 seconds, ending the sessions still open.

The log shows each step:

```
SIGTERM received, draining (GOAWAY, timeout 10000ms)...
Broadcasting GOAWAY (uri: None) to 3 client(s)
```

Ctrl-C (SIGINT) exits at once, without GOAWAY. Pressing Ctrl-C during a drain cuts the drain short.

## Move clients to another relay

GOAWAY asks a client to open a new session, at the URI it carries or, without one, at the same URI. The `moqtail` player follows it and reconnects. To move clients off this relay before maintenance:

1. Name the relay they should move to, in the config file:

   ```yaml
   redirect_uri: moqt://relay-2.example.com:4433
   ```

   Or set it at runtime with the API's `actions` feature:

   ```sh
   curl -X PUT http://127.0.0.1:4480/api/v1/config/redirect_uri \
     -H "Authorization: Bearer $WRITE_TOKEN" \
     -H 'Content-Type: application/json' \
     -d '{"uri": "moqt://relay-2.example.com:4433"}'
   ```

2. Drain the relay. SIGTERM gives clients 10 seconds. To give them longer and keep the relay running, drain it through the API, with a timeout of up to 600000 ms:

   ```sh
   curl -X POST "http://127.0.0.1:4480/api/v1/drain?timeout_ms=60000" \
     -H "Authorization: Bearer $WRITE_TOKEN"
   ```

   It answers with how many sessions it told, such as `{"told":42,"timeout_ms":60000}`. The relay refuses new requests from then on, and closes the connections still open when the timeout ends. It stays draining until it restarts.

3. Watch the connections leave with `curl http://127.0.0.1:4480/api/v1/connections`, then restart or stop the relay.

`POST /api/v1/connections/<conn_id>/goaway` moves a single connection the same way.

## Tune UDP throughput

All media goes through the relay's UDP socket, so the socket's buffers and the number of sockets set how much traffic it can take.

### Socket buffers

`socket_buffer_kib` asks the kernel for a send and a receive buffer of that size, per socket. The default is 8192 KiB. A large buffer absorbs the bursts of fan-out to many subscribers. `0` leaves the kernel's default.

The kernel caps the buffers at `net.core.wmem_max` and `net.core.rmem_max`. When they are lower than the relay asks for, it warns at startup:

```
UDP socket buffers are 7324 KiB send / 7324 KiB receive, below the 8192 KiB asked for; raise net.core.wmem_max and net.core.rmem_max to allow more
```

Raise the limits to the size the relay asks for, here 8 MiB:

```sh
sudo sysctl -w net.core.rmem_max=8388608 net.core.wmem_max=8388608
```

To keep them after a reboot, put the same two settings in a file under `/etc/sysctl.d/`. They apply to the whole machine, so for a container, set them on the host.

### More than one socket

On Linux, `io_sockets` opens several UDP sockets on the same port with `SO_REUSEPORT`. The kernel spreads connections across them, so they no longer contend for one socket. Use up to one per CPU core; more only add idle sockets:

```yaml
io_sockets: 4
```

The startup line shows the count: `running on 4 UDP socket(s)`. Values outside 1 to 256 are clamped. On other systems the relay warns and uses one socket.

### Request streams

Every MOQT request, such as a SUBSCRIBE or a FETCH, takes a QUIC stream. `max_request_streams`, 10000 by default, is how many a single connection may have open at once. Raise it when one connection carries many requests, as a link between mesh peers does. To cap load relay-wide instead, see the limits in [Configure the relay](/relay/configure).

## More information

- [Configuration file](/relay/reference/config)
- [Relay CLI](/relay/reference/cli)
- [Monitor the relay](/relay/observability)
- [Build a relay mesh](/relay/clustering)
