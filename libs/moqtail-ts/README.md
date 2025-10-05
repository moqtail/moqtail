# MOQtail TypeScript Client Library

> ⚠️ **Work in Progress**: This library is under active development and the API is subject to change. Please use with caution in production environments.

MOQT (Media over QUIC Transport) is a protocol for media delivery over QUIC connections, enabling efficient streaming of live and on-demand content. The MOQtail client library provides a TypeScript implementation that supports both publisher and subscriber roles in the MOQT ecosystem.

## Overview

The `MOQtailClient` serves as the main entry point for interacting with MoQ relays and other peers. A client can act as:

- **Original Publisher**: Creates and announces tracks, making content available to subscribers
- **End Subscriber**: Discovers and consumes content from publishers via track subscriptions

## Publisher

As a publisher, the MOQtail client allows you to create, manage, and distribute content through tracks. The library handles protocol-level details while giving you full control over content creation and packaging.

### Track Management

Publishers can add or remove tracks using the `addOrUpdateTrack()` and `removeTrack()` methods:

```typescript
const client = await MOQtailClient.new(clientSetup, webTransport)

// Add a new track
client.addOrUpdateTrack(myTrack)

// Remove an existing track
client.removeTrack(myTrack)
```

### Track Structure

Each track is defined by the `Track` interface, which consists of:

- **`fullTrackName`**: Unique identifier for the track (namespace + track name)
- **`trackAlias`**: Numeric alias used for efficient wire representation
- **`forwardingPreference`**: How objects should be delivered (Datagram or Subgroup)
- **`contentSource`**: The source of content for this track

### Content Sources

The `ContentSource` interface is the heart of the publisher model, providing two distinct patterns for content delivery:

#### Live Content (Streaming)

For real-time content like live video streams, use `LiveContentSource`:

- Content flows through a `ReadableStream<MoqtObject>`
- Subscribers receive content via **Subscribe** operations
- Suitable for continuously generated content

#### Static Content (On-Demand)

For archived or pre-generated content, use `StaticContentSource`:

- Content is stored in an `ObjectCache` for random access
- Subscribers retrieve specific ranges via **Fetch** operations
- Ideal for video-on-demand, file transfers, or cached content

#### Hybrid Content

For tracks that support both patterns, use `HybridContentSource`:

- Combines live streaming with historical data access
- New objects are added to cache while also flowing to live subscribers

### Object Packaging

All content is packaged as `MoqtObject` instances, which represent the atomic units of data in MoQ:

- **Location**: Identified by `groupId` and `objectId` (e.g., video frames within GOPs)
- **Payload**: The actual media data or content
- **Metadata**: Publisher priority, forwarding preferences, and extension headers
- **Status**: Normal data, end-of-group markers, or error conditions

### Object Caching

The `ObjectCache` interface provides two simple implementations for static content:

- **`MemoryObjectCache`**: Unlimited in-memory storage with binary search indexing
- **`RingBufferObjectCache`**: Fixed-size cache with automatic eviction of oldest objects

### Publisher Workflow

1. **Create Content**: Generate or prepare your media content
2. **Package as Objects**: Wrap content in `MoqtObject` instances with appropriate metadata
3. **Choose Content Source**: Select `LiveContentSource`, `StaticContentSource`, or `HybridContentSource`
4. **Define Track**: Create a `Track` with your content source and metadata
5. **Add to Client**: Register the track with `addOrUpdateTrack()`
6. **Publish Namespace**: Use `publishNamespace()` to make the track discoverable by subscribers
7. **Manage Lifecycle**: The library handles incoming subscribe/fetch requests and data delivery

### Example

