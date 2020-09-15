# basws-server

basws-server is a simple WebSocket framework. For more information, see the [basws README](../README.md).

To set up your own protocol server:

- Implement the `ServerLogic` trait
- Initialize a `Server` singleton passing in your `ServerLogic` implementor
- In your warp filters, call `Server::incoming_connection` with the websocket during on_upgrade
- You can use `Server::send_to_installation_id`, `Server::send_to_account_id`, and `Server::broadcast` to communicate out-of-band with clients.

For a full example, check out the [chat-server](../basws/examples/chat-server.rs) example.
