use crate::{connected_client::ConnectedClient, Identifiable, ServerLogic};
use async_handle::Handle;
use async_rwlock::RwLock;
use async_trait::async_trait;
use basws_shared::{
    challenge,
    protocol::{
        protocol_version_requirements, InstallationConfig, ServerError, ServerRequest,
        ServerResponse, WsBatchResponse, WsRequest,
    },
    Uuid, Version,
};
use futures::{SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use std::{collections::HashMap, collections::HashSet};
use warp::ws::Message;

static SERVER: OnceCell<Box<dyn ServerPublicApi>> = OnceCell::new();

#[async_trait]
pub(crate) trait ServerPublicApi: Send + Sync {
    fn as_any(&self) -> &'_ dyn std::any::Any;
}

pub type ConnectedClientHandle<Response, Account> = Handle<ConnectedClient<Response, Account>>;
#[allow(type_alias_bounds)] // This warning is a false positive. Without the bounds, we can't use ::Id
pub type AccountMap<TAccount: Identifiable> = Handle<HashMap<TAccount::Id, Handle<TAccount>>>;

pub struct Server<L>
where
    L: ServerLogic,
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
impl<L> ServerPublicApi for Server<L>
where
    L: ServerLogic + 'static,
{
    fn as_any(&self) -> &'_ dyn std::any::Any {
        self
    }
}

