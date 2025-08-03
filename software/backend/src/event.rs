// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <event.rs>


use serde::Serialize;

#[derive(Debug, Serialize, Clone)]
pub enum EventType {
    DirectMesh { from: u32, to: u32 },
    RelayedMesh { from: u32, to: u32 },
    NodeInfo   { node_id: u32, }, // Liste aktueller Node-IDs
    Unknown,
}
#[derive(Debug, Serialize, Clone)]
pub struct Event {
    pub port: String,
    pub event_type: EventType,
    // Optional: Zusatzinfos (from/to/text etc)
}