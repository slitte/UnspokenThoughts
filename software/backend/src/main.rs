// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <main.rs>

mod event;
mod message;
mod logging;
mod port_handler;
mod tcp_server;

mod mesh_proto {
    include!(concat!(env!("OUT_DIR"), "/meshtastic.rs"));
}

use tokio::sync::{mpsc, Mutex};
use tokio::signal;
use std::sync::Arc;
use event::Event;
use logging::init_logging;
use tcp_server::start_tcp_server;

const PORTS: &[(&str, u32)] = &[
    ("/dev/UT_Long-Fast", 12345678),
];

const TCP_ADDR: &str = "127.0.0.1:9000";

#[tokio::main]
async fn main() {
    init_logging();

    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let clients = Arc::new(Mutex::new(Vec::new()));

    // TCP-Server starten
    let tcp_clients = Arc::clone(&clients);
    tokio::spawn(async move {
        start_tcp_server(tcp_clients, TCP_ADDR).await;
    });

    // Serial-Ports starten (gepaart)
    for (port, _node_id) in PORTS {
        let tx = tx.clone();
        let port = port.to_string();
        tokio::spawn(async move {
            port_handler::read_port(port, tx).await;
        });
    }

    // Eventloop und Signal-Handling (sauber beenden bei Strg+C)
    tokio::select! {
        _ = async {
            // Eventloop: Nachrichten empfangen, an alle TCP-Clients weiterleiten
            while let Some(event) = rx.recv().await {
                log::info!("Empfangen von {}: {:?}", event.port, event.event_type);
                let mut clients = clients.lock().await;
                clients.retain_mut(|stream| {
                    if let Ok(json) = serde_json::to_string(&event) {
                        match stream.try_write((json.clone() + "\n").as_bytes()) {
                            Ok(_) => true,
                            Err(_) => false,
                        }
                    } else {
                        false
                    }
                });
            }
        } => {}
        _ = signal::ctrl_c() => {
            log::info!("Strg+C empfangen – Programm wird sauber beendet.");
        }
    }

    log::info!("UnspokenThoughts ist beendet.");
}
