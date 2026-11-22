---
description: Open a MOQT session to a relay with MOQtailClient, set its options, react when the session ends, and turn on logging.
---

# Connect to a relay

Every subscription, fetch and publication goes through a `MOQtailClient`: one WebTransport session to one relay. Open it first. If you only want to play a stream, [`MOQtailPlayer`](/ts/player/player) opens the session for you.

The examples import from the `moqtail` package. To add it to a project, see [Install the library](/ts/install).

## Open a session

```ts
import { MOQtailClient } from 'moqtail'

const client = new MOQtailClient({ url: 'moqt://relay.moqtail.dev' })
client.on('sessionterminated', (reason) => console.warn('Session ended:', reason))
await client.init()
```

The constructor only stores the options. `init()` opens the WebTransport session, offers `moqt-22` as its protocol and exchanges SETUP with the relay. Listeners you attach before `init()` see that exchange too. Once `init()` resolves, `client.serverSetup` holds the relay's SETUP.

`init()` runs once per client. A second call throws a `MOQtailError`. If it fails, it closes the client, fires `sessionterminated` and rejects, so to try again you make a new client:

```ts
async function connect(url: string, attempts = 3): Promise<MOQtailClient> {
  for (let attempt = 1; ; attempt++) {
    const client = new MOQtailClient({ url })
    try {
      await client.init()
      return client
    } catch (error) {
      if (attempt === attempts) throw error
      await new Promise((resolve) => setTimeout(resolve, 1000 * attempt))
    }
  }
}
```

## Choose the URL

`url` takes a `moqt://` or an `https://` URL, as a string or a `URL`. A `moqt://` URL is opened as the matching `https://` URL, because the browser only opens WebTransport over HTTPS:

| You pass                                 | WebTransport opens                     |
| ---------------------------------------- | -------------------------------------- |
| `moqt://relay.moqtail.dev`               | `https://relay.moqtail.dev` (port 443) |
| `moqt://localhost:4433`                  | `https://localhost:4433`               |
| `moqt://localhost:4433/app?room=1#msf:x` | `https://localhost:4433/app?room=1`    |
| `https://relay.example.com:4433/app`     | `https://relay.example.com:4433/app`   |

The fragment after `#` never leaves the browser. The scheme is not case-sensitive. [Transports](/guide/concepts/transports) explains how one relay port serves both WebTransport and raw QUIC.

The browser must trust the relay's certificate, or `init()` fails without asking. For a relay on your own machine, see [Set up TLS certificates](/relay/certificates).

## Send setup options

`setupOptions` takes a `SetupOptions` object. Its options go into the client's SETUP message. For example, to send an authorization token and name your application:

```ts
import { MOQtailClient, TokenAliasType } from 'moqtail'
import type { SetupOptions } from 'moqtail'

const token = 'your token'
const setupOptions: SetupOptions = {
  authorizationToken: [
    { aliasType: TokenAliasType.UseValue, tokenType: 0n, tokenValue: new TextEncoder().encode(token) },
  ],
  moqtImplementation: 'my-app/1.0',
}

const client = new MOQtailClient({ url: 'moqt://localhost:4433', setupOptions })
```

- You don't need `path`: over WebTransport the path travels in the URL.
- An option of a type MOQtail doesn't name goes in `extensions`, as a `KeyValuePair`.
- Don't add an AUTHORITY option. It is not allowed over WebTransport, and `init()` rejects it.
- The client tells the relay it accepts 32 filter ranges per request. `addMaxFilterRanges()` sets another number.

## Pass WebTransport options

`transportOptions` goes to the browser's `WebTransport` constructor as it is. The client adds `moqt-22` to its `protocols`. For example, `serverCertificateHashes` makes the browser accept a self-signed certificate by its SHA-256 hash. The browser only allows this for a certificate that is valid for 14 days or less.

```ts
const certificateHash = Uint8Array.from(atob('<base64 SHA-256 of the DER certificate>'), (c) => c.charCodeAt(0))

const client = new MOQtailClient({
  url: 'moqt://localhost:4433',
  transportOptions: {
    serverCertificateHashes: [{ algorithm: 'sha-256', value: certificateHash }],
  },
})
```

