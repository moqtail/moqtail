# MOQtail FFmpeg Integration Plan

**Goal:** Port FFmpeg MMT-MOQ muxer to MOQtail with Draft 14 compliance and LOC container format

**Timeline:** 2-3 weeks for full implementation and testing

---

## Architecture Overview

```
FFmpeg (SMPTE bars)
    ↓ [H.264 NAL units + PTS]
mmtenc_moq_draft14.c (NEW MUXER)
    ↓ [LOC frames: timestamp + NAL unit]
MOQtail Relay (Draft 14)
    ↓ [Objects with LOC extension headers]
Browser Client
    ↓ [WebCodecs VideoDecoder]
Canvas Rendering ✅
```

### Key Differences from moq.dev

| Component | moq.dev (Draft 07) | MOQtail (Draft 14) |
|-----------|-------------------|-------------------|
| Protocol Version | 0xff000007 | 0xff00000e |
| Container Format | MMTP (12-byte header) | LOC (varint extension headers) |
| Catalog Format | Custom JSON | WARP Streaming Format |
| Relay Implementation | moq-lite (Rust) | moqtail-rs (Rust) |
| Client Library | @moq/hang (TypeScript) | moqtail-ts (TypeScript) |

---

## Phase 1: FFmpeg Muxer (Week 1)

### 1.1 Create New Muxer File

**File:** `libavformat/moqenc_loc.c`

**Purpose:** Replace MMTP container with LOC format

**Key Changes:**
```c
// BEFORE (MMTP):
uint8_t mmtp_header[12];
AV_WB16(mmtp_header + 0, flags);
AV_WB16(mmtp_header + 2, packet_id);
AV_WB32(mmtp_header + 4, timestamp);
AV_WB32(mmtp_header + 8, packet_sequence);

// AFTER (LOC):
uint8_t loc_header[16]; // Max size for 2 varints
int header_len = 0;

// Write CaptureTimestamp extension (ID=1, value=timestamp_us)
header_len += write_varint(loc_header + header_len, 1);  // Extension ID
header_len += write_varint(loc_header + header_len, timestamp_us);

// Write VideoFrameMarking extension (ID=2, value=keyframe)
header_len += write_varint(loc_header + header_len, 2);  // Extension ID
header_len += write_varint(loc_header + header_len, keyframe ? 1 : 0);
```

### 1.2 Varint Encoding Implementation

```c
static int write_varint(uint8_t *buf, uint64_t value) {
    int len = 0;
    // Encode as QUIC varint (1, 2, 4, or 8 bytes)
    if (value < 64) {
        buf[0] = value;
        return 1;
    } else if (value < 16384) {
        buf[0] = 0x40 | (value >> 8);
        buf[1] = value & 0xFF;
        return 2;
    } else if (value < 1073741824) {
        buf[0] = 0x80 | (value >> 24);
        buf[1] = (value >> 16) & 0xFF;
        buf[2] = (value >> 8) & 0xFF;
        buf[3] = value & 0xFF;
        return 4;
    } else {
        buf[0] = 0xC0 | (value >> 56);
        buf[1] = (value >> 48) & 0xFF;
        buf[2] = (value >> 40) & 0xFF;
        buf[3] = (value >> 32) & 0xFF;
        buf[4] = (value >> 24) & 0xFF;
        buf[5] = (value >> 16) & 0xFF;
        buf[6] = (value >> 8) & 0xFF;
        buf[7] = value & 0xFF;
        return 8;
    }
}
```

### 1.3 MOQtail C Client Integration

**Use MOQtail Rust Library via FFI:**

