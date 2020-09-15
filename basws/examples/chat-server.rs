#[macro_use]
extern crate log;
use basws::{
    server::{
        async_trait, ConnectedClient, Handle, Identifiable, RequestHandling, Server, ServerLogic,
    },
    shared::{protocol::InstallationConfig, Uuid, VersionReq},
};
use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;
use warp::Filter;

pub mod shared;
use shared::chat::{
    protocol_version_requirements, ChatRequest, ChatResponse, ChatSender, SERVER_PORT,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    pretty_env_logger::init();
    let server = Server::new(ChatServer::default());

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

#[derive(Default)]
struct ChatServer {
    accounts: Handle<HashMap<i64, Handle<Account>>>,
    screennames: Handle<HashMap<String, i64>>,
    installations: Handle<HashMap<Uuid, InstallationConfig>>,
    installation_accounts: Handle<HashMap<Uuid, i64>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Account {
    id: i64,
    username: String,
}

impl Identifiable for Account {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id
    }
}

#[async_trait]
impl ServerLogic for ChatServer {
    type Request = ChatRequest;
    type Response = ChatResponse;
    type Account = Account;
    type AccountId = i64;

    async fn handle_request(
        &self,
        client: &ConnectedClient<Self::Response, Self::Account>,
        request: Self::Request,
        server: &Server<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        match request {
            ChatRequest::Login { username } => {
                info!("Received login request: {}", username);
                let installation = client.installation().await.unwrap();
                let mut screennames = self.screennames.write().await;
                let account = if let Some(&account_id) = screennames.get(&username) {
                    let accounts = self.accounts.read().await;
                    accounts
                        .get(&account_id)
                        .expect("Screennames contained an account_id that couldn't be found")
                        .clone()
                } else {
                    // New username
                    let mut accounts = self.accounts.write().await;
                    let id = accounts.len() as i64;
                    let account = Account {
                        id,
                        username: username.clone(),
                    };
                    let account = Handle::new(account);
                    accounts.insert(id, account.clone());
                    screennames.insert(username.clone(), id);
                    account
                };

                server
                    .associate_installation_with_account(installation.id, account)
                    .await;

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

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Handle<Self::Account>>> {
        trace!("Looking up account: {}", installation_id);
        let installation_accounts = self.installation_accounts.read().await;

        let account = if let Some(account_id) = installation_accounts.get(&installation_id) {
            let accounts = self.accounts.read().await;
            accounts.get(account_id).cloned()
        } else {
            None
        };

        Ok(account)
    }

    fn protocol_version_requirements(&self) -> VersionReq {
        protocol_version_requirements()
    }

    async fn lookup_or_create_installation(
        &self,
        installation_id: Option<Uuid>,
    ) -> anyhow::Result<InstallationConfig> {
        let mut installations = self.installations.write().await;
        if let Some(installation_id) = installation_id {
            if let Some(installation) = installations.get(&installation_id) {
                return Ok(*installation);
            }
        }

        let config = InstallationConfig::default();
        installations.insert(config.id, config);
        Ok(config)
    }

    async fn client_reconnected(
        &self,
        installation_id: Uuid,
        account: Option<Handle<Self::Account>>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        if let Some(account) = account {
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

    async fn new_installation_connected(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        info!("New client connected: {}", installation_id);
        Ok(RequestHandling::Respond(ChatResponse::Unauthenticated))
    }
}
