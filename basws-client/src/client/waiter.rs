use crate::{client::Client, logic::ClientLogic};

#[derive(Clone)]
pub struct PendingResponse<T>
where
    T: ClientLogic,
{
    request_id: u64,
    client: Client<T>,
}

impl<T> PendingResponse<T>
where
    T: ClientLogic + 'static,
{
    pub(crate) fn new(request_id: u64, client: Client<T>) -> Self {
        Self { request_id, client }
    }

    pub async fn receive_response(&self) -> Result<Vec<T::Response>, async_channel::RecvError> {
        self.client.wait_for_response_to(self.request_id).await
    }
}
