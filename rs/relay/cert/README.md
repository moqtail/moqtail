# Local Certificate Setup for WebTransport

## Quick Setup

1. **Install mkcert**:

- Follow the [official mkcert installation instructions](https://github.com/FiloSottile/mkcert#installation)

Sample script:

```bash
# Install local CA
mkcert -install

# Run from `apps/relay/cert` or manually move the *.pem under `cert/`
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 ::1
```

2. **Enable browser to trust private CAs**:

- Chrome:
  - Navigate to `chrome://flags/#webtransport-developer-mode`
  - Enable `WebTransport Developer Mode`
  - Restart Chrome

- Firefox:
  - Navigate to `about:config`
  - Set `network.http.http3.disable_when_third_party_roots_found` to `false`

> [!NOTE]
> Instructions for Edge are pending. If you successfully configure this browser,
> please consider contributing the steps!

---

Certificates should be placed next to this README as `cert.pem` and `key.pem`
