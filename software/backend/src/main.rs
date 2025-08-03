// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <main.rs>

mod port_handler;
mod tcp_server;
mod event;
mod message;
mod logging;

use crate::event::Event;
use crate::port_handler::PortHandler;
use crate::tcp_server::start_tcp_server;

use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::signal;
use logging::init_logging;

use std::fs::OpenOptions;
use std::io::Write;


const PORTS: [&str; 1] = [
    "/dev/UT_Long-Fast"];

#[tokio::main]
async fn main() {
    init_logging();
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let clients = Arc::new(Mutex::new(Vec::new()));

    log::info!("Build-Trigger-Test");
    // Datei zum Mitschreiben der Events öffnen
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open("/home/schlitte/event_log.txt")
        .expect("Konnte event_log.txt nicht öffnen");










    // TCP-Server starten
    let tcp_clients = Arc::clone(&clients);
    tokio::spawn(async move {
        start_tcp_server(tcp_clients).await;
    });

    // Ports starten
    for port in PORTS {
        let tx = tx.clone();
        let port = port.to_string();
        tokio::spawn(async move {
            let mut handler = PortHandler::new(port, tx);
            handler.run().await;
        });
    }

    // Shutdown-Handler
    let mut shutdown = tokio::spawn(async {
        signal::ctrl_c().await.expect("Fehler beim Warten auf Ctrl+C");
        log::info!("Ctrl+C erkannt, beende...");
    });

    // Event-Verarbeitung
    loop {
        tokio::select! {
            Some(event) = rx.recv() => {
                match &event {
    Event::MeshMessage(msg) => {
        log::info!("[Sternschnuppe] Von {}: {}", msg.port, msg.raw);
    }
    Event::TextMessage { port, message } => {
        log::info!("[Text] {} → {}", port, message);
    }
    Event::Error(e) => log::warn!("[Fehler] {}", e),
    Event::NodeInfo(info) => log::info!("[NodeInfo] {:?}", info),
}
                
                 // In Datei schreiben
                if let Ok(json) = serde_json::to_string(&event) {
                    if let Err(e) = writeln!(file, "{}", json) {
                        log::warn!("Fehler beim Schreiben in Datei: {}", e);
                    }
                }




                // TCP weiterleiten
                let mut clients = clients.lock().await;
                clients.retain_mut(|stream| {
                    if let Ok(json) = serde_json::to_string(&event) {
                        match stream.try_write((json + "\n").as_bytes()) {
                            Ok(_) => true,
                            Err(_) => false,
                        }
                    } else {
                        false
                    }
                });
            },
            _ = &mut shutdown => {
                break;
            }
        }
    }

    log::info!("Hauptloop beendet");
}