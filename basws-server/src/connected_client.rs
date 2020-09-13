use crate::{AccountHandle, WebsocketServerLogic};
use async_channel::Sender;
use basws_shared::{protocol::WsBatchResponse, timing::NetworkTiming};
use uuid::Uuid;

pub struct ConnectedClient<L>
where
    L: WebsocketServerLogic,
{
    pub installation_id: Option<Uuid>,
    sender: Sender<WsBatchResponse<L::Response>>,
    pub account: Option<AccountHandle<L::AccountId, L::Account>>,
    pub network_timing: NetworkTiming,
}

impl<L> ConnectedClient<L>
where
    L: WebsocketServerLogic,
{
    pub fn new(sender: Sender<WsBatchResponse<L::Response>>) -> Self {
        Self {
            sender,
            account: None,
            installation_id: None,
            network_timing: Default::default(),
        }
    }

    pub fn new_with_installation_id(
        installation_id: Uuid,
        sender: Sender<WsBatchResponse<L::Response>>,
    ) -> Self {
        Self {
            sender,
            account: None,
            installation_id: Some(installation_id),
            network_timing: Default::default(),
        }
    }
}
