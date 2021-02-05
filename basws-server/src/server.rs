use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use async_channel::Sender;
use async_handle::Handle;
use async_rwlock::RwLock;
use async_trait::async_trait;
use futures::{stream::SplitStream, SinkExt, StreamExt};
use warp::ws::Message;

use basws_shared::{
    challenge, compression,
    protocol::{
        protocol_version_requirements, InstallationConfig, ServerError, ServerRequest,
        ServerResponse, WsBatchResponse, WsRequest,
    },
    Uuid, Version,
};

use crate::{connected_client::ConnectedClient, Identifiable, ServerLogic};

#[async_trait]
pub(crate) trait ServerPublicApi: Send + Sync {
    fn as_any(&self) -> &'_ dyn std::any::Any;
}

#[allow(type_alias_bounds)] // This warning is a false positive. Without the bounds, we can't use ::Id
pub type AccountMap<TAccount: Identifiable> = Handle<HashMap<TAccount::Id, Handle<TAccount>>>;

pub struct Server<L>
where
    L: ServerLogic + ?Sized,
{
    data: Arc<ServerData<L>>,
}

impl<L> Clone for Server<L>
where
    L: ServerLogic,
{
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}

struct ServerData<L>
where
    L: ServerLogic + ?Sized,
{
    logic: Arc<Box<L>>,
    clients: RwLock<ClientData<L>>,
    accounts_by_id: AccountMap<L::Account>,
}

struct ClientData<L>
where
    L: ServerLogic + ?Sized,
{
    clients: HashMap<Uuid, Vec<ConnectedClient<L>>>,
    installations_by_account: HashMap<L::AccountId, HashSet<Uuid>>,
    account_by_installation: HashMap<Uuid, L::AccountId>,
}

