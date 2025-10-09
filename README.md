# MOQtail

Draft 14-compliant MOQ protocol library with publisher, subscriber and relay components, featuring various live and on-demand demo applications using the LOC and CMAF formats.

## moqtail-ts (MOQtail TypeScript Library)

The TypeScript client library for Media-over-QUIC (MoQ) applications, designed for seamless integration with WebTransport and MoQ relay servers.

### ✨ Features

- 🛡️ **TypeScript**: Type-safe development
- 🔗 **WebTransport**: Next-gen transport protocol support
- 🔥 **Hot Module Reloading**: Instant feedback during development

README available at: [moqtail-ts/README.md](libs/moqtail-ts/README.md)

## 🚀 Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (v18+ recommended)
- [npm](https://www.npmjs.com/)

### Installation

```bash
# Clone the repository (if not already)
git clone https://github.com/moqtail/moqtail.git
cd moqtail

# Install dependencies
npm install
```

## moqtail-rs (MOQtail Rust Library)

The Rust library for Media-over-QUIC (MoQ) applications, providing core protocol functionalities and utilities.

## Relay

The relay is a Rust application that forwards MoQ messages between publishers and subscribers.

```bash
cargo run --bin relay -- --port 4433 --cert-file cert/cert.pem --key-file cert/key.pem
```

### ⚙️ Configuration

- **WebTransport**: Ensure your browser supports WebTransport and that you have trusted the local CA, see the [README.md](apps/relay/cert/README.md) of the relay for instructions.

## 🤝 Contributing

Contributions are welcome! Please open issues or submit pull requests for improvements, bug fixes, or documentation updates.
