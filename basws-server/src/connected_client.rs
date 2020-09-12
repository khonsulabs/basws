use crate::{AccountHandle, WebsocketServerLogic};
use async_channel::Sender;
use uuid::Uuid;
use basws_shared::{protocol::WsBatchResponse, timing::NetworkTiming};

pub struct ConnectedClient<L>
where
    L: WebsocketServerLogic,
{
    pub installation_id: Option<Uuid>,
    pub sender: Sender<WsBatchResponse<L::Response>>,
    pub account: Option<AccountHandle<L::AccountId, L::Account>>,
    pub network_timing: NetworkTiming,
}
