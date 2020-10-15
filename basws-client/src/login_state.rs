use basws_shared::{protocol::InstallationConfig, Uuid};
use std::fmt::Debug;

#[derive(Debug, Clone)]
pub enum LoginState {
    Disconnected,
    Handshaking { config: Option<InstallationConfig> },
    Connected { installation_id: Uuid },
    Error { message: Option<String> },
}

impl Default for LoginState {
    fn default() -> Self {
        LoginState::Disconnected
    }
}
