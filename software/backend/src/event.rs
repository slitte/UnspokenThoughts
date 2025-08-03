// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <event.rs>


use crate::message::{PortMessage};
use serde::{Serialize, Deserialize};


#[derive(Debug, Serialize, Deserialize)]
pub enum Event {
    MeshMessage(PortMessage),
    NodeInfo(String),
    Error(String),

    TextMessage {
        port: String,
        message: String,
    },
}