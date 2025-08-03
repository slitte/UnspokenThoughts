// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// Filename: <port_handler.rs>



use bytes::{BytesMut, Buf};
use tokio::io::AsyncReadExt;
use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc;
use prost::Message; // <--- nicht vergessen!
use crate::mesh_proto::FromRadio; // <--- passt für dein Projekt
use crate::Event; // dein Event-Struct, ggf. anpassen

const BAUDRATE: u32 = 921600;

pub async fn read_port(port_name: String, tx: mpsc::UnboundedSender<Event>) {
    loop {
        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(mut port) => {
                let mut buffer = BytesMut::with_capacity(4096);
                log::info!("Lausche auf {}", port_name);

                loop {
                    let mut temp = [0u8; 512];
                    let n = match port.read(&mut temp).await {
                        Ok(0) => {
                            log::warn!("[{}] Port geschlossen", port_name);
                            break;
                        }
                        Ok(n) => n,
                        Err(e) => {
                            log::warn!("[{}] Lesefehler: {:?}", port_name, e);
                            break;
                        }
                    };
                    buffer.extend_from_slice(&temp[..n]);

                    // --- Protobuf-Framing ---
                    while buffer.len() > 2 {
                        let len = u16::from_le_bytes([buffer[0], buffer[1]]) as usize;
                        if buffer.len() < 2 + len {
                            break;
                        }
                        let msg_bytes = &buffer[2..2+len];

                        match FromRadio::decode(msg_bytes) {
                            Ok(msg) => {
                                let event = Event {
                                    port: port_name.clone(),
                                    proto: format!("{:?}", msg),
                                };
                                let _ = tx.send(event);
                            }
                            Err(e) => {
                                log::warn!("[{}] Dekodierungsfehler: {:?}", port_name, e);
                            }
                        }
                        buffer.advance(2 + len);
                    }
                }

                log::info!("[{}] Warte auf Reconnect...", port_name);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Konnte Port nicht öffnen: {:?}", port_name, e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}
