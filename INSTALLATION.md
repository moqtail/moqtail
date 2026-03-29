# MOQtail Installation Guide

Complete setup guide for running the MOQtail stack natively on **Windows**.

## Table of Contents

- [Prerequisites](#prerequisites)
- [Install Dependencies via vcpkg](#install-dependencies-via-vcpkg)
- [Environment Configuration](#environment-configuration)
- [Clone and Install](#clone-and-install)
- [TLS Certificate Setup](#tls-certificate-setup)
- [Building the Project](#building-the-project)
- [Running the Stack](#running-the-stack)
- [Running Components Manually](#running-components-manually)
- [Video Encoding](#video-encoding)
- [Troubleshooting](#troubleshooting)

---

## Prerequisites

### 1. Visual Studio Build Tools 2022

Download from [visualstudio.microsoft.com](https://visualstudio.microsoft.com/downloads/) and install the **Desktop development with C++** workload.

### 2. Rust

Download and run the installer from [rustup.rs](https://rustup.rs/). Select the **MSVC** toolchain when prompted.

### 3. Node.js

Install Node.js v18+ from [nodejs.org](https://nodejs.org/).

### 4. Scoop

Open PowerShell and run:

```powershell
Set-ExecutionPolicy -ExecutionPolicy RemoteSigned -Scope CurrentUser
Invoke-RestMethod -Uri https://get.scoop.sh | Invoke-Expression
```

### 5. LLVM, NASM, and mkcert via Scoop

```powershell
scoop install llvm nasm mkcert
```

- **LLVM**: Required for `bindgen`
- **NASM**: Required for FFmpeg compilation
- **mkcert**: Required for local TLS certificates

---

## Install Dependencies via vcpkg

```powershell
git clone https://github.com/microsoft/vcpkg.git $HOME\vcpkg
cd $HOME\vcpkg
.\bootstrap-vcpkg.bat
.\vcpkg install 'ffmpeg[x265,x264,vpx,opus,mp3lame,dav1d,amf]:x64-windows' libjpeg-turbo:x64-windows --recurse
```

> The `vcpkg install` step compiles FFmpeg and its codecs from source and will take 20–40 minutes.

The enabled codec features are:

| Feature | Purpose |
|---------|---------|
| `x265` | H.265/HEVC encoder (required by publisher) |
| `x264` | H.264 encoder |
| `dav1d` | AV1 decoder (required if source video is AV1-encoded) |
| `vpx` | VP8/VP9 codec support |
| `opus` | Opus audio codec |
| `mp3lame` | MP3 encoding support |
| `amf` | AMD AMF hardware encoder support |

After installation, register vcpkg system-wide (one-time):

```powershell
& "$HOME\vcpkg\vcpkg.exe" integrate install
```

> If you reinstall or update vcpkg packages after an initial build, always run `cargo clean` before rebuilding. Cargo will not automatically relink against updated libraries if the Rust source is unchanged.

---

## Environment Configuration

A setup script is provided at `scripts\win-env-setup.ps1`. It automatically resolves paths for your user account.

Run it in every new terminal before building or running the project:

```powershell
. .\scripts\win-env-setup.ps1
```

The script sets the following variables:

| Variable | Purpose |
|----------|---------|
| `FFMPEG_DIR` | Root vcpkg install path |
| `FFMPEG_INCLUDE_DIR` | FFmpeg headers |
| `FFMPEG_LIB_DIR` | FFmpeg import libraries |
| `PATH` | vcpkg `bin` directory (prevents `STATUS_DLL_NOT_FOUND`) |
| `LIBCLANG_PATH` | Required by `bindgen` to locate `libclang.dll` |

---

## Clone and Install

```powershell
git clone https://github.com/BaylorMultimediaLab/moqtail.git
cd moqtail
```

### Git submodule (test video data)

The test videos are stored in a Git submodule hosted on `gitlab.ecs.baylor.edu`. You need access to clone it:

```powershell
git lfs install
git submodule update --init --recursive
cd data; git lfs pull; cd ..
```

The submodule uses **Git LFS** for large files (`.mp4`, `.onnx`, etc.). Without `git lfs pull`, you will only get small pointer files instead of the actual data.

If the submodule clone hangs, Git may be waiting for credentials silently. You can either:

- **Embed credentials in the URL:**
  1. Create a Personal Access Token on `https://gitlab.ecs.baylor.edu/-/user_settings/personal_access_tokens` with `read_repository` scope
  2. Run:
     ```powershell
     git config submodule.data.url https://USERNAME:TOKEN@gitlab.ecs.baylor.edu/freeman-multimedia-lab/distrubuted-content-analysis/data.git
     git submodule update --init --recursive
     ```
- **Skip the submodule** and supply your own H.264 MP4 video file:
  ```powershell
  New-Item -ItemType Directory -Force -Path data\video
  Copy-Item C:\path\to\your\video.mp4 data\video\smoking_test_1080p.mp4
  ```

### Node dependencies

From the repo root:

```powershell
npm install
```

---

## TLS Certificate Setup

WebTransport requires TLS certificates. Place `cert.pem` and `key.pem` in `apps\relay\cert\`.

### 1. Install the local Certificate Authority (one-time)

```powershell
mkcert -install
```

Click **Yes** on the UAC prompt to add the CA to your Windows trust store.

### 2. Generate the relay certificate

Run from the repo root:

```powershell
cd apps\relay\cert
mkcert -key-file key.pem -cert-file cert.pem localhost 127.0.0.1 ::1
cd ..\..\..
```

### 3. Enable WebTransport in Chrome

1. Navigate to `chrome://flags/#webtransport-developer-mode`
2. Set it to **Enabled**
3. Restart Chrome

> Firefox and Edge instructions are pending. Chrome is the only fully tested browser.

---

## Building the Project

### Rust components

```powershell
. .\scripts\win-env-setup.ps1
cargo build --release
```

### TypeScript client

Build from the repo root — this builds `libs/moqtail-ts` first, then `apps/client-js`:

```powershell
npm run build
```

> Do not run `npm run build` inside `apps/client-js` directly — the `moqtail` library must be built first.

---

## Running the Stack

Use the provided PowerShell script to start all components at once:

```powershell
. .\scripts\win-env-setup.ps1
.\scripts\win-run-stack.ps1
```

To use a custom video file:

```powershell
.\scripts\win-run-stack.ps1 "data\video\Smoking Test.mp4"
```

To stop all components:

```powershell
.\scripts\win-run-stack.ps1 stop
```

Logs are written to `logs\` with one file per component.

| Component | URL |
|-----------|-----|
| Relay | https://localhost:4433 |
| Client-JS | http://localhost:5173 |
| Publisher | (connects to relay) |

---

## Running Components Manually

Open a separate PowerShell terminal for each component and run `. .\scripts\win-env-setup.ps1` in each.

### Relay Server

```powershell
cargo run --bin relay -- --port 4433 --cert-file apps/relay/cert/cert.pem --key-file apps/relay/cert/key.pem
```

### Video Publisher

```powershell
cargo run --bin publisher -- --video-path '.\data\video\Smoking Test.mp4'
```

**Publisher flags** (all optional):

| Flag | Default | Description |
|------|---------|-------------|
| `--max-variants <2-4>` | `4` | Number of ABR quality variants to encode |
| `--namespace <name>` | `moqtail` | Track namespace |
| `--target-latency-ms <ms>` | `1500` | Target playback latency in milliseconds |
| `--validate-cert` | off | Validate TLS certificate |

### Viewer

```powershell
cargo run --bin moqtail-client -- --mode viewer --skip-cert-validation
```

### Analyzer

> Requires a Linux machine with an NVIDIA GPU and CUDA. Not supported on Windows.

---

## Video Encoding

The publisher decodes the source video and re-encodes all ABR variants using **H.265/HEVC (libx265)** via software encoding. Hardware encoders are used automatically if detected.

The source video can be in any format supported by the installed FFmpeg codecs, including H.264, H.265, AV1, VP8/VP9, and more.

### ABR Variants

The publisher generates up to 4 adaptive bitrate variants:

| Variant | Resolution | Bitrate |
|---------|-----------|---------|
| 1080p | 1920x1080 | 4000 kbps |
| 720p | 1280x720 | 2000 kbps |
| 480p | 854x480 | 1000 kbps |
| 360p | 640x360 | 500 kbps |

Use `--max-variants <2-4>` to limit the number of variants encoded (fewer = less CPU/memory usage).

---

## Troubleshooting

| Problem | Fix |
|---------|-----|
| `STATUS_DLL_NOT_FOUND (0xc0000135)` | Run `. .\scripts\win-env-setup.ps1` to add the vcpkg `bin` directory to PATH |
| FFmpeg not found during build | Check `$env:FFMPEG_DIR` is set — if empty, run the setup script and retry |
| `Cannot find module 'moqtail'` | Run `npm run build` from the repo root, not from inside `apps/client-js` |
| Relay fails with cert `NotFound` | Generate certificates (see [TLS Certificate Setup](#tls-certificate-setup)) |
| Publisher says `box with a larger size` | Video file is a Git LFS pointer — run `cd data; git lfs pull; cd ..` |
| Submodule clone hangs | Git is waiting for credentials — see [Clone and Install](#clone-and-install) |
| `Opening handshake failed` | Certificate not trusted — ensure `mkcert -install` was run and WebTransport Developer Mode is enabled in Chrome |
| `Catalog stream unavailable` | Publisher is not running or crashed — check `logs\publisher_*.log` and restart the stack |
| `encoder 'libx265' not found` or `Decoder failed: Function not implemented` | FFmpeg is missing a required codec — reinstall with the full feature set (see [Install Dependencies via vcpkg](#install-dependencies-via-vcpkg)), then `cargo clean` and `cargo build --release` |
| `VCPKG_ROOT` not found / FFmpeg not found | You opened a new terminal without sourcing the env script — run `. .\scripts\win-env-setup.ps1` first |