```typescript
// Create a live video track
const videoTrack: Track = {
  fullTrackName: FullTrackName.tryNew('live/conference', 'video'),
  trackAlias: 1n,
  forwardingPreference: ObjectForwardingPreference.Subgroup,
  contentSource: new LiveContentSource(videoStream),
}

// Create a static file track
const fileCache = new MemoryObjectCache()
// ... populate cache with file chunks ...
const fileTrack: Track = {
  fullTrackName: FullTrackName.tryNew('files/documents', 'presentation.pdf'),
  trackAlias: 2n,
  forwardingPreference: ObjectForwardingPreference.Datagram,
  contentSource: new StaticContentSource(fileCache),
}

// Register tracks and announce
client.addOrUpdateTrack(videoTrack)
client.addOrUpdateTrack(fileTrack)

await client.publishNamespace(new PublishNamespace(client.nextClientRequestId, Tuple.tryNew(['live', 'conference'])))
```

The library automatically manages active requests, handles protocol negotiation, and ensures efficient data delivery based on subscriber demands and network conditions.

## Subscriber

As a subscriber, the MOQtail client enables you to discover, request, and consume content from publishers. The library provides two main mechanisms for content retrieval: `subscribe()` for live streaming content and `fetch()` for on-demand content access.

### Live Content Subscription

For real-time streaming content, use `subscribe()` which returns either a `ReadableStream<MoqtObject>` or a `SubscribeError`:

#### Subscribe Implementation

Subscribe operations are designed for live streaming and can be delivered through multiple transport mechanisms:

- **Datagrams**: For low-latency delivery where occasional packet loss is acceptable
- **Multiple Streams**: Each group (GOP) can be delivered in a separate stream for better prioritization
- **Stream Cancellation**: The library implements intelligent stream cancellation on both publisher and subscriber sides:
  - **Publisher Side**: Automatically cancels streams for older groups when bandwidth is limited
  - **Subscriber Side**: Cancels streams for groups that are no longer needed due to latency constraints

This approach ensures that subscribers always receive the most recent content with minimal latency, automatically dropping outdated frames during network congestion.

```typescript
const subscribe = new Subscribe(
  client.nextClientRequestId,
  trackAlias, // Numeric alias for the track
  fullTrackName, // Full track name
  subscriberId, // Your subscriber ID
  startGroup, // Starting group ID (or null for latest)
  startObject, // Starting object ID (or null for latest)
  endGroup, // Ending group ID (or null for ongoing)
  endObject, // Ending object ID (or null for group end)
  authInfo, // Authorization information
)

const result = await client.subscribe(subscribe)

if (result instanceof SubscribeError) {
  console.error(`Subscription failed: ${result.reasonPhrase}`)
  // Handle error based on error code
  switch (result.errorCode) {
    case SubscribeErrorCode.InvalidRange:
      // Adjust range and retry
      break
    default:
      console.error(`Unknown error: ${result.reasonPhrase}`)
  }
} else {
  // Success - result is ReadableStream<MoqtObject>
  const objectStream = result
  const reader = objectStream.getReader()

  try {
    while (true) {
      const { done, value: object } = await reader.read()
      if (done) break

      // Process each object
      console.log(`Received object ${object.objectId} from group ${object.groupId}`)
      processObject(object)
    }
  } finally {
    reader.releaseLock()
  }
}
```

### On-Demand Content Fetching

For static or archived content, use `fetch()` which returns either a `ReadableStream<MoqtObject>` or a `FetchError`:

#### Fetch Implementation

Fetch operations are optimized for reliable delivery of static content:

- **Single Stream**: All requested objects are delivered sequentially in a single stream
- **Reliable Delivery**: Uses QUIC streams for guaranteed, ordered delivery
- **No Cancellation**: All requested objects are delivered as they provide historical data

```typescript
const fetch = new Fetch(
  client.nextClientRequestId,
  trackAlias,
  fullTrackName,
  subscriberId,
  startGroup, // Starting group ID
  startObject, // Starting object ID
  endGroup, // Ending group ID
  endObject, // Ending object ID
  authInfo,
)

const result = await client.fetch(fetch)

if (result instanceof FetchError) {
  console.error(`Fetch failed: ${result.reasonPhrase}`)
  // Handle fetch error
} else {
  // Success - result is ReadableStream<MoqtObject>
  const objectStream = result
  const reader = objectStream.getReader()

  try {
    while (true) {
      const { done, value: object } = await reader.read()
      if (done) break

      // Process fetched object
      processObject(object)
    }
  } finally {
    reader.releaseLock()
  }
}
```

