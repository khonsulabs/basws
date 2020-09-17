use crate::{client::Client, login_state::LoginState};
use async_trait::async_trait;
use basws_shared::{
    protocol::{InstallationConfig, ServerError},
    Version,
};
use serde::{de::DeserializeOwned, Serialize};
use std::fmt::Debug;
use url::Url;

#[derive(Debug)]
pub enum Error {
    Server(ServerError),
    Cbor(serde_cbor::Error),
    Websocket(tokio_tungstenite::tungstenite::Error),
}

#[async_trait]
pub trait ClientLogic: Send + Sync {
    type Request: Serialize + DeserializeOwned + Sync + Send + Clone + Debug;
    type Response: Serialize + DeserializeOwned + Sync + Send + Clone + Debug;

    fn server_url(&self) -> Url;
    fn protocol_version(&self) -> Version;

    async fn state_changed(&self, state: &LoginState, client: Client<Self>) -> anyhow::Result<()>;
    async fn stored_installation_config(&self) -> Option<InstallationConfig>;
    async fn store_installation_config(&self, config: InstallationConfig) -> anyhow::Result<()>;

    async fn response_received(
        &self,
        response: Self::Response,
        original_request_id: Option<u64>,
        client: Client<Self>,
    ) -> anyhow::Result<()>;

    async fn handle_error(&self, error: Error, client: Client<Self>) -> anyhow::Result<()>;
}
