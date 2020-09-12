use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug)]
pub enum Message<T> {
    Initialize,
    Reset,
    Message(WsBatchResponse<T>),
    Connected,
}

pub trait ConnectedMessage
where
    Self: Sized,
{
    fn connected() -> Self;
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WsRequest<T> {
    pub id: i64,
    pub request: ServerRequest<T>,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
pub struct WsBatchResponse<T> {
    pub request_id: i64,
    pub results: Vec<ServerResponse<T>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerRequest<T> {
    Authenticate {
        version: String,
        installation_id: Option<Uuid>,
    },
    Pong {
        original_timestamp: f64,
        timestamp: f64,
    },
    Request(T),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerResponse<T> {
    Ping {
        timestamp: f64,
        average_roundtrip: f64,
        average_server_timestamp_delta: f64,
    },
    Response(T),
    Error(ServerError),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerError {
    IncompatibleVersion,
    Other(String),
}
