#[macro_use]
extern crate log;

use basws::{
    server::{
        async_trait,
        persistent::{
            PersistentConnectedClient, PersistentServer, PersistentServerHandle,
            PersistentServerLogic,
        },
        Handle, Identifiable, RequestHandling,
    },
    shared::{Uuid, VersionReq},
};
use serde_derive::{Deserialize, Serialize};
use warp::Filter;

pub mod shared;
use shared::chat::{
    protocol_version_requirements, ChatRequest, ChatResponse, ChatSender, SERVER_PORT,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let server = PersistentServer::new(ChatServer, &std::path::PathBuf::from("server.sleddb"))?;

    let routes = warp::path("ws")
        .and(warp::ws())
        .map(move |ws: warp::ws::Ws| {
            // The async closure needs its own reference to the server
            let server = server.clone();
            ws.on_upgrade(|ws| async move { server.incoming_connection(ws).await })
        });

    warp::serve(routes).run(([127, 0, 0, 1], SERVER_PORT)).await;
    Ok(())
}

struct ChatServer;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Account {
    id: Uuid,
    username: String,
}

impl Identifiable for Account {
    type Id = Uuid;
    fn id(&self) -> Self::Id {
        self.id
    }
}

#[async_trait]
impl PersistentServerLogic for ChatServer {
    type Request = ChatRequest;
    type Response = ChatResponse;
    type Client = ();
    type Account = Account;

    async fn initialize_client_for(
        &self,
        _client: &PersistentConnectedClient<Self>,
    ) -> anyhow::Result<Self::Client> {
        Ok(())
    }

    async fn handle_request(
        &self,
        client: &PersistentConnectedClient<Self>,
        request: Self::Request,
        server: &PersistentServerHandle<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        match request {
            ChatRequest::Login { username } => {
                info!("Received login request: {}", username);
                let installation = client.installation().await.unwrap();
                let lower_username = username.to_lowercase();
                let account = if let Some(account_id) = server
                    .load::<Uuid>(b"usernames", lower_username.as_bytes())
                    .await?
                {
                    server
                        .load_account(account_id)
                        .await?
                        .expect("Screennames contained an account_id that couldn't be found")
                } else {
                    // New username
                    let account = Account {
                        id: Uuid::new_v4(),
                        username: username.clone(),
                    };
                    server
                        .save(b"usernames", lower_username.as_bytes(), &account.id)
                        .await?;
                    let account = Handle::new(account);
                    server.save_account(&account).await?;
                    account
                };

                server
                    .associate_installation_with_account(installation.id, account)
                    .await?;

                Ok(RequestHandling::Respond(ChatResponse::LoggedIn {
                    username,
                }))
            }
            ChatRequest::Chat { message } => {
                let from = {
                    if let Some(account) = client.account().await {
                        let account = account.read().await;
                        ChatSender::User(account.username.clone())
                    } else if let Some(installation) = client.installation().await {
                        ChatSender::Anonymous(installation.id)
                    } else {
                        anyhow::bail!("Client did not finish handshake before chatting");
                    }
                };

                server
                    .broadcast(ChatResponse::ChatReceived { from, message })
                    .await;

                Ok(RequestHandling::NoResponse)
            }
        }
    }

    fn protocol_version_requirements(&self) -> VersionReq {
        protocol_version_requirements()
    }

    async fn client_reconnected(
        &self,
        client: &PersistentConnectedClient<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        let installation_id = client.installation().await.unwrap().id;
        if let Some(account) = client.account().await {
            let account = account.read().await;
            info!(
                "Previously authenticated client reconnected: {} ({})",
                account.username, installation_id
            );
            Ok(RequestHandling::Respond(ChatResponse::LoggedIn {
                username: account.username.clone(),
            }))
        } else {
            info!(
                "Previously connected anonymous client reconnected: {}",
                installation_id
            );
            Ok(RequestHandling::Respond(ChatResponse::Unauthenticated))
        }
    }

    async fn new_client_connected(
        &self,
        client: &PersistentConnectedClient<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        info!(
            "New client connected: {}",
            client.installation().await.unwrap().id
        );
        Ok(RequestHandling::Respond(ChatResponse::Unauthenticated))
    }

    async fn client_disconnected(
        &self,
        client: &PersistentConnectedClient<Self>,
    ) -> anyhow::Result<()> {
        info!(
            "Client disconnected: {}",
            client.installation().await.unwrap().id
        );
        Ok(())
    }
}
