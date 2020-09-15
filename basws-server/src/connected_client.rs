use async_channel::Sender;
use async_handle::Handle;
use basws_shared::{
    protocol::{InstallationConfig, WsBatchResponse},
    timing::NetworkTiming,
};

pub struct ConnectedClient<Response, Account> {
    pub installation: Option<InstallationConfig>,
    pub(crate) nonce: Option<[u8; 32]>,
    sender: Sender<WsBatchResponse<Response>>,
    pub account: Option<Handle<Account>>,
    pub network_timing: NetworkTiming,
}

impl<Response, Account> ConnectedClient<Response, Account>
where
    Response: Send + Sync + 'static,
{
    pub fn new(sender: Sender<WsBatchResponse<Response>>) -> Self {
        Self {
            sender,
            nonce: None,
            account: None,
            installation: None,
            network_timing: Default::default(),
        }
    }

    pub fn new_with_installation(
        installation: InstallationConfig,
        sender: Sender<WsBatchResponse<Response>>,
    ) -> Self {
        Self {
            sender,
            nonce: None,
            account: None,
            installation: Some(installation),
            network_timing: Default::default(),
        }
    }

    pub async fn send(&self, response: WsBatchResponse<Response>) -> anyhow::Result<()> {
        Ok(self.sender.send(response).await?)
    }
}