### Content Processing

Once you have the stream, process each `MoqtObject` based on its status:

```typescript
function processObject(object: MoqtObject) {
  // Check object status
  switch (object.objectStatus) {
    case ObjectStatus.Normal:
      // Regular data object with payload
      if (object.payload) {
        processData(object.payload)
      }
      break
    case ObjectStatus.ObjectDoesNotExist:
      // Object was not available
      handleMissingObject(object.groupId, object.objectId)
      break
    case ObjectStatus.GroupDoesNotExist:
      // Entire group was not available
      handleMissingGroup(object.groupId)
      break
    case ObjectStatus.EndOfGroup:
      // Marks the end of a group
      finalizeGroup(object.groupId)
      break
    case ObjectStatus.EndOfTrack:
      // Marks the end of the track
      finalizeTrack()
      break
  }
}
```

### Subscription Management

#### Subscription Lifecycle

```typescript
// Create and send subscription
const subscribe = new Subscribe(/*...*/)
const result = await client.subscribe(subscribe)

if (result instanceof SubscribeError) {
  console.error(`Subscription failed: ${result.reasonPhrase}`)
} else {
  console.log('Subscription successful, processing stream...')
  // Process the stream as shown above
}

// Unsubscribe when done
await client.unsubscribe(subscribeId)
```

#### Subscription Updates

For live content, you can update the subscription range dynamically:

```typescript
const subscribeUpdate = new SubscribeUpdate(
  subscribeId,
  startGroup, // New start group
  startObject, // New start object
  endGroup, // New end group (optional)
  endObject, // New end object (optional)
  subscriberPriority, // New priority (optional)
)

await client.subscribeUpdate(subscribeUpdate)
```

### Complete Subscriber Example

```typescript
import { MOQtailClient } from './client/client'
import { PullPlayoutBuffer } from './util/pull_playout_buffer'

async function createSubscriber() {
  // Initialize client
  const client = await MOQtailClient.new(clientSetup, webTransport)

  // Subscribe to live video
  const subscribe = new Subscribe(
    client.nextClientRequestId,
    1n, // trackAlias
    FullTrackName.tryNew('live/conference', 'video'),
    generateSubscriberId(),
    null,
    null, // Latest content
    null,
    null, // Ongoing
    null, // No auth
  )

  const result = await client.subscribe(subscribe)

  if (result instanceof SubscribeError) {
    console.error(`Failed to subscribe: ${result.reasonPhrase}`)
    return
  }

  // Set up playout buffer with the stream
  const playoutBuffer = new PullPlayoutBuffer(result, {
    bucketCapacity: 50,
    targetLatencyMs: 500,
    maxLatencyMs: 2000,
  })

  // Consumer-driven playout
  const playoutLoop = () => {
    playoutBuffer.nextObject((nextObject) => {
      if (nextObject) {
        // Decode and render the frame
        decodeAndRender(nextObject)
      }
      requestAnimationFrame(playoutLoop)
    })
  }

  // Start playout
  requestAnimationFrame(playoutLoop)

  return client
}
```

### Other Client Operations

The MOQtail client supports additional operations for track discovery and status management:

#### PublishNamespace Operations

Publishers use announce operations to make their tracks discoverable:

```typescript
// PublishNamespace a namespace
const announce = new PublishNamespace(
  client.nextClientRequestId,
  Tuple.tryNew(['live', 'conference']), // Track namespace
)

const result = await client.publishNamespace(announce)
if (result instanceof PublishNamespaceError) {
  console.error(`Publishing the namespace failed: ${result.reasonPhrase}`)
} else {
  console.log('Namespace published successfully')
}

// Stop announcing a namespace
const publish_namespace_done = new publishNamespaceDone(Tuple.tryNew(['live', 'conference']))
await client.publishNamespaceDone(publish_namespace_done)
```

#### Subscribe to Announcements

