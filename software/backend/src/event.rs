// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <event.rs>


use serde::{Serialize, Deserialize};
use serde_json::Value; // ← neu

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum EventType {
    DirectMesh  { from: u32, to: u32 },
    RelayedMesh { from: u32, to: u32 },
    NodeInfo    { node_id: u32     },
    NodeInfoJson(Value),            // ← neu
    Unknown,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Event {
    pub port: String,
    pub event_type: EventType,
}