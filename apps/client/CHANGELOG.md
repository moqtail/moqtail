# client

## 0.14.2

### Patch Changes

- [#214](https://github.com/moqtail/moqtail/pull/214) [`fbdbc75`](https://github.com/moqtail/moqtail/commit/fbdbc752fdf6c83b976539cc904bed476236d49c) Thanks [@sharmafb](https://github.com/sharmafb)! - handle malformed FETCH track on end client

- [#289](https://github.com/moqtail/moqtail/pull/289) [`7daccb6`](https://github.com/moqtail/moqtail/commit/7daccb61603cd2d50826011bea7f79ac94d7615e) Thanks [@kerembkmz](https://github.com/kerembkmz)! - fix(moqtail-ts): stop TerminationCode.tryFrom throwing on five valid enum values

## 0.14.0

### Minor Changes

- [`0ca44e5`](https://github.com/moqtail/moqtail/commit/0ca44e59cf39ac97e73e465e80b64dba0302b2ba) Thanks [@zafergurel](https://github.com/zafergurel)! - Add raw QUIC transport support alongside WebTransport. `TransportConnection`/`TransportSendStream`/`TransportRecvStream` in moqtail-rs now wrap both wtransport and raw quinn behind one API, threaded through the relay and client stream handlers. The relay's QUIC listener demultiplexes WebTransport and raw-QUIC connections on the same UDP socket/port via ALPN, and the client connects over raw QUIC when given a `moqt://authority[/path]` server URL (carrying authority/path via CLIENT_SETUP parameters instead of HTTP CONNECT).

## 0.13.0

### Minor Changes

- [#145](https://github.com/moqtail/moqtail/pull/145) [`1b855cf`](https://github.com/moqtail/moqtail/commit/1b855cfece77cbade63f8263f485b8b5c7839134) Draft-16 compliance

## 0.11.0

### Minor Changes

- [#104](https://github.com/moqtail/moqtail/pull/104) [`a08c438`](https://github.com/moqtail/moqtail/commit/a08c4380f7a0abd25fcfa424ce3fbb5d90b4a977) Thanks [@zafergurel](https://github.com/zafergurel)! - Implements SWITCH message

## 0.9.0

### Minor Changes

- [#95](https://github.com/moqtail/moqtail/pull/95) [`5cd1a9e`](https://github.com/moqtail/moqtail/commit/5cd1a9ee3a04cb0a086c0873772d1ed8f85136d1) Thanks [@zafergurel](https://github.com/zafergurel)! - Add datagram support for publishing and subscribing to objects via QUIC datagrams

- [#92](https://github.com/moqtail/moqtail/pull/92) [`fa6c468`](https://github.com/moqtail/moqtail/commit/fa6c468ed5dc0dd8fb6a8ff2d07e063394cddeb5) Thanks [@DenizUgur](https://github.com/DenizUgur)! - update wtransport create

### Patch Changes

- [#96](https://github.com/moqtail/moqtail/pull/96) [`c94f626`](https://github.com/moqtail/moqtail/commit/c94f626cf320f18b6b042a6da971a6a2dc1108bd) Thanks [@fatih-alperen](https://github.com/fatih-alperen)! - Added support for TRACK_STATUS control messages

## 0.8.0

### Minor Changes

- [#74](https://github.com/moqtail/moqtail/pull/74) [`aa4ff01`](https://github.com/moqtail/moqtail/commit/aa4ff01d9c642de9f0d3fb5fdaaeb12d29abc8ee) Thanks [@zafergurel](https://github.com/zafergurel)! - Add auth token parameter as a setup parameter
