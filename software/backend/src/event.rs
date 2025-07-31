use crate::message::{PortMessage};
use serde::{Serialize, Deserialize};

#[derive(Debug, Serialize, Deserialize)]
pub enum Event {
    MeshMessage(PortMessage),
    NodeInfo(String), // später evtl. strukturieren
    Error(String),
}