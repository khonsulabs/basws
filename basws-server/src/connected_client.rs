use async_channel::Sender;
use async_handle::Handle;
use basws_shared::{
    protocol::{InstallationConfig, WsBatchResponse},
    timing::NetworkTiming,
};

pub struct ConnectedClient<Response, Account> {
    data: Handle<ConnectedClientData<Response, Account>>,
}

impl<Response, Account> Clone for ConnectedClient<Response, Account> {
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}

struct ConnectedClientData<Response, Account> {
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
            data: Handle::new(ConnectedClientData {
                sender,
                nonce: None,
                account: None,
                installation: None,
                network_timing: Default::default(),
            }),
        }
    }

    pub fn new_with_installation(
        installation: InstallationConfig,
        sender: Sender<WsBatchResponse<Response>>,
    ) -> Self {
        Self {
            data: Handle::new(ConnectedClientData {
                sender,
                nonce: None,
                account: None,
                installation: Some(installation),
                network_timing: Default::default(),
            }),
        }
    }

    pub async fn send(&self, response: WsBatchResponse<Response>) -> anyhow::Result<()> {
        let data = self.data.read().await;
        Ok(data.sender.send(response).await?)
    }

    pub async fn installation(&self) -> Option<InstallationConfig> {
        let data = self.data.read().await;
        data.installation
    }

    pub async fn set_installation(&self, installation: InstallationConfig) {
        let mut data = self.data.write().await;
        data.installation = Some(installation);
    }

    pub async fn account(&self) -> Option<Handle<Account>> {
        let data = self.data.read().await;
        data.account.clone()
    }

    pub async fn set_account(&self, account: Handle<Account>) {
        let mut data = self.data.write().await;
        data.account = Some(account);
    }

    pub async fn nonce(&self) -> Option<[u8; 32]> {
        let data = self.data.read().await;
        data.nonce
    }

    pub(crate) async fn set_nonce(&self, nonce: [u8; 32]) {
        let mut data = self.data.write().await;
        data.nonce = Some(nonce);
    }

    pub(crate) async fn update_network_timing(&self, original_timestamp: f64, timestamp: f64) {
        let mut data = self.data.write().await;
        data.network_timing.update(original_timestamp, timestamp)
    }
}
