#[macro_use]
extern crate log;

mod connected_client;
mod logic;
mod server;
pub use basws_shared as shared;

#[cfg(feature = "persistent-server")]
/// A server implementation with disk persistence built-in. **Requires feature `persistent-server`**
pub mod persistent;

#[cfg(test)]
#[macro_use]
extern crate futures_await_test;

pub use crate::{connected_client::ConnectedClient, logic::*, server::*};
pub use async_handle::Handle;
pub use async_trait::async_trait;
