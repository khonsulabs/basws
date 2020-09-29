use crate::{
    logic::{ClientLogic, Error},
    login_state::LoginState,
};
use async_channel::Receiver;
use async_handle::Handle;
use async_rwlock::RwLock;
use basws_shared::{
    challenge,
    protocol::{
        protocol_version, InstallationConfig, ServerRequest, ServerResponse, WsBatchResponse,
        WsRequest,
    },
    timing::current_timestamp,
    Version,
};
use futures::{stream::SplitSink, stream::SplitStream, SinkExt, StreamExt};
use once_cell::sync::OnceCell;
use std::sync::Arc;
use tokio::net::TcpStream;
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message, WebSocketStream};
use url::Url;

mod data;
use data::{ClientData, NetworkState};

static REQUEST_COUNTER: OnceCell<Handle<u64>> = OnceCell::new();

pub struct Client<L>
where
    L: ClientLogic + ?Sized,
{
    data: Arc<ClientData<L>>,
}

impl<L> Clone for Client<L>
where
    L: ClientLogic,
{
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}

impl<L> std::ops::Deref for Client<L>
where
    L: ClientLogic,
{
    type Target = L;

    fn deref(&self) -> &Self::Target {
        &self.data.logic
    }
}

impl<L> Client<L>
where
    L: ClientLogic + 'static,
{
    pub fn new(logic: L) -> Self {
        let (sender, receiver) = async_channel::unbounded();
        Self {
            data: Arc::new(ClientData {
                logic: Box::new(logic),
                sender,
                receiver,
                state: RwLock::new(NetworkState {
                    login_state: LoginState::Disconnected,
                    average_roundtrip: None,
                    average_server_timestamp_delta: None,
                }),
            }),
        }
    }

    pub fn spawn(self) -> tokio::task::JoinHandle<anyhow::Result<()>> {
        tokio::spawn(self.run())
    }

    async fn set_login_state(&self, state: LoginState) -> anyhow::Result<()> {
        let client = self.clone();
        let mut network = self.data.state.write().await;
        network.login_state = state;
        self.data
            .logic
            .state_changed(&network.login_state, client)
            .await
    }

    pub async fn login_state(&self) -> LoginState {
        let network = self.data.state.read().await;
        network.login_state.clone()
    }

    async fn receiver(&self) -> Receiver<WsRequest<L::Request>> {
        self.data.receiver.clone()
    }

    pub async fn average_roundtrip(&self) -> Option<f64> {
        let network = self.data.state.read().await;
        network.average_roundtrip
    }

    pub async fn average_server_timestamp_delta(&self) -> Option<f64> {
        let network = self.data.state.read().await;
        network.average_server_timestamp_delta
    }

    async fn server_url(&self) -> Url {
        self.data.logic.server_url()
    }

    async fn protocol_version(&self) -> Version {
        self.data.logic.protocol_version()
    }

    async fn stored_installation_config(&self) -> Option<InstallationConfig> {
        self.data.logic.stored_installation_config().await
    }

    async fn store_installation_config(&self, config: InstallationConfig) -> anyhow::Result<()> {
        self.data.logic.store_installation_config(config).await
    }

    async fn response_received(
        &self,
        response: L::Response,
        original_request_id: Option<u64>,
    ) -> anyhow::Result<()> {
        let client = self.clone();
        self.data
            .logic
            .response_received(response, original_request_id, client)
            .await
    }

    async fn handle_error(&self, error: Error) -> anyhow::Result<()> {
        let client = self.clone();
        self.data.logic.handle_error(error, client).await
    }

    pub async fn run(self) -> anyhow::Result<()> {
        loop {
            let url = self.server_url().await;

            let reconnect_delay = match connect_async(url).await {
                Ok((ws, _)) => {
                    let (tx, rx) = ws.split();
                    let _ = tokio::try_join!(self.send_loop(tx), self.receive_loop(rx));
                    None
                }
                Err(err) => {
                    self.handle_error(Error::Websocket(err)).await?;
                    Some(tokio::time::Duration::from_millis(500))
                }
            };

            self.set_login_state(LoginState::Disconnected).await?;

            if let Some(delay) = reconnect_delay {
                tokio::time::delay_for(delay).await
            }
        }
    }

    async fn receive_loop(
        &self,
        mut rx: SplitStream<WebSocketStream<TcpStream>>,
    ) -> anyhow::Result<()> {
        loop {
            match rx.next().await {
                Some(Ok(Message::Binary(bytes))) => {
                    match serde_cbor::from_slice::<WsBatchResponse<L::Response>>(&bytes) {
                        Ok(response) => self.handle_batch_response(response).await?,
                        Err(error) => {
                            self.handle_error(Error::Cbor(error)).await?;
                        }
                    }
                }
                Some(Err(err)) => return Err(anyhow::Error::from(err)),
                None => {
                    anyhow::bail!("Disconnected on read");
                }
                _ => {}
            }
        }
    }

    async fn handle_batch_response(
        &self,
        batch: WsBatchResponse<L::Response>,
    ) -> anyhow::Result<()> {
        for response in batch.results {
            match response {
                ServerResponse::NewInstallation(config) => {
                    self.set_login_state(LoginState::Connected {
                        installation_id: config.id,
                    })
                    .await?;
                    self.store_installation_config(config).await?;
                }
                ServerResponse::Connected { installation_id } => {
                    self.set_login_state(LoginState::Connected { installation_id })
                        .await?;
                }
                ServerResponse::Challenge { nonce } => {
                    let config = self.stored_installation_config().await.ok_or_else(|| {
                        anyhow::anyhow!("Server issued challenge, but client has no stored config")
                    })?;

                    self.server_request(ServerRequest::ChallengeResponse(
                        challenge::compute_challenge(&config.private_key, &nonce),
                    ))
                    .await?
                }
                ServerResponse::Ping {
                    average_roundtrip,
                    average_server_timestamp_delta,
                    timestamp,
                } => {
                    let mut data = self.data.state.write().await;
                    data.average_roundtrip = average_roundtrip;
                    data.average_server_timestamp_delta = average_server_timestamp_delta;
                    self.server_request(ServerRequest::Pong {
                        original_timestamp: timestamp,
                        timestamp: current_timestamp(),
                    })
                    .await?;
                }
                ServerResponse::Error(error) => self.handle_error(Error::Server(error)).await?,
                ServerResponse::Response(response) => {
                    self.response_received(response, batch.request_id).await?
                }
            }
        }

        Ok(())
    }

    async fn server_request(&self, request: ServerRequest<L::Request>) -> anyhow::Result<()> {
        let id = {
            let mut counter = REQUEST_COUNTER.get_or_init(|| Handle::new(0)).write().await;
            *counter = counter.wrapping_add(1);
            *counter
        };
        self.data.sender.send(WsRequest { id, request }).await?;
        Ok(())
    }

    pub async fn request(&self, request: L::Request) -> anyhow::Result<()> {
        self.server_request(ServerRequest::Request(request)).await
    }

    async fn send_loop(
        &self,
        mut tx: SplitSink<WebSocketStream<TcpStream>, Message>,
    ) -> anyhow::Result<()> {
        let config = self.stored_installation_config().await;
        let installation_id = config.as_ref().map(|config| config.id);
        self.set_login_state(LoginState::Handshaking { config })
            .await?;

        self.server_request(ServerRequest::Greetings {
            protocol_version: protocol_version().to_string(),
            server_version: self.protocol_version().await.to_string(),
            installation_id,
        })
        .await?;

        let receiver = self.receiver().await;
        loop {
            let request = receiver.recv().await?;

            if let Err(err) = tx
                .send(Message::Binary(serde_cbor::to_vec(&request).unwrap()))
                .await
            {
                return Err(anyhow::Error::from(err));
            }
        }
    }
}
