use basws_shared::{
    challenge, prelude::InstallationConfig, prelude::ServerError, protocol,
    timing::current_timestamp, Version,
};
use rand::{thread_rng, Rng};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
};
use thiserror::Error;
use url::Url;
use yew::prelude::*;
use yew::worker::*;
use yew::{
    format::Json,
    services::{
        storage::{Area, StorageService},
        timeout::{TimeoutService, TimeoutTask},
        websocket::{WebSocketService, WebSocketStatus, WebSocketTask},
    },
};

pub enum Message<T> {
    Initialize,
    Reset,
    Message(protocol::WsBatchResponse<T>),
    Connected,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum AgentResponse<T> {
    Connected,
    Disconnected,
    Response(T),
    StorageStatus(bool),
}

pub struct ApiAgent<T>
where
    T: ClientLogic + 'static,
{
    logic: T,
    link: AgentLink<Self>,
    web_socket_task: Option<WebSocketTask>,
    ws_request_id: u64,
    reconnect_timer: Option<TimeoutTask>,
    reconnect_sleep_ms: u32,
    callbacks: HashMap<u64, HandlerId>,
    broadcasts: HashSet<HandlerId>,
    ready_for_messages: bool,
    storage: StorageService,
    auth_state: ClientState,
    storage_enabled: bool,
}

#[derive(Debug)]
pub enum Error {
    Server(ServerError),
}

pub trait ClientLogic: Default {
    type Request: Serialize + DeserializeOwned + Sync + Send + Clone + Debug;
    type Response: Serialize + DeserializeOwned + Sync + Send + Clone + Debug;

    fn server_url(&self) -> Url;
    fn protocol_version(&self) -> Version;

    fn state_changed(&self, state: &ClientState) -> anyhow::Result<()>;

    fn response_received(
        &mut self,
        response: Self::Response,
        original_request_id: Option<u64>,
    ) -> anyhow::Result<()>;

