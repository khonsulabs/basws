# basws-server

[![crate version](https://img.shields.io/crates/v/basws-server.svg)](https://crates.io/crates/basws-server)

basws-server is a simple WebSocket framework. For more information, see the [basws README](../README.md).

To set up your own protocol server:

- Implement the `ServerLogic` trait
- Create a `Server` passing in your `ServerLogic` implementor
- In your warp filters, call `server.incoming_connection` with the websocket during on_upgrade. Make sure to `move` into closures and `clone()` as needed. The Server is a reference-counted type, so cloning is cheap.
- You can use `server.send_to_installation_id`, `server.send_to_account_id`, and `server.broadcast` to communicate out-of-band with clients.

For a full example, check out the [chat-server](../basws/examples/chat-server.rs) example.