```c
// In moqenc_loc.c
#include "moqtail_ffi.h"

typedef struct MOQtailContext {
    void *client;           // Opaque MOQtail client handle
    void *track;            // Current track
    uint64_t group_id;      // Current group ID
    uint64_t object_id;     // Current object ID
} MOQtailContext;

static int moqtail_write_header(AVFormatContext *s) {
    MOQtailContext *ctx = s->priv_data;

    // Connect to relay
    ctx->client = moqtail_client_connect(relay_url, 0xff00000e); // Draft 14

    // Create track with WARP catalog metadata
    const char *catalog =
        "{"
        "  \"version\": 1,"
        "  \"tracks\": [{"
        "    \"name\": \"video\","
        "    \"render_group\": 1,"
        "    \"packaging\": \"loc\","
        "    \"codec\": \"h264\","
        "    \"width\": 640,"
        "    \"height\": 480,"
        "    \"framerate\": 30"
        "  }]"
        "}";

    ctx->track = moqtail_publish_track(ctx->client, namespace, track_name, catalog);
    return 0;
}

static int moqtail_write_packet(AVFormatContext *s, AVPacket *pkt) {
    MOQtailContext *ctx = s->priv_data;

    // Calculate timestamp in microseconds
    int64_t timestamp_us = av_rescale_q(pkt->pts,
                                        s->streams[pkt->stream_index]->time_base,
                                        (AVRational){1, 1000000});

    // Create LOC frame with extensions
    moqtail_object_t obj = {0};
    obj.group_id = ctx->group_id;
    obj.object_id = ctx->object_id++;
    obj.publisher_priority = pkt->flags & AV_PKT_FLAG_KEY ? 0 : 128;

    // Add LOC extension headers
    moqtail_add_extension_u64(&obj, 1, timestamp_us);      // CaptureTimestamp
    moqtail_add_extension_u64(&obj, 2, pkt->flags & AV_PKT_FLAG_KEY ? 1 : 0); // VideoFrameMarking

    // Add VideoConfig for keyframes (SPS/PPS)
    if (pkt->flags & AV_PKT_FLAG_KEY && s->streams[pkt->stream_index]->codecpar->extradata_size > 0) {
        moqtail_add_extension_bytes(&obj, 4,
                                   s->streams[pkt->stream_index]->codecpar->extradata,
                                   s->streams[pkt->stream_index]->codecpar->extradata_size);
    }

    // Set payload (raw NAL unit)
    obj.payload = pkt->data;
    obj.payload_len = pkt->size;

    // Send to relay
    moqtail_send_object(ctx->track, &obj);

    // New group on keyframe
    if (pkt->flags & AV_PKT_FLAG_KEY) {
        ctx->group_id++;
        ctx->object_id = 0;
    }

    return 0;
}
```

### 1.4 Build MOQtail FFI Bindings

**In MOQtail repo:**

```bash
cd /Users/oramadan/src/github.com/blockcast/moqtail-fork/libs/moqtail-rs
cargo build --release --features ffi

# Generate C header
cbindgen --config cbindgen.toml --crate moqtail --output moqtail_ffi.h
cp moqtail_ffi.h /Users/oramadan/src/github.com/blockcast/FFmpeg-moq/
cp target/release/libmoqtail.a /Users/oramadan/src/github.com/blockcast/FFmpeg-moq/
```

---

## Phase 2: MOQtail Relay Setup (Week 1-2)

### 2.1 Build and Deploy Relay

```bash
cd /Users/oramadan/src/github.com/blockcast/moqtail-fork
cargo build --release --bin relay

# Generate self-signed cert
cd apps/relay/cert
./generate-cert.sh

# Run relay
../../../target/release/relay \
    --port 4433 \
    --cert-file cert/cert.pem \
    --key-file cert/key.pem
```

### 2.2 Configure FFmpeg to Use MOQtail Relay

```bash
ffmpeg -f lavfi -i smptebars=size=640x480:rate=30 \
    -c:v libx264 -preset ultrafast -tune zerolatency \
    -profile:v baseline -level 3.0 \
    -g 30 -keyint_min 30 \
    -f moq_loc \
    -moq_relay "https://localhost:4433" \
    -moq_namespace "smpte" \
    -moq_track "video" \
    "moq://localhost:4433/smpte/video"
```

---

## Phase 3: Browser Client (Week 2)

### 3.1 Install MOQtail TypeScript Client

```bash
mkdir -p /Users/oramadan/src/github.com/blockcast/moqtail-client
cd /Users/oramadan/src/github.com/blockcast/moqtail-client

npm init -y
npm install moqtail-ts
npm install vite
```

### 3.2 Implement WebCodecs Player

**File:** `src/player.ts`

