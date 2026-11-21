---
layout: home
description: Documentation for the MOQtail relay and the moqtail TypeScript library.

hero:
  # The spans are the animated aurora behind the letters (theme/aurora.css).
  name: 'MOQtail<span class="aurora" aria-hidden="true"><i></i><i></i><i></i><i></i><i></i></span>'
  text: Media over QUIC, relay to browser
  tagline: IETF-compliant MOQT relay, TypeScript client and browser player.
  image:
    src: /moqtail-medal.webp
    alt: The MOQtail cocktail wearing the ACM MMSys 2026 Best Open Dataset and Software Award medal
  actions:
    - theme: brand
      text: Get started
      link: /guide/quickstart
    - theme: alt
      text: Run a relay
      link: /relay/
    - theme: alt
      text: TypeScript library
      link: /ts/

features:
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><rect x="16" y="16" width="6" height="6" rx="1"/><rect x="2" y="16" width="6" height="6" rx="1"/><rect x="9" y="2" width="6" height="6" rx="1"/><path d="M5 16v-3a1 1 0 0 1 1-1h12a1 1 0 0 1 1 1v3"/><path d="M12 12V8"/></svg>'
    title: Relay
    details: WebTransport and raw QUIC on one UDP port, a relay mesh, an edge cache and post-quantum TLS.
    link: /relay/
    linkText: Run a relay
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M15.033 9.44a.647.647 0 0 1 0 1.12l-4.065 2.352a.645.645 0 0 1-.968-.56V7.648a.645.645 0 0 1 .967-.56z"/><path d="M12 17v4"/><path d="M8 21h8"/><rect x="2" y="3" width="20" height="14" rx="2"/></svg>'
    title: TypeScript library
    details: A MOQT client, the &lt;moqtail-video&gt; element and a composable player with learned ABR and live latency control.
    link: /ts/
    linkText: Play a stream
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M22 12h-2.48a2 2 0 0 0-1.93 1.46l-2.35 8.36a.25.25 0 0 1-.48 0L9.24 2.18a.25.25 0 0 0-.48 0l-2.35 8.36A2 2 0 0 1 4.49 12H2"/></svg>'
    title: Observability
    details: An HTTP API, Prometheus metrics, qlog traces, a slow-object log and an MCP server for AI agents.
    link: /relay/observability
    linkText: Monitor a relay
  - icon: '<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M14 2v6a2 2 0 0 0 .245.96l5.51 10.08A2 2 0 0 1 18 22H6a2 2 0 0 1-1.755-2.96l5.51-10.08A2 2 0 0 0 10 8V2"/><path d="M6.453 15h11.094"/><path d="M8.5 2h7"/></svg>'
    title: Standards
    details: Built on draft-ietf-moq-transport-22 with MSF, CMSF and LOC packaging, and conformance tested.
    link: /guide/
    linkText: See what is supported
---

## Try it

Play a live stream from the public relay, install the library, or run your own relay.

::: code-group

```html [Browser]
<script type="module">
  import { defineMOQtailVideo } from 'moqtail/player/component'
  defineMOQtailVideo()
</script>

<moqtail-video src="moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog" autoplay muted playsinline></moqtail-video>
```

```sh [npm]
npm install moqtail
```

```sh [Docker]
docker run --rm \
  -p 4433:4433/udp \
  -v "$PWD/cert.pem:/certs/cert.pem:ro" \
  -v "$PWD/key.pem:/certs/key.pem:ro" \
  ghcr.io/moqtail/relay:latest
```

:::