Subscribers can discover available tracks by subscribing to announcements:

```typescript
// Subscribe to announcements for a namespace prefix
const subscribeNamespace = new SubscribeNamespace(
  Tuple.tryNew(['live']), // Namespace prefix
)
await client.subscribeNamespace(subscribeNamespace)

// The client will now receive announce messages for tracks
// matching the 'live' prefix through its announcement handling

// Stop subscribing to announcements
const unsubscribeNamespace = new UnsubscribeNamespace(Tuple.tryNew(['live']))
await client.unsubscribeNamespace(unsubscribeNamespace)
```

#### Track Status Requests

Query the status of specific tracks:

```typescript
const trackStatus = new TrackStatusMessage(client.nextClientRequestId, FullTrackName.tryNew('live/conference', 'video'))

const result = await client.trackStatus(trackStatus)
if (result instanceof TrackStatusError) {
  console.error(`Track status request failed: ${result.reasonPhrase}`)
} else {
  // result is TrackStatus
  console.log(`Track status: ${result.statusCode}`)
  console.log(`Last group: ${result.lastGroup}`)
  console.log(`Last object: ${result.lastObject}`)
}
```

## Utilities

The MOQtail library provides several utility classes to help with common streaming scenarios:

### Playout Buffer

The `PullPlayoutBuffer` provides consumer-driven playout with GOP-aware buffering for smooth media playback:

```typescript
import { PullPlayoutBuffer } from './util/pull_playout_buffer'

const playoutBuffer = new PullPlayoutBuffer(objectStream, {
  bucketCapacity: 50, // Max objects in buffer (default: 50)
  targetLatencyMs: 500, // Target latency in ms (default: 500)
  maxLatencyMs: 2000, // Max latency before dropping GOPs (default: 2000)
})

// Consumer-driven object retrieval
playoutBuffer.nextObject((nextObject) => {
  if (nextObject) {
    // Process the object (decode, render, etc.)
    processFrame(nextObject)
  }
})

// Check buffer status
const status = playoutBuffer.getStatus()
console.log(`Buffer size: ${status.bufferSize}, Running: ${status.isRunning}`)
```

**Key Features:**

- **GOP-Aware**: Automatically detects and manages Group of Pictures boundaries
- **Smart Eviction**: Drops entire GOPs when buffer is full to maintain decodable content
- **Consumer-Driven**: Pull-based API eliminates rate guessing and provides natural backpressure
- **Latency Management**: Automatically manages buffer size to maintain target latency

### Network Telemetry

The `NetworkTelemetry` class provides real-time network performance monitoring:

```typescript
import { NetworkTelemetry } from './util/telemetry'

const telemetry = new NetworkTelemetry(1000) // 1-second sliding window

// Report network events
telemetry.push({
  latency: 50, // Round-trip time in ms
  size: 1024, // Bytes transferred
})

// Get current metrics
console.log(`Throughput: ${telemetry.throughput} bytes/sec`)
console.log(`Average latency: ${telemetry.latency} ms`)
```

**Use Cases:**

- Adaptive bitrate streaming decisions
- Network condition monitoring
- Performance debugging and optimization
- Quality of service reporting

### Clock Synchronization

The `AkamaiOffset` utility provides clock synchronization with Akamai's time service:

```typescript
import { AkamaiOffset } from './util/get_akamai_offset'

// Get clock skew relative to Akamai time servers
const clockSkew = await AkamaiOffset.getClockSkew()
console.log(`Local clock is ${clockSkew}ms ahead of network time`)

// Adjust local timestamps for network synchronization
const networkTime = Date.now() - clockSkew
```

**Features:**

- **Network Time Synchronization**: Aligns local time with network time servers
- **RTT Compensation**: Accounts for round-trip time in synchronization calculations
- **Cached Results**: Subsequent calls return cached offset for performance
- **Media Synchronization**: Essential for multi-source media synchronization

These utilities work together to provide a robust foundation for real-time media streaming applications, handling the complex aspects of buffering, network monitoring, and time synchronization.
