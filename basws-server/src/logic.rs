use crate::{ConnectedClientHandle, ErrorHandling, RequestHandling};
use async_handle::Handle;
use async_trait::async_trait;
use serde::{de::DeserializeOwned, Serialize};
use std::{fmt::Debug, hash::Hash};
use uuid::Uuid;

pub trait Identifiable {
    type Id: Copy + Hash + Eq + Send + Sync;
    fn id(&self) -> Self::Id;
    fn private_key(&self) -> Vec<u8>;
}

#[async_trait]
pub trait WebsocketServerLogic: Send + Sync {
    type Request: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;
    type Response: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;
    type Account: Identifiable<Id = Self::AccountId>
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + Debug;
    type AccountId: Copy + Hash + Eq + Send + Sync;

    async fn handle_request(
        &self,
        client: &ConnectedClientHandle<Self::Response, Self::Account>,
        request: Self::Request,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Handle<Self::Account>>>;

    fn check_protocol_version(&self, version: &str) -> ErrorHandling;

    async fn lookup_or_create_installation(&self, installation_id: Uuid) -> anyhow::Result<()>;

    async fn client_reconnected(
        &self,
        installation_id: Uuid,
        account: Handle<Self::Account>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;

    async fn handle_websocket_error(&self, _err: warp::Error) -> ErrorHandling {
        ErrorHandling::Disconnect
    }

    async fn new_installation_connected(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;
}
