// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <messages.rs>

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