use crate::{logic::ClientLogic, login_state::LoginState};
use async_channel::{Receiver, Sender};
use async_rwlock::RwLock;
use basws_shared::protocol::WsRequest;

pub(crate) struct ClientData<L>
where
    L: ClientLogic + ?Sized,
{
    pub(crate) logic: Box<L>,
    pub(crate) sender: Sender<WsRequest<L::Request>>,
    pub(crate) receiver: Receiver<WsRequest<L::Request>>,
    pub(crate) state: RwLock<NetworkState>,
}

pub(crate) struct NetworkState {
    pub(crate) login_state: LoginState,
    pub(crate) average_roundtrip: Option<f64>,
    pub(crate) average_server_timestamp_delta: Option<f64>,
}
