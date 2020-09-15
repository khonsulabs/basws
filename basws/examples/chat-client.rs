#[macro_use]
extern crate log;

use basws::{
    client::{async_trait, Client, LoginState, Url, WebsocketClientLogic},
    shared::protocol::{InstallationConfig, ServerError},
};
mod shared;
use basws_shared::protocol::ServerRequest;
use rand::{seq::SliceRandom, thread_rng, Rng};
use shared::chat::{ChatRequest, ChatResponse, PROTOCOL_VERSION, SERVER_PORT};
use std::{fs, path::PathBuf};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let args: Vec<_> = std::env::args().collect();
    let config_path = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| "client-config.json".to_string());

    let client = Client::new(ChatClient { config_path });

    tokio::spawn(random_chat_loop(client.clone()));

    client.run().await
}

async fn random_chat_loop(client: Client<ChatClient>) {
    loop {
        trace!("Attempting to send chat message");
        if let LoginState::Connected { .. } = client.login_state().await {
            trace!("Socket is connected");
            let message = {
                let mut rng = thread_rng();
                ["hi", "hello", "hola", "howdy"]
                    .choose(&mut rng)
                    .unwrap()
                    .to_string()
            };
            let _ = client
                .request(ServerRequest::Request(ChatRequest::Chat { message }))
                .await;
        }
        let sleep_time = {
            let mut rng = thread_rng();
            rng.gen_range(500, 2000)
        };
        trace!("Sleeping for {} before sending next message", sleep_time);
        tokio::time::delay_for(tokio::time::Duration::from_millis(sleep_time)).await
    }
}

struct ChatClient {
    config_path: String,
}

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

    async fn state_changed(&self, state: &LoginState, _client: Client<Self>) -> anyhow::Result<()> {
        info!("State Changed: {:#?}", state);
        Ok(())
    }

    async fn stored_installation_config(&self) -> Option<InstallationConfig> {
        trace!("Restoring saved installation config");
        serde_json::from_str(&fs::read_to_string(self.config_path()).ok()?).ok()
    }

    async fn store_installation_config(&self, config: InstallationConfig) -> anyhow::Result<()> {
        trace!("Received new installation config: {:?}", config);
        let config_json = serde_json::to_string(&config)?;
        fs::write(self.config_path(), config_json)?;
        Ok(())
    }

    async fn response_received(
        &self,
        response: Self::Response,
        original_request_id: Option<u64>,
        client: Client<Self>,
    ) -> anyhow::Result<()> {
        trace!(
            "Received response {:?} to request {:?}",
            response,
            original_request_id
        );
        match response {
            ChatResponse::Unauthenticated => {
                // This fake chat client will just choose a random name
                let name = {
                    let mut rng = thread_rng();
                    ["jon", "jane", "bob", "mary"].choose(&mut rng).unwrap()
                };

                client
                    .request(ServerRequest::Request(ChatRequest::Login {
                        username: name.to_string(),
                    }))
                    .await?;
            }
            ChatResponse::LoggedIn { username } => {
                info!("Successfully logged in as {}", username);
            }
            ChatResponse::ChatReceived { from, message } => println!("{}: {}", from, message),
        }
        Ok(())
    }

    async fn handle_server_error(
        &self,
        error: ServerError,
        _client: Client<Self>,
    ) -> anyhow::Result<()> {
        error!("Error from server: {:?}", error);
        Ok(())
    }
}

impl ChatClient {
    fn config_path(&self) -> PathBuf {
        PathBuf::from(&self.config_path)
    }
}
