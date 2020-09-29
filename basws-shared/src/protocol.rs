use crate::challenge;
use semver::{Version, VersionReq};
use serde_derive::{Deserialize, Serialize};
use std::convert::TryInto;
use uuid::Uuid;

pub fn protocol_version() -> Version {
    Version::parse("0.1.0-dev-1").unwrap()
}

pub fn protocol_version_requirements() -> VersionReq {
    VersionReq::parse("=0.1.0-dev-1").unwrap()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WsRequest<T> {
    pub id: u64,
    pub request: ServerRequest<T>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WsBatchResponse<T> {
    pub request_id: Option<u64>,
    pub results: Vec<ServerResponse<T>>,
}

impl<T> Default for WsBatchResponse<T> {
    fn default() -> Self {
        Self {
            request_id: None,
            results: Vec::default(),
        }
    }
}

impl<T> WsBatchResponse<T> {
    pub fn new(request_id: Option<u64>, results: Vec<ServerResponse<T>>) -> Self {
        Self {
            request_id,
            results,
        }
    }

    pub fn from_result(result: ServerResponse<T>) -> Self {
        Self::new(None, vec![result])
    }

    pub fn from_results(results: Vec<ServerResponse<T>>) -> Self {
        Self::new(None, results)
    }

    pub fn from_response(response: T) -> Self {
        Self::new(None, vec![ServerResponse::Response(response)])
    }
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServerRequest<T> {
    Greetings {
        protocol_version: String,
        server_version: String,
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
        average_roundtrip: Option<f64>,
        average_server_timestamp_delta: Option<f64>,
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
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Eq, PartialEq)]
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

impl InstallationConfig {
    pub fn from_vec(id: Uuid, private_key: Vec<u8>) -> Result<Self, InstallationConfigError> {
        let slice = private_key.into_boxed_slice();
        let private_key: Box<[u8; 32]> = slice
            .try_into()
            .map_err(|_| InstallationConfigError::InvalidPrivateKey)?;
        Ok(Self {
            id,
            private_key: *private_key,
        })
    }
}

#[derive(thiserror::Error, Debug)]
pub enum InstallationConfigError {
    #[error("invalid private key (must be 32 bytes long)")]
    InvalidPrivateKey,
}
