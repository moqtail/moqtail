# moqtail-ts

## 0.5.0

### Minor Changes

- [#69](https://github.com/moqtail/moqtail/pull/69) [`a22396a`](https://github.com/moqtail/moqtail/commit/a22396a9e891b2424a14b2ad18682473f890093d) Thanks [@acbegen](https://github.com/acbegen)! - integrated draft 14 changes

## 0.4.1

### Patch Changes

- [`92b8280`](https://github.com/moqtail/moqtail/commit/92b82804ed8dd981ceb138c6fedad7a6a69dc092) Thanks [@acbegen](https://github.com/acbegen)! - fix infinite loop case when client is destroyed

## 0.4.0

### Minor Changes

- [`6bfeaff`](https://github.com/moqtail/moqtail/commit/6bfeaff649ba665a85d2d61ef00289512eaf4f39) Thanks [@acbegen](https://github.com/acbegen)! - Handle errors in accepting unidirectional streams more gracefully

## 0.3.2

### Patch Changes

- [`d13be5e`](https://github.com/streaming-university/moqtail/commit/d13be5e2ca051d85c5230dcc3e94a5036c9bf08b) Thanks [@acbegen](https://github.com/acbegen)! - bump version to integrate the latest changes

## 0.3.1

### Patch Changes

- [#48](https://github.com/streaming-university/moqtail/pull/48) [`b89e901`](https://github.com/streaming-university/moqtail/commit/b89e901bcce0d70d5d07ae9baaccafefbe757550) Thanks [@LeventAksakal](https://github.com/LeventAksakal)! - Add documentation for most of the public facing library api
  - add api-extractor for document standardization
  - add type-doc for static site generation based off tsdocs
  - add documentation for the following items:
    - moqtail client, client options
    - track, track source and object cache
    - some items under ./model (e.g reason phrase, full track name ...)
  - update client-js library imports to use aliases

## 0.3.0

### Minor Changes

- [#33](https://github.com/streaming-university/moqtail/pull/33) [`0b59e15`](https://github.com/streaming-university/moqtail/commit/0b59e1582aca3eb307e75097b4a3716971dd523d) Thanks [@LeventAksakal](https://github.com/LeventAksakal)! - Refactor fetch message structure for better type safety and API design
  - **BREAKING**: Refactored `Fetch` class to use discriminated union for `typeAndProps`
    - Removed static factory methods (`Fetch.newStandAlone`, `Fetch.newRelative`, `Fetch.newAbsolute`)
    - Combined fetch type and properties into a single discriminated union field
    - Enables automatic TypeScript type narrowing based on fetch type
  - **NEW**: Implemented both publisher and subscriber functionality for fetch operations
  - **TODO**: Joining fetches require additional handling (partial implementation)
  - **PERFORMANCE**: Reduced test timeouts to improve CI pipeline speed
    - Telemetry tests: 5s → 1s timeouts
    - ControlStream tests: 3s → 1s timeouts
    - AkamaiOffset tests: mocked network calls instead of real ones
  - **CI**: Update husky hooks
    - Remove exec < /dev/tty from pre-commit and prepare-commit-msg for windows compatibility
  - **FIX**: Track alias map
    - Track alias map was using number for track alias instead of bigint
    - Update the structure and relevant parts of the code

  This change provides a more idiomatic and type-safe API for fetch operations while maintaining backward compatibility for serialization/deserialization.

- [#33](https://github.com/streaming-university/moqtail/pull/33) [`0b59e15`](https://github.com/streaming-university/moqtail/commit/0b59e1582aca3eb307e75097b4a3716971dd523d) Thanks [@LeventAksakal](https://github.com/LeventAksakal)! - Refactor client API and enhance subscribe request functionality
  - **BREAKING**: Updated subscribe request API for better control and lifecycle management
  - **NEW**: Added update functionality to subscribe requests
    - Allows modifying subscription parameters after initial request
    - Enables dynamic subscription management
  - **NEW**: Added cancel functionality to subscribe requests
    - Provides clean subscription cleanup
    - Prevents memory leaks from abandoned subscriptions
  - **PERFORMANCE**: Removed slow Akamai unit tests
    - Tests were causing CI pipeline delays
  - **API**: Improved client API ergonomics
    - Better type safety for subscribe operations
    - Remove redundant public identifiers (everything is public by default)
    - Use # for private members

  This change provides better control over subscription lifecycle

- [#46](https://github.com/streaming-university/moqtail/pull/46) [`74de932`](https://github.com/streaming-university/moqtail/commit/74de932bfd6d002b350eda1e09208ca39975d745) Thanks [@kerembkmz](https://github.com/kerembkmz)! - Media stream subscription management added with independent video/audio control.
