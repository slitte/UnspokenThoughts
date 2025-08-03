// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs

use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc;
use tokio::io::AsyncReadExt;
use bytes::{BytesMut, Buf};
use prost::Message;
use std::time::Duration;

use crate::event::{Event, EventType};
use crate::mesh_proto::from_radio::PayloadVariant;
use crate::mesh_proto::FromRadio;

const BAUDRATE: u32 = 921600;

pub async fn read_port(port_name: String, tx: mpsc::UnboundedSender<Event>) {
    let mut buffer = BytesMut::with_capacity(4096);

    loop {
        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(mut port) => {
                log::info!("Lausche auf {}", port_name);

                loop {
                    // --- Protobuf-Byte-Stream einlesen ---
                    let mut tmp = [0u8; 512];
                    let n = match port.read(&mut tmp).await {
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
                    buffer.extend_from_slice(&tmp[..n]);

                    // --- Framing und Decode ---
                    while buffer.len() > 2 {
                        let len = u16::from_le_bytes([buffer[0], buffer[1]]) as usize;
                        if buffer.len() < 2 + len {
                            break;
                        }
                        // gesamtes Nachricht-Bytearray extrahieren
                        let msg_bytes = buffer[2..2+len].to_vec();
                        buffer.advance(2 + len);

                        match FromRadio::decode(&*msg_bytes) {
                            Ok(msg) => {
                                if let Some(variant) = msg.payload_variant {
                                    // je nach Variant ein Event bauen
                                    let event = match variant {
                                        PayloadVariant::Packet(packet) => {
                                            // Direct vs. Relay anhand hop_limit
                                            let evtype = if packet.hop_limit > 0 {
                                                EventType::RelayedMesh {
                                                    from: packet.from,
                                                    to: packet.to,
                                                }
                                            } else {
                                                EventType::DirectMesh {
                                                    from: packet.from,
                                                    to: packet.to,
                                                }
                                            };
                                            Event { port: port_name.clone(), event_type: evtype }
                                        }
                                        PayloadVariant::NodeInfo(info) => {
                                            Event {
                                                port: port_name.clone(),
                                                event_type: EventType::NodeInfo {
                                                    node_id: info.num
                                                },
                                            }
                                        }

                                        _ => {
                                            // alle anderen Varianten überspringen
                                            Event {
                                                port:       port_name.clone(),
                                                event_type: EventType::Unknown,
                                            }
                                        }
                                    };
                                    let _ = tx.send(event);
                                }
                            }
                            Err(e) => {
                                log::warn!("[{}] Prost-Decode-Error: {:?}", port_name, e);
                            }
                        }
                    }
                }

                log::info!("[{}] Warte auf Reconnect…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Konnte Port nicht öffnen: {:?}", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}