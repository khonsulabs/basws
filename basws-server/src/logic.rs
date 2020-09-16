use crate::{connected_client::ConnectedClient, ErrorHandling, RequestHandling, Server};
use async_handle::Handle;
use async_trait::async_trait;
use basws_shared::{protocol::InstallationConfig, Uuid, VersionReq};
use serde::{de::DeserializeOwned, Serialize};
use std::{fmt::Debug, hash::Hash};

pub trait Identifiable {
    type Id: Copy + Hash + Eq + Send + Sync;
    fn id(&self) -> Self::Id;
}

#[async_trait]
pub trait ServerLogic: Send + Sync {
    type Request: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;
    type Response: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;
    type Client: Send + Sync + Debug;
    type Account: Identifiable<Id = Self::AccountId> + Send + Sync + Debug;
    type AccountId: Copy + Hash + Eq + Send + Sync;

    async fn handle_request(
        &self,
        client: &ConnectedClient<Self>,
        request: Self::Request,
        server: &Server<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Handle<Self::Account>>>;

    fn protocol_version_requirements(&self) -> VersionReq;

    async fn lookup_or_create_installation(
        &self,
        installation_id: Option<Uuid>,
    ) -> anyhow::Result<InstallationConfig>;

    async fn client_reconnected(
        &self,
        installation_id: Uuid,
        account: Option<Handle<Self::Account>>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;

    async fn new_installation_connected(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;

    async fn handle_websocket_error(&self, _err: warp::Error) -> ErrorHandling {
        ErrorHandling::Disconnect
    }
}
