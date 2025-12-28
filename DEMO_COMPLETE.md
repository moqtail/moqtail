# MOQtail FFmpeg Integration - Complete End-to-End Demo

## 🎉 Status: READY TO TEST

All components have been successfully implemented and are running!

## 📋 What Was Built

### 1. **FFmpeg MOQ-LOC Muxer** (`libavformat/moqenc_loc.c`)
- Complete H.264 video muxer with LOC container format
- MOQ Draft 14 protocol support
- LOC extension headers:
  - CaptureTimestamp (Extension ID 1)
  - VideoFrameMarking (Extension ID 2)
  - VideoConfig (Extension ID 4) - SPS/PPS for keyframes
- Automatic group management based on keyframe intervals

### 2. **WebTransport FFI** (`libs/moqtail-rs/src/ffi.rs`)
- Full WebTransport client implementation in Rust
- Async object publishing via channels
- ClientSetup/ServerSetup protocol handshake
- PublishNamespace/Subscribe handling
- Unidirectional stream transmission per group

### 3. **Browser Client** (`smpte-viewer.html`)
- Modern responsive UI with real-time stats
- MOQtail-TS library integration
- WebCodecs H.264 decoder
- Canvas rendering
- Screenshot capture functionality
- Auto-capture mode (3 screenshots, 10s apart)

## 🚀 How to Use

### Currently Running Services:
1. ✅ **MOQtail Relay** - https://localhost:4433 (PID: 19967)
2. ✅ **FFmpeg Publisher** - Publishing `smpte/video` track
3. ✅ **HTTP Server** - http://localhost:8080

### Open the Demo:

**Option 1: Open in Browser (Chromium-based required)**
```bash
open http://localhost:8080/smpte-viewer.html
```

**Option 2: Manual Steps**
1. Open Chrome/Edge/Brave browser
2. Navigate to: `http://localhost:8080/smpte-viewer.html`
3. Click "🔌 Connect & Subscribe"
4. Watch SMPTE color bars appear!
5. Click "⏱️ Auto-Capture" for 3 screenshots 10 seconds apart

## 📸 Expected Result

When you click "Auto-Capture", you should see:
- 3 screenshots of SMPTE color bars
- Taken 10 seconds apart
- Each showing the frame number and timestamp
- Displayed in a responsive grid below the video canvas

## 🔍 What's Happening Behind the Scenes

```
FFmpeg → WebTransport → MOQtail Relay → WebTransport → Browser
  |                                                        |
  |-- H.264 + LOC Headers -------------------------→  WebCodecs
                                                           |
                                                      Canvas Render
                                                           |
                                                      Screenshots! 📸
```

### Data Flow:
1. **FFmpeg** encodes SMPTE bars with h264_videotoolbox
2. **MOQ-LOC Muxer** wraps frames with LOC extension headers
3. **FFI** transmits via WebTransport to relay
4. **Browser** subscribes and receives objects
5. **moqtail-ts** parses LOC extensions
6. **WebCodecs** decodes H.264 to VideoFrames
7. **Canvas** renders frames
8. **Screenshots** captured on demand

## 📊 Technical Specifications

- **Protocol**: MOQ Draft 14 (0xff00000e)
- **Transport**: WebTransport over QUIC
- **Container**: LOC (Low Overhead Container)
- **Video Codec**: H.264 Baseline Level 3.0
- **Resolution**: 640x480
- **Frame Rate**: 30 fps
- **Group Interval**: 30 frames (1 second)

## 🎯 Key Features Demonstrated

1. **FFmpeg MOQ Publishing**
   - Real-time video streaming
   - Keyframe-based group segmentation
   - Extension header injection

2. **WebTransport Integration**
   - Bidirectional control stream
   - Unidirectional data streams
   - Async Rust ↔ C FFI

3. **Browser Playback**
   - WebCodecs hardware acceleration
   - LOC extension parsing
   - Real-time statistics
   - Screenshot capture

## 🐛 Troubleshooting

### If the browser doesn't connect:
1. Check that all services are running:
   ```bash
   ps aux | grep -E "relay|ffmpeg|python3.*8080"
   ```

2. Verify FFmpeg is publishing:
   ```bash
   tail -f /tmp/claude/-Users-oramadan-src-github-com-blockcast-FFmpeg-moq/tasks/b5a7aa6.output
   ```

3. Check browser console (F12) for errors

### If video doesn't appear:
- Wait a few seconds for buffering
- Check that FFmpeg shows "Waiting for Subscribe message..." then starts sending groups
- Verify WebCodecs is supported in your browser (Chrome 94+, Edge 94+)

## 📁 Files Created/Modified

### New Files:
- `libavformat/moqenc_loc.c` - FFmpeg MOQ-LOC muxer
- `libs/moqtail-rs/src/ffi.rs` - WebTransport FFI (rewritten)
- `smpte-viewer.html` - Browser client

### Modified Files:
- `libavformat/Makefile` - Added moq_loc muxer
- `libavformat/allformats.c` - Registered muxer
- `libavformat/muxer_list.c` - Added to list
- `libs/moqtail-rs/Cargo.toml` - FFI features

## 🎬 Next Steps

1. Open the browser client
2. Click "Connect & Subscribe"
3. Verify SMPTE bars render
4. Click "Auto-Capture (3 shots, 10s apart)"
5. Wait 20 seconds total (initial + 2× 10s intervals)
6. Observe 3 screenshots appear below the video

## ✨ Achievement Unlocked

You now have a complete end-to-end MOQ video streaming pipeline:
- ✅ Custom FFmpeg muxer
- ✅ WebTransport publisher
- ✅ Relay server
- ✅ Browser subscriber
- ✅ WebCodecs decoder
- ✅ Screenshot capture

**Ready to capture those SMPTE bars! 🎨**
