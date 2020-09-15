use crate::challenge;
use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

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

impl<T> WsBatchResponse<T> {
    pub fn new(request_id: i64, results: Vec<ServerResponse<T>>) -> Self {
        Self {
            request_id,
            results,
        }
    }

    pub fn from_result(result: ServerResponse<T>) -> Self {
        Self::new(-1, vec![result])
    }

    pub fn from_results(results: Vec<ServerResponse<T>>) -> Self {
        Self::new(-1, results)
    }

    pub fn from_response(response: T) -> Self {
        Self::new(-1, vec![ServerResponse::Response(response)])
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerRequest<T> {
    Greetings {
        version: String,
        installation_id: Option<Uuid>,
    },
    ChallengeResponse([u8; 32]),
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
    NewInstallation(InstallationConfig),
    Challenge {
        nonce: [u8; 32],
    },
    Connected {
        installation_id: Uuid,
    },
    Response(T),
    Error(ServerError),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerError {
    IncompatibleVersion,
    Other(String),
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub struct InstallationConfig {
    pub id: Uuid,
    pub private_key: [u8; 32],
}

impl Default for InstallationConfig {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            private_key: challenge::nonce(),
        }
    }
}
