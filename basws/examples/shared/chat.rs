use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

pub const PROTOCOL_VERSION: &str = "0.0.1";
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