## Set the other options

| Option                          | Default | What it does                                                                                                                                   |
| ------------------------------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------- |
| `enableDatagrams`               | `false` | Starts reading and writing QUIC datagrams in `init()`. `client.startDatagrams()` does it later.                                                |
| `ordering`                      | none    | Releases each request's objects in location order. See [Subscribe and fetch](/ts/client/subscribe#receive-objects-in-order).                   |
| `dataStreamTimeoutMs`           | none    | How long a data stream may go without new bytes before the client gives up on it.                                                              |
| `controlStreamTimeoutMs`        | none    | How long the client waits for the rest of a control message that arrived in part. When it runs out, the session ends.                          |
| `trackAliasResolutionTimeoutMs` | `2000`  | How long a data stream that arrives before its SUBSCRIBE_OK waits for it.                                                                      |
| `switchDrainTimeoutMs`          | `2000`  | How long the old track of a soft [switch](/ts/client/subscribe#switch-to-another-track) may stay quiet before the client stops waiting for it. |
| `fillOpenTimeoutMs`             | `1000`  | How long after SUBSCRIBE_OK ordering waits for a fill stream to open.                                                                          |

All timeouts are in milliseconds.

## Handle the end of a session

Two events tell you the session is ending:

- `goaway`: the relay is shutting down or wants you elsewhere. `goAway.newSessionUri` is where to reconnect, or `undefined` for the same URL. The relay closes the session `goAway.timeout` milliseconds later, or without a set deadline when that is `0`. The client does not reconnect for you.
- `sessionterminated`: the session is over, because you called `disconnect()`, the relay closed it, or it failed. It fires once. The client can't be used after it, so make a new one.

```ts
const url = 'moqt://relay.moqtail.dev'

client.on('goaway', (goAway) => {
  void moveTo(goAway.newSessionUri ?? url).catch((error) => console.error('Reconnect failed:', error))
})

async function moveTo(url: string): Promise<void> {
  const next = await connect(url)
  // Start your subscriptions and publications again on `next`, then close the old session
  await client.disconnect('moved to a new session')
}
```

A relay can also send GOAWAY for one request only. The client then sends that request again on the same session by itself. If that GOAWAY names a new session, the client ends the request and fires `goaway` instead.

[Run the relay in production](/relay/production) covers when the relay sends GOAWAY.

## Turn on logging

The library logs to the console, warnings and errors only by default. The log level and the module filter are global: they apply to every client and player on the page.

```ts
import { LogLevel, MOQtailClient } from 'moqtail'

MOQtailClient.setLogLevel(LogLevel.DEBUG)
MOQtailClient.setLogEnabledModules(['MOQtailClient', 'control_stream'])
```

Each line starts with `[MOQtail][<module>]`. `setLogEnabledModules()` keeps only the modules you list, and `null` shows all of them again. `LogLevel.NONE` silences the library.

To see every control message, listen to `messagesent` and `messagereceived`:

```ts
client.on('messagesent', (msg) => console.log('→', msg))
client.on('messagereceived', (msg) => console.log('←', msg))
```

## Disconnect

```ts
await client.disconnect()
```

`disconnect()` ends every open request, closes the WebTransport session and fires `sessionterminated`. It takes an optional reason, which `sessionterminated` passes on. Calling it again does nothing. If you publish, end your tracks first, as [Publish a track](/ts/client/publish#stop-publishing) shows.

## More information

- [`MOQtailClient`](/ts/api/client/classes/MOQtailClient)
- [`MOQtailClientOptions`](/ts/api/client/type-aliases/MOQtailClientOptions)
- [`ClientEvents`](/ts/api/client/interfaces/ClientEvents)
- [`SetupOptions`](/ts/api/model/interfaces/SetupOptions)
- [`AuthorizationToken`](/ts/api/model/type-aliases/AuthorizationToken)
- [`GoAway`](/ts/api/model/interfaces/GoAway)
- [`LogLevel`](/ts/api/util/enumerations/LogLevel)