    fn handle_error(&self, error: Error) -> anyhow::Result<()>;
}

pub type ApiBridge<T> = Box<dyn Bridge<ApiAgent<T>>>;

const DEFAULT_RECONNECT_TIMEOUT: u32 = 125;
#[derive(Serialize, Deserialize, Debug)]
pub enum AgentMessage<T> {
    Request(T),
    Initialize,
    Reset,
    RegisterBroadcastHandler,
    UnregisterBroadcastHandler,
    LogOut,
    QueryStorageStatus,
    EnableStorage,
    DisableStorage,
}

impl<T> Agent for ApiAgent<T>
where
    T: ClientLogic + 'static,
{
    type Reach = Context<Self>; // Spawn only one instance on the main thread (all components can share this agent)
    type Message = Message<T::Response>;
    type Input = AgentMessage<T::Request>;
    type Output = AgentResponse<T::Response>;

    // Create an instance with a link to the agent.
    fn create(link: AgentLink<Self>) -> Self {
        let storage = StorageService::new(Area::Local).expect("Error accessing storage service");
        let Json(login_state) = storage.restore("login_state");
        let encrypted_login_info: EncryptedLoginInformation = login_state.unwrap_or_default();
        let auth_state = encrypted_login_info.auth_state();
        let storage_enabled = auth_state.installation.is_some();
        Self {
            logic: T::default(),
            link,
            web_socket_task: None,
            ws_request_id: 0,
            reconnect_sleep_ms: DEFAULT_RECONNECT_TIMEOUT,
            reconnect_timer: None,
            callbacks: HashMap::new(),
            broadcasts: HashSet::new(),
            ready_for_messages: false,
            storage,
            auth_state,
            storage_enabled,
        }
    }

    // Handle inner messages (from callbacks)
    fn update(&mut self, msg: Self::Message) {
        match msg {
            Message::Initialize => {
                self.auth_state.connection_state = ConnectionState::Disconnected;
                self.reconnect_timer = None;
                self.initialize_websockets();
            }
            Message::Connected => {
                self.auth_state.connection_state = ConnectionState::Handshaking;
                self.reconnect_sleep_ms = DEFAULT_RECONNECT_TIMEOUT;
                self.ready_for_messages = true;
                self.ws_send(
                    protocol::ServerRequest::Greetings {
                        server_version: self.logic.protocol_version().to_string(),
                        protocol_version: protocol::protocol_version().to_string(),
                        installation_id: self.auth_state.installation.map(|config| config.id),
                    },
                    None,
                );
                self.broadcast(AgentResponse::Connected);
            }
            Message::Message(ws_response) => {
                for response in ws_response.results {
                    match response {
                        protocol::ServerResponse::Challenge { nonce } => self.ws_send(
                            protocol::ServerRequest::ChallengeResponse(
                                challenge::compute_challenge(
                                    &self.auth_state.installation.unwrap().private_key,
                                    &nonce,
                                ),
                            ),
                            None,
                        ),
                        protocol::ServerResponse::Ping { timestamp, .. } => self.ws_send(
                            protocol::ServerRequest::Pong {
                                original_timestamp: timestamp,
                                timestamp: current_timestamp(),
                            },
                            None,
                        ),
                        protocol::ServerResponse::NewInstallation(config) => {
                            self.auth_state.installation = Some(config);
                            self.save_login_state();
                        }
                        protocol::ServerResponse::Connected { .. } => {
                            self.auth_state.connection_state = ConnectionState::Connected;
                            self.logic.state_changed(&self.auth_state).unwrap();
                        }
                        protocol::ServerResponse::Response(response) => {
                            self.handle_response(response, ws_response.request_id)
                                .unwrap();
                        }
                        protocol::ServerResponse::Error(err) => {
                            self.logic.handle_error(Error::Server(err)).unwrap();
                        }
                    }
                }

                if let Some(request_id) = ws_response.request_id {
                    self.callbacks.remove(&request_id);
                }
            }
            Message::Reset => {
                self.broadcast(AgentResponse::Disconnected);

                if self.reconnect_timer.is_some() {
                    return;
                }
                self.web_socket_task = None;
                self.ready_for_messages = false;
                self.reconnect_sleep_ms = std::cmp::min(self.reconnect_sleep_ms * 2, 30_000);
                self.reconnect_timer = Some(TimeoutService::spawn(
                    std::time::Duration::from_millis(self.reconnect_sleep_ms as u64),
                    self.link.callback(|_| Message::Initialize),
                ));
            }
        }
    }

    // Handle incoming messages from components of other agents.
    fn handle_input(&mut self, msg: Self::Input, who: HandlerId) {
        match msg {
            AgentMessage::Initialize => {
                if self.web_socket_task.is_none() {
                    self.initialize_websockets();
                }
            }
            AgentMessage::Request(req) => {
                self.ws_send(protocol::ServerRequest::Request(req), Some(who));
            }
            AgentMessage::Reset => {
                self.update(Message::Reset);
            }
            AgentMessage::RegisterBroadcastHandler => {
                self.broadcasts.insert(who);
            }
            AgentMessage::UnregisterBroadcastHandler => {
                self.broadcasts.remove(&who);
            }
            AgentMessage::LogOut => {
                self.auth_state = ClientState::default();
                self.storage_enabled = false;
                self.save_login_state();
                self.update(Message::Reset);
            }
            AgentMessage::QueryStorageStatus => {
                self.link
                    .respond(who, AgentResponse::StorageStatus(self.storage_enabled));
            }
            AgentMessage::EnableStorage => {
                self.storage_enabled = true;
                self.save_login_state();
                self.link
                    .respond(who, AgentResponse::StorageStatus(self.storage_enabled));
            }
            AgentMessage::DisableStorage => {
                self.storage_enabled = false;
                self.save_login_state();
                self.link
                    .respond(who, AgentResponse::StorageStatus(self.storage_enabled));
            }
        }
    }

    fn disconnected(&mut self, id: HandlerId) {
        self.broadcasts.remove(&id);
        let callbacks_to_remove = self
            .callbacks
            .iter()
            .filter(|(_, handler)| **handler == id)
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();

        for id in callbacks_to_remove {
            self.callbacks.remove(&id);
        }
    }
}

use yew::format::{Binary, Cbor, Text};
#[derive(Debug)]
pub struct WsMessageProxy<T>(pub T);

impl<T> From<Text> for WsMessageProxy<Result<T, anyhow::Error>>
where
    T: Default,
{
    fn from(_: Text) -> Self {
        unreachable!("We shouldn't be getting non-binary messages over our websockets")
    }
}

#[derive(Debug, Error)]
enum WsMessageError {
    #[error("error decoding bincode")]
    Serialization(#[from] serde_cbor::Error),
}

impl<T> From<Binary> for WsMessageProxy<Result<T, anyhow::Error>>
where
    for<'de> T: serde::Deserialize<'de>,
{
    fn from(bytes: Binary) -> Self {
        match bytes {
            Ok(bytes) => WsMessageProxy(match serde_cbor::from_slice(bytes.as_slice()) {
                Ok(result) => Ok(result),
                Err(err) => Err(WsMessageError::Serialization(err).into()),
            }),
            Err(err) => Self(Err(err)),
        }
    }
}

impl<T> ApiAgent<T>
where
    T: ClientLogic,
{
    fn initialize_websockets(&mut self) {
        if self.reconnect_timer.is_some() {
            return;
        }
        let callback = self.link.callback(|WsMessageProxy(msg)| match msg {
            Ok(data) => Message::Message(data),
            Err(_) => Message::Reset,
        });
        let notification = self.link.callback(|status| match status {
            WebSocketStatus::Opened => Message::Connected,
            WebSocketStatus::Closed | WebSocketStatus::Error => Message::Reset,
        });
        self.web_socket_task = Some(
            WebSocketService::connect(&self.logic.server_url().to_string(), callback, notification)
                .unwrap(),
        );
    }

    fn ws_send(&mut self, request: protocol::ServerRequest<T::Request>, who: Option<HandlerId>) {
        self.ws_request_id = self.ws_request_id.wrapping_add(1);
        if self.ready_for_messages {
            if let Some(websocket) = self.web_socket_task.as_mut() {
                if let Some(who) = who {
                    self.callbacks.insert(self.ws_request_id, who);
                }
                websocket.send_binary(Cbor(&protocol::WsRequest {
                    id: self.ws_request_id,
                    request,
                }));
            }
        }
    }

    fn broadcast(&self, response: AgentResponse<T::Response>) {
        for entry in self.broadcasts.iter() {
            self.link.respond(*entry, response.clone());
        }
    }

    fn save_login_state(&mut self) {
        if self.storage_enabled {
            self.storage.store(
                "login_state",
                Json(&self.auth_state.encrypted_login_information()),
            );
        } else {
            self.storage.remove("login_state");
            self.storage.remove("return_path");
        }
    }

    fn handle_response(
        &mut self,
        response: T::Response,
        request_id: Option<u64>,
    ) -> anyhow::Result<()> {
        self.broadcast(AgentResponse::Response(response.clone()));
        if let Some(request_id) = request_id {
            if let Some(who) = self.callbacks.get(&request_id) {
                self.link
                    .respond(*who, AgentResponse::Response(response.clone()));
            };
        }

        self.logic.response_received(response, request_id)?;

        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq, Default)]
pub struct ClientState {
    installation: Option<InstallationConfig>,
    connection_state: ConnectionState,
}

#[derive(Clone, Serialize, Deserialize, Debug, Eq, PartialEq)]
pub enum ConnectionState {
    Disconnected,
    Handshaking,
    Connected,
}

impl Default for ConnectionState {
    fn default() -> Self {
        ConnectionState::Disconnected
    }
}

impl ClientState {
    fn encrypted_login_information(&self) -> EncryptedLoginInformation {
        use aead::{generic_array::GenericArray, Aead, NewAead};
        use aes_gcm::Aes256Gcm;

        let key = encryption_key();
        let key = GenericArray::from_exact_iter(key.bytes()).unwrap();
        let aead = Aes256Gcm::new(&key);

        let mut rng = thread_rng();
        let key = std::iter::repeat(())
            .map(|()| rng.gen())
            .take(12)
            .collect::<Vec<_>>();
        let nonce = GenericArray::from_slice(&key);
        let payload =
            serde_json::to_string(&self.installation).expect("Error serializing login state");
        let payload = payload.into_bytes();
        let payload: &[u8] = &payload;
        let ciphertext = aead.encrypt(nonce, payload).expect("encryption failure!");

        EncryptedLoginInformation {
            iv: base64::encode_config(nonce, base64::URL_SAFE_NO_PAD),
            encrypted: base64::encode_config(&ciphertext, base64::URL_SAFE_NO_PAD),
        }
    }
}

#[cfg(not(debug_assertions))]
fn encryption_key() -> &'static str {
    std::env!("NCOG_CLIENT_ENCRYPTION_KEY")
}

#[cfg(debug_assertions)]
fn encryption_key() -> &'static str {
    "pcnhAlQq9VNmOp325GFU8JtR8vuD1wIj"
}