impl<L> Server<L>
where
    L: ServerLogic + 'static,
{
    fn new(logic: L) -> Self {
        Self {
            logic,
            clients: Default::default(),
            accounts_by_id: Default::default(),
        }
    }

    pub fn initialize(logic: L) {
        let _ = SERVER.set(Box::new(Self::new(logic)));
    }

    pub async fn incoming_connection(ws: warp::ws::WebSocket) {
        let server = SERVER.get().expect("Server never initialized");
        let server = server.as_any().downcast_ref::<Self>().unwrap();
        server.handle_connection(ws).await
    }

    pub async fn broadcast(response: L::Response) {
        let server = SERVER.get().expect("Server never initialized");
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
        let server = SERVER.get().expect("Server never initialized");
        let server = server.as_any().downcast_ref::<Self>().unwrap();
        let data = server.clients.read().await;
        if let Some(client) = data.clients.get(&installation_id) {
            let client = client.read().await;
            let _ = client.send(WsBatchResponse::from_response(response)).await;
        }
    }

    pub async fn send_to_account_id(account_id: L::AccountId, response: L::Response) {
        let server = SERVER.get().expect("Server never initialized");
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
        let server = SERVER.get().expect("Server never initialized");
        let server = server.as_any().downcast_ref::<Self>().unwrap();

        server.associate_account(installation_id, account).await;

        Ok(())
    }

    async fn websocket_error(&self, error: warp::Error) -> ErrorHandling {
        self.logic.handle_websocket_error(error).await
    }

    async fn disconnect(&self, client: ConnectedClientHandle<L::Response, L::Account>) {
        if let Some(installation) = {
            let client = client.read().await;
            client.installation
        } {
            let mut data = self.clients.write().await;

            data.clients.remove(&installation.id);
            let account_id = match data.account_by_installation.get(&installation.id) {
                Some(account_id) => *account_id,
                None => return,
            };
            data.account_by_installation.remove(&installation.id);

            let remove_account =
                if let Some(installations) = data.installations_by_account.get_mut(&account_id) {
                    installations.remove(&installation.id);
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
    ) -> Result<ServerRequestHandling<L::Response>, anyhow::Error> {
        match ws_request.request {
            ServerRequest::Greetings {
                protocol_version,
                server_version,
                installation_id,
            } => {
                if self
                    .check_protocol_versions(&protocol_version, &server_version)
                    .await
                    .is_err()
                {
                    return Ok(ServerRequestHandling::Error(
                        ServerError::IncompatibleVersion,
                    ));
                }

                let config = self
                    .logic
                    .lookup_or_create_installation(installation_id)
                    .await?;

                if let Some(installation_id) = installation_id {
                    if installation_id == config.id {
                        self.connect(config, client_handle).await;

                        let nonce = {
                            let mut client = client_handle.write().await;
                            let nonce = challenge::nonce();
                            client.nonce = Some(nonce);
                            nonce
                        };
                        return Ok(ServerRequestHandling::Respond(ServerResponse::Challenge {
                            nonce,
                        }));
                    }
                }

                self.connect(config, client_handle).await;

                let new_installation_response = self
                    .logic
                    .new_installation_connected(config.id)
                    .await?
                    .into_server_handling();

                let response =
                    ServerRequestHandling::Respond(ServerResponse::NewInstallation(config))
                        + new_installation_response;

                Ok(response)
            }
            ServerRequest::ChallengeResponse(response) => {
                let (installation, nonce) = {
                    let client = client_handle.read().await;
                    let installation = client.installation.clone().ok_or_else(|| {
                        anyhow::anyhow!(
                            "Challenge responded on socket that didn't have installation_id setup"
                        )
                    })?;
                    let nonce = client.nonce.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Challenge responded on socket that didn't have nonce setup"
                        )
                    })?;
                    (installation, nonce)
                };

                let logic_response = if challenge::compute_challenge(
                    &installation.private_key,
                    &nonce,
                ) == response
                {
                    let profile = self
                        .lookup_account_from_installation_id(installation.id)
                        .await?;

                    if let Some(profile) = &profile {
                        self.associate_account(installation.id, profile.clone())
                            .await;
                    }

                    self.logic
                        .client_reconnected(installation.id, profile)
                        .await?
                } else {
                    self.logic
                        .new_installation_connected(installation.id)
                        .await?
                };
                Ok(ServerRequestHandling::Respond(ServerResponse::Connected {
                    installation_id: installation.id,
                }) + logic_response.into_server_handling())
            }
            ServerRequest::Pong {
                original_timestamp,
                timestamp,
            } => {
                let mut client = client_handle.write().await;
                client.network_timing.update(original_timestamp, timestamp);
                Ok(ServerRequestHandling::NoResponse)
            }
            ServerRequest::Request(request) => self
                .logic
                .handle_request(client_handle, request)
                .await
                .map(|result| result.into_server_handling()),
        }
    }

    async fn check_protocol_versions(
        &self,
        protocol_version: &str,
        server_version: &str,
    ) -> anyhow::Result<()> {
        let protocol_version = Version::parse(protocol_version)?;
        let server_version = Version::parse(server_version)?;
        if protocol_version_requirements().matches(&protocol_version)
            && self
                .logic
                .protocol_version_requirements()
                .matches(&server_version)
        {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Incompatible versions"))
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
                                    if let Some(batch) = responses.into_batch(Some(request_id)) {
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
        installation: InstallationConfig,
        client: &ConnectedClientHandle<L::Response, L::Account>,
    ) {
        let mut data = self.clients.write().await;
        data.clients.insert(installation.id, client.clone());
        {
            let mut client = client.write().await;
            client.installation = Some(installation);
        }
    }

    async fn associate_account(&self, installation_id: Uuid, account: Handle<L::Account>) {
        let mut data = self.clients.write().await;
        println!("Locked Clients");
        if let Some(client) = data.clients.get_mut(&installation_id) {
            let mut client = client.write().await;
            println!("Locked Client");
            client.account = Some(account.clone());
        }

        let account_id = {
            let account = account.read().await;
            account.id()
        };
        println!("Read account_id");

        data.account_by_installation
            .insert(installation_id, account_id);
        let installations = data
            .installations_by_account
            .entry(account_id)
            .or_insert_with(HashSet::new);
        installations.insert(installation_id);
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

pub enum ServerRequestHandling<R> {
    NoResponse,
    Error(ServerError),
    Respond(ServerResponse<R>),
    Batch(Vec<ServerResponse<R>>),
}

impl<T> RequestHandling<T> {
    fn into_server_handling(self) -> ServerRequestHandling<T> {
        match self {
            RequestHandling::NoResponse => ServerRequestHandling::NoResponse,
            RequestHandling::Error(error) => ServerRequestHandling::Error(error),
            RequestHandling::Respond(response) => {
                ServerRequestHandling::Respond(ServerResponse::Response(response))
            }
            RequestHandling::Batch(results) => {
                let results = results.into_iter().map(ServerResponse::Response).collect();
                ServerRequestHandling::Batch(results)
            }
        }
    }
}

impl<T> ServerRequestHandling<T>
where
    T: Clone,
{
    fn into_batch(self, request_id: Option<u64>) -> Option<WsBatchResponse<T>> {
        let results = self.responses();
        if results.is_empty() {
            None
        } else {
            Some(WsBatchResponse {
                request_id,
                results,
            })
        }
    }

    fn responses(&self) -> Vec<ServerResponse<T>> {
        match self {
            ServerRequestHandling::NoResponse => Vec::new(),
            ServerRequestHandling::Error(error) => vec![ServerResponse::Error(error.clone())],
            ServerRequestHandling::Respond(response) => vec![response.clone()],
            ServerRequestHandling::Batch(results) => results.clone(),
        }
    }
}

impl<T> std::ops::Add for ServerRequestHandling<T>
where
    T: Clone,
{
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        let mut responses = self.responses();
        responses.extend(rhs.responses());
        ServerRequestHandling::Batch(responses)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logic::ServerLogic;
    use async_trait::async_trait;
    use basws_shared::VersionReq;
    use maplit::hashmap;
    use serde_derive::{Deserialize, Serialize};

    struct TestServer {
        logged_in_installations: HashMap<Uuid, Option<i64>>,
        accounts: HashMap<i64, TestAccount>,
    }

    #[derive(Serialize, Deserialize, Debug, Clone, Eq, PartialEq)]
    struct TestAccount {
        id: i64,
    }

    impl Identifiable for TestAccount {
        type Id = i64;
        fn id(&self) -> Self::Id {
            self.id
        }
    }

    #[async_trait]
    #[allow(clippy::unit_arg)]
    impl ServerLogic for TestServer {
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
                .map(|account_id| account_id.map(|id| self.accounts.get(&id).cloned()))
                .flatten()
                .flatten()
                .map(Handle::new))
        }

        fn protocol_version_requirements(&self) -> VersionReq {
            VersionReq::parse(">=0.1").unwrap()
        }

        async fn lookup_or_create_installation(
            &self,
            _installation_id: Option<Uuid>,
        ) -> anyhow::Result<InstallationConfig> {
            unimplemented!()
        }

        async fn client_reconnected(
            &self,
            _installation_id: Uuid,
            _account: Option<Handle<Self::Account>>,
        ) -> anyhow::Result<RequestHandling<Self::Response>> {
            unimplemented!()
        }

        async fn new_installation_connected(
            &self,
            _installation_id: Uuid,
        ) -> anyhow::Result<RequestHandling<Self::Response>> {
            unimplemented!()
        }
    }

    #[async_test]
    async fn simulated_events() -> anyhow::Result<()> {
        let installation_no_account = InstallationConfig::default();
        let (sender, _) = async_channel::unbounded();
        let client_no_account = Handle::new(ConnectedClient::new_with_installation(
            installation_no_account,
            sender,
        ));
        let installation_has_account = InstallationConfig::default();
        let (sender, _) = async_channel::unbounded();
        let client_has_account = Handle::new(ConnectedClient::new_with_installation(
            installation_has_account,
            sender,
        ));

        let server = Server::new(TestServer {
            logged_in_installations: hashmap! {
                installation_has_account.id => Some(1),
            },
            accounts: hashmap! {
                1 => TestAccount{id: 1},
            },
        });

        server
            .connect(installation_no_account, &client_no_account)
            .await;

        {
            let data = server.clients.read().await;
            assert_eq!(1, data.clients.len());
            assert!(Handle::ptr_eq(
                &data.clients[&installation_no_account.id],
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
                &data.clients[&installation_has_account.id],
                &client_has_account
            ));
            assert_eq!(0, data.account_by_installation.len());
            assert_eq!(0, data.installations_by_account.len());

            let accounts = server.accounts_by_id.read().await;
            assert_eq!(0, accounts.len());
        }

        let account = server
            .lookup_account_from_installation_id(installation_has_account.id)
            .await?
            .unwrap();
        server
            .associate_account(installation_has_account.id, account)
            .await;

        {
            let data = server.clients.read().await;
            assert_eq!(2, data.clients.len());
            assert!(Handle::ptr_eq(
                &data.clients[&installation_has_account.id],
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
