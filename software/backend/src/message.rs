//message.rs

use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MeshMessage {
    pub from: Option<String>,
    pub to: Option<String>,
    pub text: Option<String>,
    // Weitere Felder möglich (z. B. decoded)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PortMessage {
    pub port: String,
    pub raw: String,
    pub parsed: Option<MeshMessage>,
}