#[derive(Clone, Serialize, Deserialize, Debug, Default)]
struct EncryptedLoginInformation {
    pub iv: String,
    pub encrypted: String,
}

impl EncryptedLoginInformation {
    pub fn auth_state(&self) -> ClientState {
        if !self.iv.is_empty() && !self.encrypted.is_empty() {
            if let Ok(nonce) = base64::decode_config(&self.iv, base64::URL_SAFE_NO_PAD) {
                if let Ok(ciphertext) =
                    base64::decode_config(&self.encrypted, base64::URL_SAFE_NO_PAD)
                {
                    use aead::{generic_array::GenericArray, Aead, NewAead};
                    use aes_gcm::Aes256Gcm;

                    let key = encryption_key();
                    let key =
                        GenericArray::from_exact_iter(key.bytes()).expect("Invalid encryption key");
                    let aead = Aes256Gcm::new(&key);
                    let nonce = GenericArray::from_exact_iter(nonce).expect("Invalid nonce");
                    let ciphertext: &[u8] = &ciphertext;
                    if let Ok(plaintext) = aead.decrypt(&nonce, ciphertext) {
                        if let Ok(installation) =
                            serde_json::from_slice::<Option<InstallationConfig>>(&plaintext)
                        {
                            return ClientState {
                                installation,
                                connection_state: ConnectionState::Disconnected,
                            };
                        }
                    }
                }
            }
        }
        ClientState::default()
    }
}

pub use basws_shared as shared;

pub mod prelude {
    pub use basws_shared::prelude::*;
    pub use super::{
        Message, ClientState, ClientLogic,AgentMessage, AgentResponse
    };
}