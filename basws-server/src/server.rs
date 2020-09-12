use crate::{connected_client::ConnectedClient, WebsocketServerLogic};
use async_rwlock::RwLock;
use async_trait::async_trait;
use basws_shared::{
    protocol::ServerError,
    protocol::ServerRequest,
    protocol::{ServerResponse, WsBatchResponse, WsRequest},
    timing::NetworkTiming,
};
use futures::{SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use std::{collections::HashMap, collections::HashSet, sync::Arc};
use uuid::Uuid;
use warp::ws::Message;

static SERVER: OnceCell<RwLock<Box<dyn ServerPublicApi>>> = OnceCell::new();

#[async_trait]
pub(crate) trait ServerPublicApi: Send + Sync {
    async fn handle_connection(&self, ws: warp::ws::WebSocket);
}

pub type ConnectedClientHandle<L> = Arc<RwLock<ConnectedClient<L>>>;
pub type AccountMap<AccountId, Account> =
    Arc<RwLock<HashMap<AccountId, AccountHandle<AccountId, Account>>>>;
pub type AccountHandle<AccountId, Account> = Arc<RwLock<AccountProfile<AccountId, Account>>>;

pub struct WebsocketServer<L>
where
    L: WebsocketServerLogic,
{
    logic: L,
    clients: RwLock<ClientData<L>>,
    accounts_by_id: AccountMap<L::AccountId, L::Account>,
}

struct ClientData<L>
where
    L: WebsocketServerLogic,
{
    clients: HashMap<Uuid, ConnectedClientHandle<L>>,
    installations_by_account: HashMap<L::AccountId, HashSet<Uuid>>,
    account_by_installation: HashMap<Uuid, L::AccountId>,
}

impl<L> Default for ClientData<L>
where
    L: WebsocketServerLogic,
{
    fn default() -> Self {
        Self {
            clients: Default::default(),
            installations_by_account: Default::default(),
            account_by_installation: Default::default(),
        }
    }
}

#[async_trait]
impl<L> ServerPublicApi for WebsocketServer<L>
where
    L: WebsocketServerLogic + 'static,
{
    async fn handle_connection(&self, ws: warp::ws::WebSocket) {
        let (mut tx, mut rx) = ws.split();

        let (sender, transmission_receiver) =
            async_channel::unbounded::<WsBatchResponse<L::Response>>();

        tokio::spawn(async move {
            while let Ok(response) = transmission_receiver.recv().await {
                tx.send(Message::binary(serde_cbor::to_vec(&response).unwrap()))
                    .await
                    .unwrap_or_default()
            }
        });

        let client = Arc::new(RwLock::new(ConnectedClient {
            account: None,
            installation_id: None,
            network_timing: NetworkTiming::default(),
            sender: sender.clone(),
        }));
        while let Some(result) = rx.next().await {
            match result {
                Ok(message) => {
                    match serde_cbor::from_slice::<WsRequest<L::Request>>(message.as_bytes()) {
                        Ok(ws_request) => {
                            let request_id = ws_request.id;
                            match self.handle_request(&client, ws_request).await {
                                Ok(responses) => {
                                    if let Some(batch) = responses.into_batch(request_id) {
                                        if sender.send(batch).await.is_err() {
                                            break;
                                        }
                                    }
                                }
                                Err(err) => {
                                    println!("Error handling message. Disconnecting. {:?}", err);
                                    break;
                                }
                            }
                        }
                        Err(cbor_error) => {
                            println!("Error decoding cbor {:?}", cbor_error);
                            break;
                        }
                    }
                }
                Err(err) => {
                    if let ErrorHandling::Disconnect = self.websocket_error(err).await {
                        break;
                    }
                }
            }
        }

        self.disconnect(client).await;
    }
}

impl<L> WebsocketServer<L>
where
    L: WebsocketServerLogic + 'static,
{
    fn new(logic: L) -> Self {
        Self {
            logic,
            clients: Default::default(),
            accounts_by_id: Default::default(),
        }
    }

    pub fn initialize(logic: L) {
        let _ = SERVER.set(RwLock::new(Box::new(Self::new(logic))));
    }

    pub async fn incoming_connection(ws: warp::ws::WebSocket) {
        let server = SERVER.get().expect("Server never initialized").read().await;
        server.handle_connection(ws).await
    }

    async fn websocket_error(&self, error: warp::Error) -> ErrorHandling {
        self.logic.handle_websocket_error(error).await
    }

    async fn disconnect(&self, client: ConnectedClientHandle<L>) {
        if let Some(installation_id) = {
            let client = client.read().await;
            client.installation_id
        } {
            let mut data = self.clients.write().await;

            data.clients.remove(&installation_id);
            let account_id = match data.account_by_installation.get(&installation_id) {
                Some(account_id) => *account_id,
                None => return,
            };

            let remove_account =
                if let Some(installations) = data.installations_by_account.get_mut(&account_id) {
                    installations.remove(&installation_id);
                    installations.is_empty()
                } else {
                    false
                };

            if remove_account {
                data.installations_by_account.remove(&account_id);

                let mut accounts_by_id = self.accounts_by_id.write().await;
                accounts_by_id.remove(&account_id);
            }
        }
    }

    async fn handle_request(
        &self,
        client_handle: &ConnectedClientHandle<L>,
        ws_request: WsRequest<L::Request>,
    ) -> Result<RequestHandling<L::Response>, anyhow::Error> {
        match ws_request.request {
            ServerRequest::Authenticate {
                version,
                installation_id,
            } => {
                if let ErrorHandling::Disconnect = self.logic.check_protocol_version(&version) {
                    return Ok(RequestHandling::Error(ServerError::IncompatibleVersion));
                }

                let installation_id = match installation_id {
                    Some(installation_id) => installation_id,
                    None => Uuid::new_v4(),
                };

                self.logic
                    .lookup_or_create_installation(installation_id)
                    .await?;

                self.connect(installation_id, client_handle).await;

                if let Some(profile) = self
                    .lookup_account_from_installation_id(installation_id)
                    .await?
                {
                    let account = self.associate_account(installation_id, profile).await?;

                    self.logic
                        .client_reconnected(installation_id, account)
                        .await
                } else {
                    self.logic.new_installation_connected(installation_id).await
                }
            }
            ServerRequest::Pong {
                original_timestamp,
                timestamp,
            } => {
                let mut client = client_handle.write().await;
                client.network_timing.update(original_timestamp, timestamp);
                Ok(RequestHandling::NoResponse)
            }
            ServerRequest::Request(request) => self.logic.handle_request(request).await,
        }
    }

    async fn connect(&self, installation_id: Uuid, client: &Arc<RwLock<ConnectedClient<L>>>) {
        let mut data = self.clients.write().await;
        data.clients.insert(installation_id, client.clone());
        {
            let mut client = client.write().await;
            client.installation_id = Some(installation_id);
        }
    }

    async fn associate_account(
        &self,
        installation_id: Uuid,
        account: AccountHandle<L::AccountId, L::Account>,
    ) -> anyhow::Result<AccountHandle<L::AccountId, L::Account>> {
        // let account = self
        //     .lookup_account_from_installation_id(installation_id)
        //     .await?;
        let mut data = self.clients.write().await;
        if let Some(client) = data.clients.get_mut(&installation_id) {
            let mut client = client.write().await;
            client.account = Some(account.clone());
        }

        let account_id = {
            let account = account.read().await;
            account.id
        };

        data.account_by_installation
            .insert(installation_id, account_id);
        let installations = data
            .installations_by_account
            .entry(account_id)
            .or_insert_with(HashSet::new);
        installations.insert(installation_id);
        Ok(account)
    }

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Arc<RwLock<AccountProfile<L::AccountId, L::Account>>>>> {
        match self
            .logic
            .lookup_account_from_installation_id(installation_id)
            .await?
        {
            Some(profile) => {
                let mut accounts_by_id = self.accounts_by_id.write().await;
                Ok(Some(
                    accounts_by_id
                        .entry(profile.id)
                        .or_insert_with(|| Arc::new(RwLock::new(profile)))
                        .clone(),
                ))
            }
            None => Ok(None),
        }
    }
}

pub enum ErrorHandling {
    Disconnect,
    StayConnected,
}

pub enum RequestHandling<R> {
    NoResponse,
    Error(ServerError),
    Respond(R),
    Batch(Vec<R>),
}

pub struct AccountProfile<AccountId, Account> {
    pub id: AccountId,
    pub account: Account,
}

impl<T> RequestHandling<T> {
    fn into_batch(self, request_id: i64) -> Option<WsBatchResponse<T>> {
        match self {
            RequestHandling::NoResponse => None,
            RequestHandling::Error(error) => Some(WsBatchResponse {
                request_id,
                results: vec![ServerResponse::Error(error)],
            }),
            RequestHandling::Respond(response) => Some(WsBatchResponse {
                request_id,
                results: vec![ServerResponse::Response(response)],
            }),
            RequestHandling::Batch(results) => {
                let results = results.into_iter().map(ServerResponse::Response).collect();
                Some(WsBatchResponse {
                    request_id,
                    results,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::WebsocketServerLogic;
    use async_trait::async_trait;

    struct TestServer;

    #[async_trait]
    #[allow(clippy::unit_arg)]
    impl WebsocketServerLogic for TestServer {
        type Request = ();
        type Response = ();
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
    }

    #[async_test]
    async fn simulated_events() -> anyhow::Result<()> {
        let server = WebsocketServer::new(TestServer);
        let installation_1 = Uuid::new_v4();

        Ok(())
    }
}
