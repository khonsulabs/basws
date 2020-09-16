use crate::logic::ServerLogic;
use async_channel::Sender;
use async_handle::Handle;
use basws_shared::{
    protocol::{InstallationConfig, WsBatchResponse},
    timing::NetworkTiming,
};

pub struct ConnectedClient<L>
where
    L: ServerLogic + ?Sized,
{
    data: Handle<ConnectedClientData<L>>,
}

impl<L> Clone for ConnectedClient<L>
where
    L: ServerLogic,
{
    fn clone(&self) -> Self {
        Self {
            data: self.data.clone(),
        }
    }
}

struct ConnectedClientData<L>
where
    L: ServerLogic + ?Sized,
{
    client: Handle<L::Client>,
    pub installation: Option<InstallationConfig>,
    pub(crate) nonce: Option<[u8; 32]>,
    sender: Sender<WsBatchResponse<L::Response>>,
    pub account: Option<Handle<L::Account>>,
    pub network_timing: NetworkTiming,
}

impl<L> ConnectedClient<L>
where
    L: ServerLogic + 'static,
{
    pub(crate) fn new(client: L::Client, sender: Sender<WsBatchResponse<L::Response>>) -> Self {
        Self::new_with_installation(None, client, sender)
    }

    pub(crate) fn new_with_installation(
        installation: Option<InstallationConfig>,
        client: L::Client,
        sender: Sender<WsBatchResponse<L::Response>>,
    ) -> Self {
        Self {
            data: Handle::new(ConnectedClientData {
                sender,
                client: Handle::new(client),
                nonce: None,
                account: None,
                installation,
                network_timing: Default::default(),
            }),
        }
    }

    pub async fn send(&self, response: WsBatchResponse<L::Response>) -> anyhow::Result<()> {
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

    pub async fn client(&self) -> Handle<L::Client> {
        let data = self.data.read().await;
        data.client.clone()
    }

    pub async fn account(&self) -> Option<Handle<L::Account>> {
        let data = self.data.read().await;
        data.account.clone()
    }

    pub async fn set_account(&self, account: Handle<L::Account>) {
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
