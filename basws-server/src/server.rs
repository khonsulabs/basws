use crate::{connected_client::ConnectedClient, Identifiable, WebsocketServerLogic};
pub use async_handle::Handle;
use async_rwlock::RwLock;
use async_trait::async_trait;
use basws_shared::{
    protocol::ServerError,
    protocol::ServerRequest,
    protocol::{ServerResponse, WsBatchResponse, WsRequest},
};
use futures::{SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use std::{collections::HashMap, collections::HashSet};
use uuid::Uuid;
use warp::ws::Message;

static SERVER: OnceCell<RwLock<Box<dyn ServerPublicApi>>> = OnceCell::new();

#[async_trait]
pub(crate) trait ServerPublicApi: Send + Sync {
    fn as_any(&self) -> &'_ dyn std::any::Any;
}

pub type ConnectedClientHandle<Response, Account> = Handle<ConnectedClient<Response, Account>>;
#[allow(type_alias_bounds)] // This warning is a false positive. Without the bounds, we can't use ::Id
pub type AccountMap<TAccount: Identifiable> = Handle<HashMap<TAccount::Id, Handle<TAccount>>>;

pub struct WebsocketServer<L>
where
    L: WebsocketServerLogic,
{
    logic: L,
    clients: RwLock<ClientData<L::Response, L::Account>>,
    accounts_by_id: AccountMap<L::Account>,
}

struct ClientData<Response, TAccount>
where
    TAccount: Identifiable,
{
    clients: HashMap<Uuid, ConnectedClientHandle<Response, TAccount>>,
    installations_by_account: HashMap<TAccount::Id, HashSet<Uuid>>,
    account_by_installation: HashMap<Uuid, TAccount::Id>,
}

impl<Response, TAccount> Default for ClientData<Response, TAccount>
where
    TAccount: Identifiable,
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
    fn as_any(&self) -> &'_ dyn std::any::Any {
        self
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
        let server = server.as_any().downcast_ref::<Self>().unwrap();
        server.handle_connection(ws).await
    }

    pub async fn broadcast(response: L::Response) {
        let server = SERVER.get().expect("Server never initialized").read().await;
        let server = server.as_any().downcast_ref::<Self>().unwrap();
        let data = server.clients.read().await;

        futures::future::join_all(
            data.clients
                .keys()
                .map(|&id| Self::send_to_installation_id(id, response.clone())),
        )
        .await;
    }

    pub async fn send_to_installation_id(installation_id: Uuid, response: L::Response) {
        let server = SERVER.get().expect("Server never initialized").read().await;
        let server = server.as_any().downcast_ref::<Self>().unwrap();
        let data = server.clients.read().await;
        if let Some(client) = data.clients.get(&installation_id) {
            let client = client.read().await;
            let _ = client.send(WsBatchResponse::from_response(response)).await;
        }
    }

    pub async fn send_to_account_id(account_id: L::AccountId, response: L::Response) {
        let server = SERVER.get().expect("Server never initialized").read().await;
        let server = server.as_any().downcast_ref::<Self>().unwrap();
        let data = server.clients.read().await;
        if let Some(clients) = data.installations_by_account.get(&account_id) {
            for installation_id in clients {
                if let Some(client) = data.clients.get(&installation_id) {
                    let client = client.read().await;
                    let _ = client
                        .send(WsBatchResponse::from_response(response.clone()))
                        .await;
                }
            }
        }
    }

    pub async fn associate_installation_with_account(
        installation_id: Uuid,
        account: Handle<L::Account>,
    ) -> anyhow::Result<()> {
        let server = SERVER.get().expect("Server never initialized").read().await;
        let server = server.as_any().downcast_ref::<Self>().unwrap();

        server.associate_account(installation_id, account).await?;

        Ok(())
    }

    async fn websocket_error(&self, error: warp::Error) -> ErrorHandling {
        self.logic.handle_websocket_error(error).await
    }

    async fn disconnect(&self, client: ConnectedClientHandle<L::Response, L::Account>) {
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
            data.account_by_installation.remove(&installation_id);

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
        client_handle: &ConnectedClientHandle<L::Response, L::Account>,
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
                    self.associate_account(installation_id, profile.clone())
                        .await?;

                    self.logic
                        .client_reconnected(installation_id, profile)
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
            ServerRequest::Request(request) => {
                self.logic.handle_request(client_handle, request).await
            }
        }
    }

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

        let client = Handle::new(ConnectedClient::new(sender.clone()));
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

    async fn connect(
        &self,
        installation_id: Uuid,
        client: &ConnectedClientHandle<L::Response, L::Account>,
    ) {
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
        account: Handle<L::Account>,
    ) -> anyhow::Result<()> {
        let mut data = self.clients.write().await;
        if let Some(client) = data.clients.get_mut(&installation_id) {
            let mut client = client.write().await;
            client.account = Some(account.clone());
        }

        let account_id = {
            let account = account.read().await;
            account.id()
        };

        data.account_by_installation
            .insert(installation_id, account_id);
        let installations = data
            .installations_by_account
            .entry(account_id)
            .or_insert_with(HashSet::new);
        installations.insert(installation_id);
        Ok(())
    }

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Handle<L::Account>>> {
        match self
            .logic
            .lookup_account_from_installation_id(installation_id)
            .await?
        {
            Some(profile) => {
                let mut accounts_by_id = self.accounts_by_id.write().await;
                let id = {
                    let profile = profile.read().await;
                    profile.id()
                };
                Ok(Some(
                    accounts_by_id.entry(id).or_insert_with(|| profile).clone(),
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
    use maplit::hashmap;
    use serde_derive::{Deserialize, Serialize};

    struct TestServer {
        logged_in_installations: HashMap<Uuid, i64>,
        accounts: HashMap<i64, TestAccount>,
        protocol_version: String,
    }

    #[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
    struct TestAccount(i64);

    impl Identifiable for TestAccount {
        type Id = i64;
        fn id(&self) -> Self::Id {
            self.0
        }
    }

    #[async_trait]
    #[allow(clippy::unit_arg)]
    impl WebsocketServerLogic for TestServer {
        type Request = ();
        type Response = ();
        type Account = TestAccount;
        type AccountId = i64;

        async fn handle_request(
            &self,
            _client: &ConnectedClientHandle<Self::Response, Self::Account>,
            _request: Self::Request,
        ) -> anyhow::Result<RequestHandling<Self::Response>> {
            unimplemented!()
        }

        async fn lookup_account_from_installation_id(
            &self,
            installation_id: Uuid,
        ) -> anyhow::Result<Option<Handle<Self::Account>>> {
            Ok(self
                .logged_in_installations
                .get(&installation_id)
                .map(|account_id| self.accounts.get(account_id).cloned())
                .flatten()
                .map(Handle::new))
        }

        fn check_protocol_version(&self, version: &str) -> ErrorHandling {
            if self.protocol_version == version {
                ErrorHandling::StayConnected
            } else {
                ErrorHandling::Disconnect
            }
        }

        async fn lookup_or_create_installation(
            &self,
            _installation_id: Uuid,
        ) -> anyhow::Result<()> {
            unimplemented!()
        }

        async fn client_reconnected(
            &self,
            _installation_id: Uuid,
            _account: Handle<Self::Account>,
        ) -> anyhow::Result<RequestHandling<Self::Response>> {
            todo!()
        }

        async fn new_installation_connected(
            &self,
            _installation_id: Uuid,
        ) -> anyhow::Result<RequestHandling<Self::Response>> {
            todo!()
        }
    }

    #[async_test]
    async fn simulated_events() -> anyhow::Result<()> {
        let installation_no_account = Uuid::new_v4();
        let (sender, _) = async_channel::unbounded();
        let client_no_account = Handle::new(ConnectedClient::new_with_installation_id(
            installation_no_account,
            sender,
        ));
        let installation_has_account = Uuid::new_v4();
        let (sender, _) = async_channel::unbounded();
        let client_has_account = Handle::new(ConnectedClient::new_with_installation_id(
            installation_has_account,
            sender,
        ));

        let server = WebsocketServer::new(TestServer {
            logged_in_installations: hashmap! {
                installation_has_account => 1,
            },
            accounts: hashmap! {
                1 => TestAccount(1),
            },
            protocol_version: "1".to_string(),
        });

        server
            .connect(installation_no_account, &client_no_account)
            .await;

        {
            let data = server.clients.read().await;
            assert_eq!(1, data.clients.len());
            assert!(Handle::ptr_eq(
                &data.clients[&installation_no_account],
                &client_no_account
            ));
            assert_eq!(0, data.account_by_installation.len());
            assert_eq!(0, data.installations_by_account.len());

            let accounts = server.accounts_by_id.read().await;
            assert_eq!(0, accounts.len());
        }

        server
            .connect(installation_has_account, &client_has_account)
            .await;

        {
            let data = server.clients.read().await;
            assert_eq!(2, data.clients.len());
            assert!(Handle::ptr_eq(
                &data.clients[&installation_has_account],
                &client_has_account
            ));
            assert_eq!(0, data.account_by_installation.len());
            assert_eq!(0, data.installations_by_account.len());

            let accounts = server.accounts_by_id.read().await;
            assert_eq!(0, accounts.len());
        }

        let account = server
            .lookup_account_from_installation_id(installation_has_account)
            .await?
            .unwrap();
        server
            .associate_account(installation_has_account, account)
            .await?;

        {
            let data = server.clients.read().await;
            assert_eq!(2, data.clients.len());
            assert!(Handle::ptr_eq(
                &data.clients[&installation_has_account],
                &client_has_account
            ));
            assert_eq!(1, data.account_by_installation.len());
            assert_eq!(1, data.installations_by_account.len());

            let accounts = server.accounts_by_id.read().await;
            assert_eq!(1, accounts.len());
        }

        server.disconnect(client_no_account).await;

        {
            let data = server.clients.read().await;
            assert_eq!(1, data.clients.len());
            assert_eq!(1, data.account_by_installation.len());
            assert_eq!(1, data.installations_by_account.len());

            let accounts = server.accounts_by_id.read().await;
            assert_eq!(1, accounts.len());
        }

        server.disconnect(client_has_account).await;

        {
            let data = server.clients.read().await;
            assert_eq!(0, data.clients.len());
            assert_eq!(0, data.account_by_installation.len());
            assert_eq!(0, data.installations_by_account.len());

            let accounts = server.accounts_by_id.read().await;
            assert_eq!(0, accounts.len());
        }

        Ok(())
    }
}
