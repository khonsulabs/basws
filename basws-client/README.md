# basws-client

basws-client is a simple WebSocket framework. For more information, see the [basws README](../README.md).

To set up your own protocol client:

- Implement the `ClientLogic` trait
- Create a `Client` passing in your `ClientLogic` implementor
- Spawn the client by either `client.run().await` or `client.spawn()`
- You can clone the client and pass it around in your application as needed

For a full example, check out the [chat-client](../basws/examples/chat-client.rs) example.
