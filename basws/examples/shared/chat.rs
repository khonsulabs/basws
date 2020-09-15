use basws::shared::{Uuid, Version, VersionReq};
use serde_derive::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

pub fn protocol_version() -> Version {
    Version::parse("0.1.1").unwrap()
}

pub fn protocol_version_requirements() -> VersionReq {
    VersionReq::parse("=0.1").unwrap()
}

pub const SERVER_PORT: u16 = 12345;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatRequest {
    Login { username: String }, // Super secure, no password
    Chat { message: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatResponse {
    LoggedIn { username: String },
    Unauthenticated,
    ChatReceived { from: ChatSender, message: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatSender {
    Anonymous(Uuid),
    User(String),
}

impl Display for ChatSender {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Anonymous(uuid) => f.write_fmt(format_args!("<{}>", uuid)),
            Self::User(username) => f.write_str(username),
        }
    }
}