```typescript
import { MOQtailClient, Subscribe, FullTrackName } from 'moqtail-ts';
import { CaptureTimestamp, VideoFrameMarking, VideoConfig } from 'moqtail-ts/extension_header';

class MOQVideoPlayer {
  private client: MOQtailClient;
  private decoder: VideoDecoder;
  private canvas: HTMLCanvasElement;
  private ctx: CanvasRenderingContext2D;

  async init(relayUrl: string, namespace: string, track: string) {
    // Connect to relay
    const wt = new WebTransport(relayUrl);
    await wt.ready;

    const clientSetup = {
      supported_versions: [0xff00000e], // Draft 14
      setup_parameters: []
    };

    this.client = await MOQtailClient.new(clientSetup, wt);

    // Setup VideoDecoder
    this.decoder = new VideoDecoder({
      output: (frame) => this.renderFrame(frame),
      error: (e) => console.error('Decoder error:', e)
    });

    this.decoder.configure({
      codec: 'avc1.42001e', // H.264 Baseline Level 3.0
      optimizeForLatency: true
    });

    // Subscribe to track
    const subscribe = new Subscribe(
      this.client.nextClientRequestId,
      1n, // track_alias
      FullTrackName.tryNew(namespace, track),
      crypto.randomUUID(),
      null, null, // Latest content
      null, null, // Ongoing
      null // No auth
    );

    const stream = await this.client.subscribe(subscribe);
    if (stream instanceof Error) {
      throw stream;
    }

    // Process objects
    const reader = stream.getReader();
    while (true) {
      const { done, value: obj } = await reader.read();
      if (done) break;

      await this.processObject(obj);
    }
  }

  async processObject(obj: MoqtObject) {
    if (!obj.payload || !obj.extensions) return;

    // Parse LOC extensions
    let timestamp = 0;
    let isKeyframe = false;
    let config: Uint8Array | null = null;

    for (const ext of obj.extensions) {
      if (ext instanceof CaptureTimestamp) {
        timestamp = Number(ext.value);
      } else if (ext instanceof VideoFrameMarking) {
        isKeyframe = ext.value > 0;
      } else if (ext instanceof VideoConfig) {
        config = ext.value;
      }
    }

    console.log(`[MOQ] Received object ${obj.location.group}:${obj.location.object}, ` +
                `timestamp=${timestamp}, keyframe=${isKeyframe}, size=${obj.payload.byteLength}`);

    // Update decoder config on keyframe with SPS/PPS
    if (isKeyframe && config) {
      console.log('[MOQ] Updating decoder config with SPS/PPS');
      this.decoder.configure({
        codec: 'avc1.42001e',
        description: config,
        optimizeForLatency: true
      });
    }

    // Decode frame
    const chunk = new EncodedVideoChunk({
      type: isKeyframe ? 'key' : 'delta',
      timestamp: timestamp,
      data: obj.payload
    });

    this.decoder.decode(chunk);
  }

  renderFrame(frame: VideoFrame) {
    console.log(`[WebCodecs] Rendering frame: ${frame.displayWidth}x${frame.displayHeight}`);

    if (!this.canvas) {
      this.canvas = document.querySelector('canvas')!;
      this.ctx = this.canvas.getContext('2d')!;
      this.canvas.width = frame.displayWidth;
      this.canvas.height = frame.displayHeight;
    }

    this.ctx.drawImage(frame, 0, 0);
    frame.close();
  }
}

// Usage
const player = new MOQVideoPlayer();
player.init('https://localhost:4433', 'smpte', 'video');
```

### 3.3 HTML Page

**File:** `index.html`

```html
<!DOCTYPE html>
<html>
<head>
  <title>MOQtail Video Player</title>
</head>
<body>
  <h1>MOQtail SMPTE Bars Test</h1>
  <canvas id="video"></canvas>
  <div id="stats"></div>
  <script type="module" src="/src/player.ts"></script>
</body>
</html>
```

### 3.4 Dev Server

**File:** `vite.config.ts`

```typescript
import { defineConfig } from 'vite';

export default defineConfig({
  server: {
    port: 5173,
    headers: {
      'Cross-Origin-Opener-Policy': 'same-origin',
      'Cross-Origin-Embedder-Policy': 'require-corp'
    }
  }
});
```

---

## Phase 4: Testing (Week 2-3)

### 4.1 Unit Tests

**Test LOC encoding:**
```c
// In FFmpeg tests
void test_loc_varint_encoding() {
    uint8_t buf[16];
    assert(write_varint(buf, 63) == 1);
    assert(buf[0] == 63);

    assert(write_varint(buf, 16383) == 2);
    assert(buf[0] == 0x7F && buf[1] == 0xFF);
}

void test_loc_frame_creation() {
    // Test CaptureTimestamp + VideoFrameMarking extensions
    uint8_t frame[32];
    int len = 0;

    len += write_varint(frame + len, 1);      // CaptureTimestamp ID
    len += write_varint(frame + len, 1000000); // 1 second
    len += write_varint(frame + len, 2);      // VideoFrameMarking ID
    len += write_varint(frame + len, 1);      // Keyframe

    assert(len <= 32);
}
```

### 4.2 Integration Test Script

**File:** `/tmp/moqtail-test.mjs`

