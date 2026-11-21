---
description: Get a TLS certificate for the relay, with mkcert for local development or from a certificate authority in production.
---

# Set up TLS certificates

The relay needs a TLS certificate and its private key in PEM files, for WebTransport and raw QUIC alike. Use mkcert for a relay on your own machine, and a public certificate authority (CA) for a relay that others connect to.

## Local development

### 1. Make a certificate with mkcert

Install [mkcert](https://github.com/FiloSottile/mkcert#installation). On Linux, install `certutil` too (the `libnss3-tools` package on Debian and Ubuntu), so that mkcert can add its CA to Firefox and Chrome.

```sh
mkcert -install
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 ::1
```

The first command creates a local CA and adds it to your system's and your browsers' trust stores. The second makes `cert.pem` and `key.pem`, valid for `localhost`, `127.0.0.1` and `::1`. The Rust CLI client checks certificates against the system's trust store, so it accepts this one as it is.

### 2. Let the browser use it for WebTransport

Browsers do not use a locally installed CA for WebTransport until you allow it:

- **Chrome**: open `chrome://flags/#webtransport-developer-mode`, enable **WebTransport Developer Mode** and restart Chrome.
- **Firefox**: open `about:config` and set `network.http.http3.disable_when_third_party_roots_found` to `false`.

### 3. Point the relay at the files

Give the relay the two files:

```sh
relay --cert-file cert.pem --key-file key.pem
```

Without the flags, the relay reads `apps/relay/cert/cert.pem` and `apps/relay/cert/key.pem`, relative to the folder it starts in. A relay started from the repository root finds files you make in `apps/relay/cert` without any flag. In a configuration file, the keys are `cert_file` and `key_file`.

The container image reads `/certs/cert.pem` and `/certs/key.pem`. It runs as an unprivileged user, and mkcert makes `key.pem` readable only by you, so add `--user "$(id -u):$(id -g)"` to `docker run`. [Install the relay](/relay/install) has the full command.

## Production

### 1. Get a certificate

Any CA works, for example Let's Encrypt. The relay does not answer ACME challenges itself, so use a client that does, such as `certbot certonly --standalone` (it needs TCP port 80) or one that uses a DNS challenge. The certificate must name the host that clients put in their `moqt://` URL.

The relay loads:

- **The certificate file**: your certificate followed by the intermediate certificates, the full chain. With certbot, that is `fullchain.pem`, not `cert.pem`.
- **The private key file**: an unencrypted RSA or ECDSA key, as PKCS#8 (`BEGIN PRIVATE KEY`), PKCS#1 (`BEGIN RSA PRIVATE KEY`) or SEC1 (`BEGIN EC PRIVATE KEY`). An encrypted key (`BEGIN ENCRYPTED PRIVATE KEY`) stops the relay with `no private key section found in PEM file`.

### 2. Make the key readable by the relay

The container runs as the user `nonroot`, UID 65532, and certbot's keys are readable only by root. Copy both files into a folder of the relay's own, with the key owned by that user:

```sh
sudo install -d /srv/moqtail
sudo install -m 0644 /etc/letsencrypt/live/relay.example.com/fullchain.pem /srv/moqtail/cert.pem
sudo install -m 0400 -o 65532 -g 65532 /etc/letsencrypt/live/relay.example.com/privkey.pem /srv/moqtail/key.pem
```

Then mount the folder at `/certs`:

```sh
docker run -d --name relay \
  -p 4433:4433/udp \
  -v /srv/moqtail:/certs:ro \
  ghcr.io/moqtail/relay:latest
```

For a relay you built from source, give the key to the user the relay runs as instead.

### 3. Restart the relay after each renewal

The relay reads the certificate and key once, when it starts, so a renewed certificate takes effect on the next restart. On SIGTERM, the relay sends GOAWAY to its clients and exits 10 seconds later; [Run the relay in production](/relay/production) covers how clients move to another relay.

With certbot, a deploy hook copies the renewed files and restarts the relay after every renewal. Save it as `/etc/letsencrypt/renewal-hooks/deploy/moqtail.sh` and make it executable:

```sh
#!/bin/sh
install -m 0644 "$RENEWED_LINEAGE/fullchain.pem" /srv/moqtail/cert.pem
install -m 0400 -o 65532 -g 65532 "$RENEWED_LINEAGE/privkey.pem" /srv/moqtail/key.pem
docker restart -t 15 relay
```

`-t 15` gives the relay's 10-second drain time to finish before Docker stops the container by force.

## Certificates between mesh peers

A relay dials its [mesh](/relay/clustering) peers over raw QUIC and checks their certificates against the system's trust store, like any client. Certificates from a public CA pass this check. So do mkcert certificates, for relays that run outside containers on the machine where you ran `mkcert -install`; the container image trusts public CAs only.

`peer_no_cert_validation: true` turns that check off. Use it only between development relays with self-signed certificates: with it, anyone who can intercept the connection can pose as a peer.

## More information

- [Transports](/guide/concepts/transports), for why browsers are strict about WebTransport certificates
- [Install the relay](/relay/install)
- [Relay CLI](/relay/reference/cli): `--cert-file` and `--key-file`
- [Configuration file](/relay/reference/config): `cert_file`, `key_file` and `peer_no_cert_validation`
