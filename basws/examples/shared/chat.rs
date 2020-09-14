use serde_derive::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatRequest {
    Login { username: String }, // Super secure, no password
    Chat { message: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatResponse {
    LoggedIn { username: String },
    ChatReceived { from: ChatSender, message: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub enum ChatSender {
    Anonymous(Uuid),
    User(String),
}