```javascript
import puppeteer from 'puppeteer-core';

const browser = await puppeteer.launch({
  executablePath: '/Users/oramadan/src/chromium.googlesource.com/chromium/src/out/Default/Chromium.app/Contents/MacOS/Chromium',
  headless: false,
  args: [
    '--enable-features=WebCodecs,WebTransport',
    '--origin-to-force-quic-on=localhost:4433',
    '--enable-quic',
    '--ignore-certificate-errors'
  ]
});

const page = await browser.newPage();
page.on('console', msg => console.log(`[BROWSER] ${msg.text()}`));

await page.goto('https://localhost:5173/');
await new Promise(r => setTimeout(r, 30000)); // Wait 30 seconds

// Take 3 screenshots 10 seconds apart
for (let i = 1; i <= 3; i++) {
  console.log(`\n📸 Screenshot ${i}/3`);

  const analysis = await page.evaluate(() => {
    const c = document.querySelector('canvas');
    const ctx = c.getContext('2d');
    const img = ctx.getImageData(0, 0, c.width, c.height);
    let colored = 0;
    for (let i = 0; i < img.data.length; i += 4) {
      const r = img.data[i], g = img.data[i+1], b = img.data[i+2];
      if (Math.abs(r-g) > 20 || Math.abs(g-b) > 20) colored++;
    }
    return { colored, total: img.data.length/4 };
  });

  console.log(`  Colored pixels: ${analysis.colored}/${analysis.total}`);

  if (analysis.colored > 1000) {
    console.log(`  ✅ SUCCESS! SMPTE bars rendering!`);
  }

  await page.screenshot({ path: `/tmp/moqtail-${i}.png` });

  if (i < 3) await new Promise(r => setTimeout(r, 10000));
}

await browser.close();
```

### 4.3 Run End-to-End Test

```bash
# Terminal 1: Start MOQtail relay
cd /Users/oramadan/src/github.com/blockcast/moqtail-fork
./target/release/relay --port 4433 --cert-file apps/relay/cert/cert.pem --key-file apps/relay/cert/key.pem

# Terminal 2: Start FFmpeg publisher
cd /Users/oramadan/src/github.com/blockcast/FFmpeg-moq
./ffmpeg -f lavfi -i smptebars=size=640x480:rate=30 \
    -c:v libx264 -preset ultrafast -tune zerolatency \
    -f moq_loc moq://localhost:4433/smpte/video

# Terminal 3: Start web server
cd /Users/oramadan/src/github.com/blockcast/moqtail-client
npm run dev

# Terminal 4: Run test
node /tmp/moqtail-test.mjs
```

---

## Success Criteria

✅ **3 screenshots showing SMPTE color bars rendered in browser**
✅ All screenshots taken 10 seconds apart
✅ Colored pixel count > 1000 in each screenshot
✅ Timestamps incrementing properly
✅ Keyframes delivering SPS/PPS via VideoConfig extension
✅ WebCodecs decoder producing valid frames
✅ Canvas showing 640x480 video

---

## Risk Mitigation

### If MOQtail Rust FFI is too complex:
- **Plan B:** Use MOQtail relay + write FFmpeg muxer to output LOC files, then publish via Node.js script

### If Draft 14 incompatibility issues:
- **Plan B:** Contribute patches to MOQtail to add backward compatibility mode

### If WebCodecs issues persist:
- **Plan B:** Use FFmpeg.wasm in browser as mentioned by user

---

## Timeline

| Week | Tasks | Deliverables |
|------|-------|--------------|
| 1 | Phase 1: FFmpeg muxer + MOQtail FFI | Working `moqenc_loc.c` muxer |
| 1-2 | Phase 2: Relay setup + integration | FFmpeg publishing to MOQtail relay |
| 2 | Phase 3: Browser client with WebCodecs | Working video player |
| 2-3 | Phase 4: Testing + debugging | 3 screenshots of SMPTE bars |

---

## Next Steps

1. ✅ Fork MOQtail repository
2. ✅ Explore architecture and LOC format
3. 🔄 **CURRENT:** Design integration strategy
4. ⏭️ Implement MOQtail FFI bindings
5. ⏭️ Create FFmpeg muxer with LOC support
6. ⏭️ Build browser client
7. ⏭️ Run end-to-end tests

---

**Notes:**
- This plan leverages MOQtail's Draft 14 compliance and LOC format
- Parallel work with main agent debugging moq.dev WebCodecs issues
- User mentioned FFmpeg WASM solution suggests browser-side decoding is proven to work
- Focus is on getting clean LOC frames to browser via MOQtail relay
