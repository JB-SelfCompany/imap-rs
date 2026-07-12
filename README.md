<div align="center">

# imap-rs

</div>

An IMAP4rev1 server in Rust: a wire-level protocol library (`imap-core`) and an
async server framework (`imap-server`). A port of [go-imap v2][goimap] to Tokio.

[goimap]: https://github.com/emersion/go-imap

## Crates

| Crate | Purpose | Dependencies |
|-------|---------|--------------|
| `imap-core` | Protocol types + wire codec (encoder/decoder) | `tokio` |
| `imap-server` | Connection state machine + `Backend`/`UserSession` traits | `imap-core`, `tokio`, `tokio-rustls`, `async-trait`, `tracing` |

## Design

- **Hand-written codec, no parser generators.** The decoder and encoder in
  `imap-core/src/codec.rs` are written by hand (no `nom`, no `combine`), mirroring
  go-imap v2. Atoms, quoted strings, literals (`{N}\r\n`), parenthesized lists and
  flag lists; 50 KiB command limit.
- **No error-handling helper crates.** `ImapError` is a plain enum with manual
  `Display`/`Error`/`From` impls — no `thiserror`, no `anyhow`.
- **Two-trait extension contract.** Implement `Backend` (session factory, shared via
  `Arc`) and `UserSession` (per-connection state) to back the server with your own
  storage. Extension methods (`move_messages`, `idle`, `namespace`) have defaults.

## Usage

```rust
use std::sync::Arc;
use imap_server::Server;

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let backend = Arc::new(MyBackend::new());   // impl Backend + UserSession
    Server::new(backend).listen("127.0.0.1:143").await
}
```

`Backend::login` returns a `Box<dyn UserSession>`; the session implements the
mailbox and message operations (`list`, `select`, `fetch`, `store`, `search`,
`copy`, `expunge`, `append`). See `examples/memstore.rs` for a complete in-memory
implementation.

```bash
cargo run --example memstore        # in-memory server on 127.0.0.1:11443
```

## Capabilities

Advertised by default: `IMAP4rev1`, `LITERAL+`, `IDLE`, `MOVE`, `NAMESPACE`,
`UIDPLUS`. `STARTTLS` is advertised only when the backend supplies a TLS config.

- **Implicit TLS** (port 993) via `Server::listen_tls` — implemented.
- **STARTTLS** (port 143) — advertised but returns `BAD` (stub); the stream halves
  are type-erased so it can be added without reworking the connection type.
- **IDLE** emits untagged `* N EXISTS` when the selected mailbox grows, driven by
  `UserSession::current_message_count` (RFC 2177).

## Connection states

```
NotAuthenticated ──LOGIN──▶ Authenticated ──SELECT──▶ Selected
                                              ▲          │
                                              └──CLOSE───┘
All states ──LOGOUT──▶ Logout
```

Enforced by `require_auth` / `require_selected`, which return `BAD` when violated.

## Build & test

Requirements:

- **Rust** stable — install via [rustup](https://rustup.rs)
- **A C compiler and CMake** — `imap-server` pulls `rustls`/`tokio-rustls`, whose
  default `aws-lc-rs` crypto backend builds native code (on Windows also install NASM).
  `imap-core` alone (tokio only) needs no C toolchain.

```bash
cargo build -p imap-core -p imap-server
cargo test
```

## License

MIT