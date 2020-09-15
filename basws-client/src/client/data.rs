use crate::{logic::ClientLogic, login_state::LoginState};
use async_channel::{Receiver, Sender};
use async_handle::Handle;
use basws_shared::protocol::{ServerRequest, WsRequest};
use once_cell::sync::OnceCell;

static REQUEST_COUNTER: OnceCell<Handle<u64>> = OnceCell::new();

pub(crate) struct ClientData<L>
where
    L: ClientLogic + ?Sized,
{
    pub(crate) logic: Box<L>,
    pub(crate) login_state: LoginState,
    pub(crate) sender: Sender<WsRequest<L::Request>>,
    pub(crate) receiver: Receiver<WsRequest<L::Request>>,
    pub(crate) average_roundtrip: f64,
    pub(crate) average_server_timestamp_delta: f64,
}

impl<L> ClientData<L>
where
    L: ClientLogic + 'static,
{
    pub async fn send(&self, request: ServerRequest<L::Request>) -> anyhow::Result<()> {
        let id = {
            let mut counter = REQUEST_COUNTER.get_or_init(|| Handle::new(0)).write().await;
            *counter = counter.wrapping_add(1);
            *counter
        };
        self.sender.send(WsRequest { id, request }).await?;
        Ok(())
    }
}
