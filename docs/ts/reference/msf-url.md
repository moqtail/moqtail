---
description: The MSF URL format the moqtail player takes, how namespaces and track names are encoded in it, and the parameters it reads.
---

# MSF URLs

An MSF URL names a relay and a track on it. `MOQtailPlayer.source()` and the `src` of `<moqtail-video>` take one. The format comes from the MSF draft, section 11.1.

## Format

```
moqt://<authority>[/<path>][?<query>]#msf:<namespace>--<track>[&<name>=<value>…]
```

| Part                | Description                                                                           |
| ------------------- | ------------------------------------------------------------------------------------- |
| `moqt://`           | The scheme, in any case. It is lowercased.                                            |
| `<authority>`       | The relay, `host[:port]`. Required.                                                   |
| `<path>`, `<query>` | Passed to the relay. Over WebTransport they are part of the session's `https://` URL. |
| `#msf:`             | The fragment type. It is required, and the fragment never leaves the browser.         |
| `<namespace>`       | The track namespace, its fields joined by `-`.                                        |
| `--`                | Separates the namespace from the track name.                                          |
| `<track>`           | The track to start from. See [Which tracks play](#which-tracks-play).                 |
| `&<name>=<value>`   | [Parameters](#parameters), as many as needed.                                         |

Everything before `#` is the session URL, `MsfSource.url`. The player opens a WebTransport session at its `https://` form, `https://<authority>/<path>?<query>`, on port 443 when the authority has none. One player takes one session URL: a second `source()` with another one throws `UNSUPPORTED_ORIGIN`.

## Which tracks play

`init()` picks a source element for the track by its identifiers. The URL gives the namespace and the track name, but not the packaging or the role that the media elements probe for. Of the built-in elements, only the catalog source accepts a URL, when its track is named `catalog`, and the catalog then [names the media tracks](/guide/concepts/media#what-the-player-reads-from-it).

A URL that names any other track plays only through a source element you register with `registerElement()`, whose `probe` accepts it. Without one, `init()` rejects with `UNSUPPORTED_SOURCE`.

## Namespace and track names

A namespace is a tuple of fields, and each field and the track name are bytes. In the URL, each one is encoded on its own:

- `A`–`Z`, `a`–`z`, `0`–`9` and `_` are written as they are.
- Every other byte is written as `.` and two lowercase hexadecimal digits: a space is `.20`, `-` is `.2d`, `.` is `.2e`, `é` (UTF-8 `c3 a9`) is `.c3.a9`.

Since a name never holds a bare `-`, the first `--` ends the namespace and each single `-` inside it ends a field.

| Namespace fields                | Track name | In the URL                           |
| ------------------------------- | ---------- | ------------------------------------ |
| `moqtail`, `ch07`               | `catalog`  | `moqtail-ch07--catalog`              |
| `moqtail`, `ch07_vod`           | `catalog`  | `moqtail-ch07_vod--catalog`          |
| `live`, `my show`               | `video 1`  | `live-my.20show--video.201`          |
| `customer`, `livestream`, `123` | `café`     | `customer-livestream-123--caf.c3.a9` |

A name has exactly one encoding. The parser rejects any other spelling: uppercase hexadecimal (`.2D`), an escaped literal (`.61` for `a`), percent-encoding (`%41`), a bare `-` in a name, or a `.` not followed by two digits.

The namespace has 1 to 32 fields, none of them empty. The track name may be empty.

## Parameters

Each parameter is `name=value`, and both are percent-decoded. The parser reads these:

| Parameter         | Value           | Meaning                                                                                                                                                                      |
| ----------------- | --------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `location-range`  | `G[.O][-G[.O]]` | Plays from group `G`, object `O`, to the end location, both inclusive. The start object defaults to 0. An end without an object runs to the end of that group. No end: open. |
| `mediatime-range` | `ms[-ms]`       | Plays a range of media time, in milliseconds. No end: open.                                                                                                                  |
| `wallclock-range` | `ms[-ms]`       | Plays a range of wall-clock time, in milliseconds since the Unix epoch. No end: open.                                                                                        |
| `cat`             | a token         | A Common Access Token, kept as `MsfSource.cat`. The player does not send it by itself.                                                                                       |
| `connection`      | `q` or `wt`     | The transport: raw QUIC or WebTransport. Browsers have only WebTransport, so the player rejects `q`.                                                                         |

Each range parameter can appear more than once. The player sorts the ranges and merges the ones that overlap. It plays only what they cover, clamped to the window the stream still has. Media time and wall-clock ranges need a media timeline in the catalog, a timeline track or a template; without one, the player rejects them.

### Catalog variables

Every parameter, including those the parser does not read, is a variable for the catalog. A string value in the catalog that holds `%name%` gets the parameter's value in its place. When a name appears twice, the last value counts.

The value has to consist of `A`–`Z`, `a`–`z`, `0`–`9`, `_`, `@` and `-`. A variable with other characters, or one the URL does not have, stays as written, and the player emits a `VARIABLE_SKIPPED` warning.

## Errors

`parseMsfUrl()` and `MOQtailPlayer.source()` throw a `PlayerError` with the code `UNSUPPORTED_SOURCE` when:

- the URL does not start with `moqt://`, has no authority, or has no `#msf:` fragment;
- the fragment has no `--`, or a name is not in its one encoding;
- a parameter has no `=`, has an empty name, or has bad percent-encoding;
- a range does not parse, starts after it ends, or holds a time past `Number.MAX_SAFE_INTEGER`;
- `connection` is neither `q` nor `wt`. `source()` also rejects `connection=q`.

`init()` rejects with `UNSUPPORTED_SOURCE` when no source element accepts the track.

## Examples

The public relay has these streams:

| URL                                                                                | Plays                                  |
| ---------------------------------------------------------------------------------- | -------------------------------------- |
| `moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog`                               | The live stream `moqtail/ch07`         |
| `moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog`                           | The same content as VOD                |
| `moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog&location-range=2-6`        | Groups 2 to 6 of the VOD               |
| `moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog&mediatime-range=4000-9999` | 4 s to 10 s of the VOD                 |
| `moqt://localhost:4433/app?token=abc#msf:live-demo--catalog`                       | A local relay, with a path and a query |

## `parseMsfUrl` and `formatMsfUrl`

`moqtail/player` exports both. `parseMsfUrl()` returns an `MsfSource`: the session URL, the namespace as a `Tuple`, the track name as bytes and as text, the ranges, `cat`, `connection` and the variables. `formatMsfUrl()` builds the URL from a session URL, a namespace and a track name, and encodes the names:

```ts
import { Tuple } from 'moqtail'
import { formatMsfUrl, parseMsfUrl } from 'moqtail/player'

formatMsfUrl('moqt://relay.moqtail.dev', Tuple.fromUtf8Path('moqtail/ch07'), 'catalog')
// 'moqt://relay.moqtail.dev#msf:moqtail-ch07--catalog'

parseMsfUrl('moqt://relay.moqtail.dev#msf:moqtail-ch07_vod--catalog&location-range=2-6').locationRanges
// [{ start: Location(2, 0), end: { group: 6n } }]
```

`MOQtailClient` takes an MSF URL as its `url` too. It uses the session URL and ignores the fragment.

## Related

- [`parseMsfUrl`](/ts/api/player/functions/parseMsfUrl), [`formatMsfUrl`](/ts/api/player/functions/formatMsfUrl)
- [`MsfSource`](/ts/api/player/interfaces/MsfSource), [`LocationRange`](/ts/api/player/interfaces/LocationRange), [`TimeRange`](/ts/api/player/interfaces/TimeRange)
- [Catalogs and packaging](/guide/concepts/media)
- [Write plugins and elements](/ts/player/plugins)
- [Transports](/guide/concepts/transports)
