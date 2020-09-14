# basws

basws is a simple framework that aims to simplify the amount of code required to build an interactive WebSocket API.

basws is built atop [warp](https://github.com/seanmonstar/warp) on the server, and [yarws](https://github.com/ianic/yarws) on the client. Both crates utilize [tokio](https://tokio.rs/).

## Features

- Built atop [cbor](https://cbor.io/), which has many implementations in various technology stacks
- Basic support for one account logging in on multiple devices
- Easy out-of-band async message sending
- Provides network timing statistics on both the server and client

For a simple example, check out chat example in the [./basws/examples](basws/examples) directory.

## Concepts

- **Installation**: An API client, represented by a [Uuid](https://en.wikipedia.org/wiki/Universally_unique_identifier).
- **Account**: Some data type that has an ID that represents something an Installation can be linked to.
