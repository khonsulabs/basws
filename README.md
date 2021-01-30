# basws

[![crate version](https://img.shields.io/crates/v/basws.svg)](https://crates.io/crates/basws)

basws is a simple framework that aims to simplify the amount of code required to build an interactive WebSocket API.

basws is built atop [warp](https://github.com/seanmonstar/warp) on the server, and [tokio-tungstenite](https://github.com/snapview/tokio-tungstenite) on the client. Both crates utilize the [tokio](https://tokio.rs/) runtime.

## Features

- Built atop [cbor](https://cbor.io/), which has many implementations in various technology stacks
- Basic support for one account logging in on multiple devices
- Easy out-of-band async message sending
- Provides network timing statistics on both the server and client

For a simple example, check out chat example in the [./basws/examples](basws/examples) directory.

## Usage

### Environment

### Server

Add either of these lines to your Cargo.toml:

```toml
# Either use the basws parent crate
basws = { version = "0.1", features = ["server"] }
# Or, use the basws-server crate
basws-server = "0.1"
```

### Client

Add either of these lines to your Cargo.toml:

```toml
# Either use the basws parent crate
basws = { version = "0.1", features = ["client"] }
# Or, use the basws-client crate
basws-client = "0.1"
```

### Yew

Add either of these lines to your Cargo.toml:

```toml
# Either use the basws parent crate
basws = { version = "0.1", features = ["yew"] }
# Or, use the basws-client crate
basws-yew = "0.1"
```

When building for release, an environment variable needs to be present: `BASWS_CLIENT_ENCRYPTION_KEY`. This should be a 32-character string. It is used to encrypt the session information stored in the browser. If you ever suspect that you need to invalidate existing session information, rotating this key will cause every user to start a new session.
