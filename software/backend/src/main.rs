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

const PORTS: [&str; 8] = [
    "/dev/ttyUSB0", "/dev/ttyUSB1", "/dev/ttyUSB2", "/dev/ttyUSB3",
    "/dev/ttyUSB4", "/dev/ttyUSB5", "/dev/ttyUSB6", "/dev/ttyUSB7",
];

#[tokio::main]
async fn main() {
    init_logging();
    let (tx, mut rx) = mpsc::unbounded_channel::<Event>();
    let clients = Arc::new(Mutex::new(Vec::new()));

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
                    Event::Error(e) => log::warn!("[Fehler] {}", e),
                    Event::NodeInfo(info) => log::info!("[NodeInfo] {:?}", info),
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