impl<L> Default for ClientData<L>
where
    L: ServerLogic,
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
    pub fn new(logic: L) -> Self {
        Self {
            data: Arc::new(ServerData {
                logic: Arc::new(Box::new(logic)),
                clients: Default::default(),
                accounts_by_id: Default::default(),
            }),
        }
    }

    pub fn logic(&self) -> &'_ L {
        &self.data.logic
    }

    pub async fn incoming_connection_for_client(&self, ws: warp::ws::WebSocket, client: L::Client) {
        let (mut tx, rx) = ws.split();

        let (sender, transmission_receiver) =
            async_channel::unbounded::<WsBatchResponse<L::Response>>();

        tokio::spawn(async move {
            while let Ok(response) = transmission_receiver.recv().await {
                let bytes = compression::compress(&response);
                tx.send(Message::binary(bytes)).await.unwrap_or_default()
            }
        });

        let client = ConnectedClient::new(client, sender.clone());
        let _ = tokio::try_join!(
            client.ping_loop(self.data.logic.ping_period()),
            self.receive_loop(client.clone(), rx, sender)
        );

        let _ = self.disconnect(client).await;
    }

    async fn receive_loop(
        &self,
        client: ConnectedClient<L>,
        mut rx: SplitStream<warp::ws::WebSocket>,
        sender: Sender<WsBatchResponse<L::Response>>,
    ) -> anyhow::Result<()> {
        while let Some(result) = rx.next().await {
            match result {
                Ok(message) => {
                    match compression::decompress::<WsRequest<L::Request>>(message.as_bytes()) {
                        Ok(ws_request) => {
                            let request_id = ws_request.id;
                            match self.handle_request(&client, ws_request).await {
                                Ok(responses) => {
                                    if let Some(batch) = responses.into_batch(Some(request_id)) {
                                        if let Err(err) = sender.send(batch).await {
                                            error!(
                                                "Error sending response, disconnecting. {:?}",
                                                err
                                            );
                                            break;
                                        }
                                    }
                                }
                                Err(err) => {
                                    error!("Error handling message. Disconnecting. {:?}", err);
                                    break;
                                }
                            }
                        }
                        Err(error) => {
                            error!("Error decoding message {:?}", error);
                            break;
                        }
                    }
                }
                Err(err) => {
                    if let ErrorHandling::Disconnect = self.websocket_error(err).await {
                        println!("Disconnecting");
                        break;
                    }
                }
            }
        }

        // For try_join to exit early, we need to return an Err, even though everything is "fine"
        anyhow::bail!("Disconnected")
    }

    pub async fn broadcast(&self, response: L::Response) {
        let data = self.data.clients.read().await;

        futures::future::join_all(
            data.clients
                .keys()
                .map(|&id| self.send_to_installation_id(id, response.clone())),
        )
        .await;
    }

    pub async fn send_to_installation_id(&self, installation_id: Uuid, response: L::Response) {
        let data = self.data.clients.read().await;
        if let Some(clients) = data.clients.get(&installation_id) {
            let _ = futures::future::join_all(
                clients
                    .iter()
                    .map(|client| client.send(WsBatchResponse::from_response(response.clone()))),
            )
            .await;
        }
    }

    pub async fn send_to_account_id(&self, account_id: L::AccountId, response: L::Response) {
        let data = self.data.clients.read().await;
        if let Some(clients) = data.installations_by_account.get(&account_id) {
            for installation_id in clients {
                if let Some(client) = data.clients.get(&installation_id) {
                    let _ = futures::future::join_all(client.iter().map(|client| {
                        client.send(WsBatchResponse::from_response(response.clone()))
                    }))
                    .await;
                }
            }
        }
    }

    pub async fn associate_installation_with_account(
        &self,
        installation_id: Uuid,
        account: Handle<L::Account>,
    ) -> anyhow::Result<()> {
        let mut data = self.data.clients.write().await;

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

        if let Some(clients) = data.clients.get_mut(&installation_id) {
            for client in clients.iter_mut() {
                client.set_account(account.clone()).await;
                self.data.logic.account_associated(client).await?
            }
        }

        Ok(())
    }

    async fn websocket_error(&self, error: warp::Error) -> ErrorHandling {
        self.data.logic.handle_websocket_error(error).await
    }

    async fn disconnect(&self, client: ConnectedClient<L>) -> anyhow::Result<()> {
        if let Some(installation) = client.installation().await {
            self.data.logic.client_disconnected(&client).await?;
            let mut data = self.data.clients.write().await;

            data.clients.remove(&installation.id);
            let account_id = match data.account_by_installation.get(&installation.id) {
                Some(account_id) => *account_id,
                None => return Ok(()),
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

                let mut accounts_by_id = self.data.accounts_by_id.write().await;
                accounts_by_id.remove(&account_id);
            }
        }
        Ok(())
    }

    async fn handle_request(
        &self,
        client_handle: &ConnectedClient<L>,
        ws_request: WsRequest<L::Request>,
    ) -> anyhow::Result<ServerRequestHandling<L::Response>> {
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
                    .data
                    .logic
                    .lookup_or_create_installation(client_handle, installation_id)
                    .await?;

                if let Some(installation_id) = installation_id {
                    if installation_id == config.id {
                        self.connect(config, client_handle).await;

                        let nonce = {
                            let nonce = challenge::nonce();
                            client_handle.set_nonce(nonce).await;
                            nonce
                        };
                        return Ok(ServerRequestHandling::Respond(ServerResponse::Challenge {
                            nonce,
                        }));
                    }
                }

                self.connect(config, client_handle).await;

                let new_installation_response = self
                    .data
                    .logic
                    .new_client_connected(client_handle)
                    .await?
                    .into_server_handling();

                let response =
                    ServerRequestHandling::Respond(ServerResponse::NewInstallation(config))
                        + new_installation_response;

                Ok(response)
            }
            ServerRequest::ChallengeResponse(response) => {
                let (installation, nonce) = {
                    let installation = client_handle.installation().await.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Challenge responded on socket that didn't have installation_id setup"
                        )
                    })?;
                    let nonce = client_handle.nonce().await.ok_or_else(|| {
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
                        self.associate_installation_with_account(installation.id, profile.clone())
                            .await?;
                    }

                    self.data.logic.client_reconnected(client_handle).await?
                } else {
                    // Failed the challenge, treat them as a new client.
                    let config = self
                        .data
                        .logic
                        .lookup_or_create_installation(client_handle, None)
                        .await?;
                    let new_installation_response = self
                        .data
                        .logic
                        .new_client_connected(client_handle)
                        .await?
                        .into_server_handling();

                    return Ok(
                        ServerRequestHandling::Respond(ServerResponse::NewInstallation(config))
                            + new_installation_response,
                    );
                };
                Ok(ServerRequestHandling::Respond(ServerResponse::Connected {
                    installation_id: installation.id,
                }) + logic_response.into_server_handling())
            }
            ServerRequest::Pong {
                original_timestamp,
                timestamp,
            } => {
                client_handle
                    .update_network_timing(original_timestamp, timestamp)
                    .await;
                self.logic()
                    .client_timings_updated(client_handle)
                    .await
                    .map(|r| r.into_server_handling())
            }
            ServerRequest::Request(request) => self
                .data
                .logic
                .handle_request(client_handle, request, self)
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
                .data
                .logic
                .protocol_version_requirements()
                .matches(&server_version)
        {
            Ok(())
        } else {
            Err(anyhow::anyhow!("Incompatible versions"))
        }
    }

    async fn connect(&self, installation: InstallationConfig, client: &ConnectedClient<L>) {
        let mut data = self.data.clients.write().await;
        data.clients
            .entry(installation.id)
            .and_modify(|clients| clients.push(client.clone()))
            .or_insert_with(|| vec![client.clone()]);
        {
            client.set_installation(installation).await;
        }
    }

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Handle<L::Account>>> {
        match self
            .data
            .logic
            .lookup_account_from_installation_id(installation_id)
            .await?
        {
            Some(profile) => {
                let mut accounts_by_id = self.data.accounts_by_id.write().await;
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

    pub async fn connected_clients(&self) -> Vec<ConnectedClient<L>> {
        let data = self.data.clients.read().await;
        data.clients.values().cloned().flatten().collect()
    }
}

impl<L> Server<L>
where
    L: ServerLogic + 'static,
    L::Client: Default,
{
    pub async fn incoming_connection(&self, ws: warp::ws::WebSocket) {
        self.incoming_connection_for_client(ws, Default::default())
            .await
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
    use async_trait::async_trait;
    use maplit::hashmap;
    use serde_derive::{Deserialize, Serialize};

    use basws_shared::VersionReq;

    use crate::logic::ServerLogic;

    use super::*;

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
        type Client = ();
        type Account = TestAccount;
        type AccountId = i64;

        async fn handle_request(
            &self,
            _client: &ConnectedClient<Self>,
            _request: Self::Request,
            _server: &Server<Self>,
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
            _client: &ConnectedClient<Self>,
            _installation_id: Option<Uuid>,
        ) -> anyhow::Result<InstallationConfig> {
            unimplemented!()
        }

        async fn client_reconnected(
            &self,
            _client: &ConnectedClient<Self>,
        ) -> anyhow::Result<RequestHandling<Self::Response>> {
            unimplemented!()
        }

        async fn new_client_connected(
            &self,
            _client: &ConnectedClient<Self>,
        ) -> anyhow::Result<RequestHandling<Self::Response>> {
            unimplemented!()
        }

        async fn client_disconnected(&self, _client: &ConnectedClient<Self>) -> anyhow::Result<()> {
            Ok(())
        }
    }

    #[async_test]
    async fn simulated_events() -> anyhow::Result<()> {
        let installation_no_account = InstallationConfig::default();
        let (sender, _) = async_channel::unbounded();
        let client_no_account =
            ConnectedClient::new_with_installation(Some(installation_no_account), (), sender);
        let installation_has_account = InstallationConfig::default();
        let (sender, _) = async_channel::unbounded();
        let client_has_account =
            ConnectedClient::new_with_installation(Some(installation_has_account), (), sender);

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
            let data = server.data.clients.read().await;
            assert_eq!(1, data.clients.len());
            assert_eq!(0, data.account_by_installation.len());
            assert_eq!(0, data.installations_by_account.len());

            let accounts = server.data.accounts_by_id.read().await;
            assert_eq!(0, accounts.len());
        }

        server
            .connect(installation_has_account, &client_has_account)
            .await;

        {
            let data = server.data.clients.read().await;
            assert_eq!(2, data.clients.len());
            assert_eq!(0, data.account_by_installation.len());
            assert_eq!(0, data.installations_by_account.len());

            let accounts = server.data.accounts_by_id.read().await;
            assert_eq!(0, accounts.len());
        }

        let account = server
            .lookup_account_from_installation_id(installation_has_account.id)
            .await?
            .unwrap();
        server
            .associate_installation_with_account(installation_has_account.id, account)
            .await?;

        {
            let data = server.data.clients.read().await;
            assert_eq!(2, data.clients.len());
            assert_eq!(1, data.account_by_installation.len());
            assert_eq!(1, data.installations_by_account.len());

            let accounts = server.data.accounts_by_id.read().await;
            assert_eq!(1, accounts.len());
        }

        server.disconnect(client_no_account).await?;

        {
            let data = server.data.clients.read().await;
            assert_eq!(1, data.clients.len());
            assert_eq!(1, data.account_by_installation.len());
            assert_eq!(1, data.installations_by_account.len());

            let accounts = server.data.accounts_by_id.read().await;
            assert_eq!(1, accounts.len());
        }

        server.disconnect(client_has_account).await?;

        {
            let data = server.data.clients.read().await;
            assert_eq!(0, data.clients.len());
            assert_eq!(0, data.account_by_installation.len());
            assert_eq!(0, data.installations_by_account.len());

            let accounts = server.data.accounts_by_id.read().await;
            assert_eq!(0, accounts.len());
        }

        Ok(())
    }
}

impl<L> std::ops::Deref for Server<L>
where
    L: ServerLogic + 'static,
{
    type Target = L;

    fn deref(&self) -> &'_ Self::Target {
        self.logic()
    }
}
