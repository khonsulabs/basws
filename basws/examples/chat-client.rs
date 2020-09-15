use basws::{
    client::{async_trait, Client, LoginState, Url, WebsocketClientLogic},
    shared::protocol::{InstallationConfig, ServerError},
};
mod shared;
use shared::chat::{ChatRequest, ChatResponse, PROTOCOL_VERSION, SERVER_PORT};
use std::{fs, path::PathBuf};

struct ChatClient;

#[async_trait]
impl WebsocketClientLogic for ChatClient {
    type Request = ChatRequest;
    type Response = ChatResponse;

    fn server_url(&self) -> Url {
        Url::parse(&format!("ws://localhost:{}/ws", SERVER_PORT)).unwrap()
    }

    fn protocol_version(&self) -> String {
        PROTOCOL_VERSION.to_owned()
    }

    async fn state_changed(&self, state: &LoginState) -> anyhow::Result<()> {
        println!("State Changed: {:#?}", state);
        Ok(())
    }

    async fn stored_installation_config(&self) -> Option<InstallationConfig> {
        println!("Restoring saved installation config");
        serde_json::from_str(&fs::read_to_string(Self::config_path()).ok()?).ok()
    }

    async fn store_installation_config(&self, config: InstallationConfig) -> anyhow::Result<()> {
        println!("Received new installation config: {:?}", config);
        let config_json = serde_json::to_string(&config)?;
        fs::write(Self::config_path(), config_json)?;
        Ok(())
    }

    async fn response_received(
        &self,
        response: Self::Response,
        request_id: i64,
    ) -> anyhow::Result<()> {
        println!("Received response {:?} to request {}", response, request_id);
        Ok(())
    }

    async fn handle_error(&self, error: ServerError) -> anyhow::Result<()> {
        println!("Error from server: {:?}", error);
        Ok(())
    }
}

impl ChatClient {
    fn config_path() -> PathBuf {
        PathBuf::from("client-config.json")
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let client = Client::new(ChatClient);

    client.run().await
}
