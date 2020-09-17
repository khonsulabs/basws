use crate::{logic::{Identifiable, ServerLogic}, connected_client::ConnectedClient, server::{Server, RequestHandling}};
use serde::{de::DeserializeOwned, Serialize, Deserialize};
use std::{fmt::Debug, borrow::Borrow, collections::HashMap};
use async_trait::async_trait;
use basws_shared::{Uuid, VersionReq, protocol::InstallationConfig};
use async_handle::Handle;

pub type PersistentConnectedClient<T> = ConnectedClient<PersistentServer<T>>;
pub type PersistentServerHandle<T> = Server<PersistentServer<T>>;

#[async_trait]
pub trait PersistentServerLogic: Send + Sync + 'static {
    type Request: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;
    type Response: Serialize + DeserializeOwned + Clone + Send + Sync + Debug;
    type Client: Serialize + DeserializeOwned + Send + Sync + Debug;
    type Account: Identifiable<Id = Uuid> + Serialize + DeserializeOwned + Send + Sync + Debug;

    fn protocol_version_requirements(&self) -> VersionReq;

    async fn initialize_client_for(&self, 
        client: &PersistentConnectedClient<Self>,) -> anyhow::Result<Self::Client>;

    async fn handle_request(
        &self,
        client: &PersistentConnectedClient<Self>,
        request: Self::Request,
        server: &PersistentServerHandle<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;

    async fn client_reconnected(
        &self,
        client: &PersistentConnectedClient<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;

    async fn new_client_connected(
        &self,
        client: &PersistentConnectedClient<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>;
}

pub struct PersistentServer<L: ?Sized>
where L: PersistentServerLogic {
    logic: Box<L>,
    database: sled::Db,
    connected_clients: Handle<HashMap<Uuid, Handle<PersistentClient<L::Client>>>>,
}

impl<L> PersistentServer<L>
where L: PersistentServerLogic {
    pub fn new(logic: L, database_path: &std::path::Path) -> Result<PersistentServerHandle<L>, sled::Error> {
        let database = sled::open(database_path)?;
        Ok(Server::new(Self { logic: Box::new(logic), database, connected_clients: Default::default() }))
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PersistentClient<T> {
    pub installation: InstallationConfig,
    pub client: T,
}

#[async_trait]
impl<L> ServerLogic for PersistentServer<L>
where
    L: PersistentServerLogic + ?Sized + 'static,
{
    type Request = L::Request;
    type Response = L::Response;
    type Client = Option<PersistentClient<L::Client>>;
    type Account = L::Account;
    type AccountId = Uuid;

    async fn handle_request(
        &self,
        client: &ConnectedClient<Self>,
        request: Self::Request,
        server: &Server<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>> {
        self.logic.handle_request(client, request, server).await
    }

    async fn lookup_account_from_installation_id(
        &self,
        installation_id: Uuid,
    ) -> anyhow::Result<Option<Handle<Self::Account>>>{
        info!("Looking up {}", installation_id);
        let account_id = self.load_installation_account_id(installation_id).await?;
        info!("Got account {:?}", account_id);

        let account = if let Some(account_id) = account_id {
            self.load_account(account_id).await?
        } else {
            None
        };

        Ok(account)
    }

    fn protocol_version_requirements(&self) -> VersionReq {
        self.logic.protocol_version_requirements()
    }

    async fn lookup_or_create_installation(
        &self,
        client: &ConnectedClient<Self>,
        installation_id: Option<Uuid>,
    ) -> anyhow::Result<InstallationConfig>{
        if let Some(installation_id) = installation_id {
            info!("Loading client up {}", installation_id);
            if let Some(client) = self.load_client(installation_id).await? {
                info!("Loaded client {:?}", client);
                let client = client.read().await;
                return Ok(client.installation);
            }
        }

        info!("Creating new installation");
        let installation = InstallationConfig::default();
        let new_client = PersistentClient {
            installation,
            client: self.logic.initialize_client_for(client).await?,
        };
        self.save_persistent_client(&new_client).await?;

        let persistent_client = client.client().await;
        let mut persistent_client = persistent_client.write().await;
        *persistent_client = Some(new_client);

        Ok(installation)
    }

    async fn client_reconnected(
        &self,
        client: &ConnectedClient<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>{
        self.logic.client_reconnected(client).await
    }

    async fn new_client_connected(
        &self,
        client: &ConnectedClient<Self>,
    ) -> anyhow::Result<RequestHandling<Self::Response>>{
        self.logic.new_client_connected(client).await
    }

    async fn account_associated(&self, client: &ConnectedClient<Self>) -> anyhow::Result<()> {
        info!("Account associated");
        let installation_id = client.installation().await.unwrap().id;
        let account = client.account().await.unwrap();
        let account = account.read().await;
        self.save_installation_account_id(installation_id, account.id()).await
    }
}

impl<L> PersistentServer<L>
where L: PersistentServerLogic + ?Sized + 'static {
    pub fn database(&self) -> &'_ sled::Db {
        &self.database
    }

    pub async fn load_account(&self, account_id: Uuid) -> anyhow::Result<Option<Handle<L::Account>>> {
        Ok(self.load(b"accounts", account_id.as_bytes()).await?.map(Handle::new))
    }

    pub async fn save_account(&self, account: &Handle<L::Account>) -> anyhow::Result<()> {
        let account = account.read().await;
        let account: &L::Account = account.borrow();
        self.save(b"accounts", account.id().as_bytes(), account).await
    }

    pub async fn load_client(&self, id: Uuid) -> anyhow::Result<Option<Handle<PersistentClient<L::Client>>>> {
        let mut connected_clients = self.connected_clients.write().await;

        if let Some(client) = connected_clients.get(&id) {
            return Ok(Some(client.clone()));
        }

        if let Some(client) = self.load(b"installations", id.as_bytes()).await? {
            let client = Handle::new(client);
            connected_clients.insert(id, client.clone());
            Ok(Some(client))
        } else {
            Ok(None)
        }
    }

    pub async fn save_client(&self, client: &ConnectedClient<Self>) -> anyhow::Result<()> {
        let persistent_client = client.client().await;
        let persistent_client = persistent_client.read().await;
        if let Some(persistent_client) = persistent_client.as_ref() {
            self.save_persistent_client(persistent_client).await
        } else {
            anyhow::bail!("Saving an uninitialized client")
        }
    }

    pub async fn save_persistent_client(&self, client: &PersistentClient<L::Client>) -> anyhow::Result<()> {
        self.save(b"installations", client.installation.id.as_bytes(), client).await
    }

    pub async fn load<T: DeserializeOwned>(&self, keyspace: &[u8], id: &[u8]) -> anyhow::Result<Option<T>> {
        let tree = self.database.open_tree(keyspace)?;
        let value = tree.get(id)?;
        let handle = if let Some(value) = value {
            Some(serde_cbor::from_slice(&value)?)
        } else {
            None
        };

        Ok(handle)
    }

    pub async fn save<T: Serialize>(&self, keyspace: &[u8], id: &[u8], value: &T) -> anyhow::Result<()> {
        let tree = self.database.open_tree(keyspace)?;
        tree.insert(id, serde_cbor::to_vec(value)?)?;
        tree.flush_async().await?;
        
        Ok(())
    }

    async fn load_installation_account_id(&self, uuid: Uuid) -> anyhow::Result<Option<Uuid>> {
        self.load(b"installation_accounts", uuid.as_bytes()).await
    }

    async fn save_installation_account_id(&self, uuid: Uuid, account_id: Uuid) -> anyhow::Result<()> {
        self.save(b"installation_accounts", uuid.as_bytes(), &account_id).await
    }
}