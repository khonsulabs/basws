use async_trait::async_trait;
use basws::server::{
    AccountHandle, AccountProfile, ErrorHandling, RequestHandling, WebsocketServerLogic,
};
use uuid::Uuid;
mod shared;

struct ChatServer;

#[async_trait]
impl WebsocketServerLogic for ChatServer {
    type Request = shared::chat::ChatRequest;
    type Response = shared::chat::ChatResponse;
    type Account = ();
    type AccountId = i64;

    async fn handle_request(
        &self,
        request: Self::Request,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        todo!()
    }

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<AccountProfile<Self::AccountId, Self::Account>>> {
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
        account: AccountHandle<Self::AccountId, Self::Account>,
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
