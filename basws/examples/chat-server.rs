use async_trait::async_trait;
use basws::server::{
    AccountHandle, ConnectedClientHandle, ErrorHandling, Identifiable, RequestHandling,
    WebsocketServerLogic,
};
use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

mod shared;
use shared::chat::{ChatRequest, ChatResponse};

struct ChatServer;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Account {
    id: i64,
    screenname: String,
}

impl Identifiable for Account {
    type Id = i64;
    fn id(&self) -> Self::Id {
        self.id
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
            ChatRequest::Login { username } => todo!(),
            ChatRequest::Chat { message } => todo!(),
        }
    }

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Self::Account>> {
        todo!()
    }

    fn check_protocol_version(&self, version: &str) -> ErrorHandling {
        todo!()
    }

    async fn lookup_or_create_installation(&self, installation_id: Uuid) -> anyhow::Result<()> {
        todo!()
    }

    async fn client_reconnected(
        &self,
        installation_id: Uuid,
        account: AccountHandle<Self::Account>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        todo!()
    }

    async fn new_installation_connected(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        todo!()
    }

    async fn handle_websocket_error(&self, _err: warp::Error) -> ErrorHandling {
        ErrorHandling::Disconnect
    }
}

fn main() {}
