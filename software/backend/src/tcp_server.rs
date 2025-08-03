// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <tcp_server.rs>

use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

pub async fn start_tcp_server(clients: Arc<Mutex<Vec<TcpStream>>>, addr: &str) {
    let listener = TcpListener::bind(addr).await.expect("TCP-Server konnte nicht starten");
    log::info!("[TCP] Lausche auf {}", addr);

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                log::info!("[TCP] Neuer Client: {}", addr);
                stream.set_nodelay(true).ok();
                clients.lock().await.push(stream);
            }
            Err(e) => log::warn!("[TCP] Verbindungsfehler: {:?}", e),
        }
    }
}