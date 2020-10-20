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

### Server

Add either of these lines to your Cargo.toml:

```toml
# Either use the basws parent crate
basws = { version = ">=0.1.0-dev-8", features = ["server"] }
# Or, use the basws-server crate
basws-server = ">=0.1.0-dev-8"
```

### Client

Add either of these lines to your Cargo.toml:

```toml
# Either use the basws parent crate
basws = { version = ">=0.1.0-dev-8", features = ["client"] }
# Or, use the basws-client crate
basws-client = ">=0.1.0-dev-8"
```
