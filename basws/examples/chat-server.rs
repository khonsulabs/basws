use async_trait::async_trait;
use basws::{
    server::{
        ConnectedClientHandle, ErrorHandling, Handle, Identifiable, RequestHandling,
        WebsocketServer, WebsocketServerLogic,
    },
    shared::challenge,
};
use serde_derive::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;
use warp::Filter;

mod shared;
use shared::chat::{ChatRequest, ChatResponse, ChatSender, PROTOCOL_VERSION, SERVER_PORT};

#[derive(Default)]
struct ChatServer {
    accounts: Handle<HashMap<i64, Handle<Account>>>,
    screennames: Handle<HashMap<String, i64>>,
    installation_accounts: Handle<HashMap<Uuid, i64>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Account {
    id: i64,
    username: String,
    private_key: Vec<u8>,
}

impl Identifiable for Account {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id
    }
    fn private_key(&self) -> Vec<u8> {
        self.private_key.clone()
    }
}

#[async_trait]
impl WebsocketServerLogic for ChatServer {
    type Request = ChatRequest;
    type Response = ChatResponse;
    type Account = Account;
    type AccountId = i64;

    async fn handle_request(
        &self,
        client: &ConnectedClientHandle<Self::Response, Self::Account>,
        request: Self::Request,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        match request {
            ChatRequest::Login { username } => {
                let client = client.read().await;
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
                        private_key: challenge::nonce(),
                    };
                    let account = Handle::new(account);
                    accounts.insert(id, account.clone());
                    screennames.insert(username.clone(), id);
                    account
                };

                WebsocketServer::<Self>::associate_installation_with_account(
                    client.installation_id.unwrap(),
                    account,
                )
                .await?;

                Ok(RequestHandling::Respond(ChatResponse::LoggedIn {
                    username,
                }))
            }
            ChatRequest::Chat { message } => {
                let from = {
                    let client = client.read().await;
                    if let Some(account) = &client.account {
                        let account = account.read().await;
                        ChatSender::User(account.username.clone())
                    } else if let Some(installation_id) = client.installation_id {
                        ChatSender::Anonymous(installation_id)
                    } else {
                        anyhow::bail!("Client did not finish handshake before chatting");
                    }
                };

                WebsocketServer::<Self>::broadcast(ChatResponse::ChatReceived { from, message })
                    .await;

                Ok(RequestHandling::NoResponse)
            }
        }
    }

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Handle<Self::Account>>> {
        let installation_accounts = self.installation_accounts.read().await;
        Ok(
            if let Some(account_id) = installation_accounts.get(&installation_id) {
                let accounts = self.accounts.read().await;
                accounts.get(account_id).cloned()
            } else {
                None
            },
        )
    }

    fn check_protocol_version(&self, version: &str) -> ErrorHandling {
        if PROTOCOL_VERSION == version {
            ErrorHandling::StayConnected
        } else {
            ErrorHandling::Disconnect
        }
    }

    async fn lookup_or_create_installation(&self, _installation_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }

    async fn client_reconnected(
        &self,
        installation_id: Uuid,
        account: Handle<Self::Account>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        let account = account.read().await;
        println!(
            "Previously authenticated client reconnected: {} ({})",
            account.username, installation_id
        );
        Ok(RequestHandling::Respond(ChatResponse::LoggedIn {
            username: account.username.clone(),
        }))
    }

    async fn new_installation_connected(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        println!("New client connected: {}", installation_id);
        Ok(RequestHandling::Respond(ChatResponse::Unauthenticated))
    }

    async fn handle_websocket_error(&self, err: warp::Error) -> ErrorHandling {
        println!("Error on socket: {:?}", err);
        ErrorHandling::Disconnect
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    WebsocketServer::initialize(ChatServer::default());

    let routes = warp::path("ws").and(warp::ws()).map(|ws: warp::ws::Ws| {
        // And then our closure will be called when it completes...
        ws.on_upgrade(|ws| async { WebsocketServer::<ChatServer>::incoming_connection(ws).await })
    });

    warp::serve(routes).run(([127, 0, 0, 1], SERVER_PORT)).await;
    Ok(())
}
