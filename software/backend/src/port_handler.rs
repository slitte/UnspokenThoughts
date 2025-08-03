// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs


use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc;
use prost::Message;
use std::time::Duration;
use bytes::{BytesMut};
use serde_json::Value;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32 = 921600;

pub async fn read_port(port_name: String, tx: mpsc::UnboundedSender<Event>) {
    // gemeinsamer Buffer für alles
    let mut buffer = BytesMut::with_capacity(4096);

    loop {
        match tokio_serial::new(&port_name, BAUDRATE)
            .open_native_async()
        {
            Ok(mut port) => {
                log::info!("Lausche auf {}", port_name);

                loop {
                    // 1) Bytes vom Port holen
                    let mut tmp = [0u8; 512];
                    let n = match tokio::io::AsyncReadExt::read(&mut port, &mut tmp).await {
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

                    // 2) JSON-NodeInfo abfangen (eine ganze Zeile „{…}\n“)
                    while buffer.first().copied() == Some(b'{') {
                        if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                            let line = buffer.split_to(pos + 1);
                            if let Ok(s) = std::str::from_utf8(&line) {
                                if let Ok(json) = serde_json::from_str::<Value>(s.trim()) {
                                    // JSON-NodeInfo als Event verschicken
                                    let event = Event {
                                        port: port_name.clone(),
                                        event_type: EventType::NodeInfo { node_id: json["num"].as_u64().unwrap_or(0) as u32 },
                                    };
                                    let _ = tx.send(event);
                                    continue;
                                }
                            }
                        }
                        // wenn kein kompletter JSON-Line da ist, raus aus der JSON-Schleife
                        break;
                    }

                    // 3) Protobuf-Framing: solange ein komplettes Paket da ist
                    while buffer.len() > 2 {
                        let len = u16::from_le_bytes([buffer[0], buffer[1]]) as usize;
                        if buffer.len() < 2 + len { break; }
                        // Paket aus dem Buffer nehmen
                        let msg_bytes = buffer.split_to(2 + len).split_off(2);
                        // und decodieren
                        match mesh_proto::FromRadio::decode(&*msg_bytes) {
                            Ok(msg) => {
                                if let Some(variant) = msg.payload_variant {
                                    let event = match variant {
                                        PayloadVariant::Packet(p) => {
                                            log::info!("[{}] Packet erhalten: {} → {}, hop_limit={}", port_name, p.from, p.to, p.hop_limit);
                                            let from = p.from;
                                            let to   = p.to;
                                            let ty = if p.hop_limit > 0 {
                                                EventType::RelayedMesh { from, to }
                                            } else {
                                                EventType::DirectMesh  { from, to }
                                            };
                                            Event { port: port_name.clone(), event_type: ty }
                                        }
                                        PayloadVariant::NodeInfo(info) => {
                                            log::info!("[{}] NodeInfo erhalten für Node {}", port_name, info.num);
                                            Event {
                                                port:       port_name.clone(),
                                                event_type: EventType::NodeInfo { node_id: info.num },
                                            }
                                        }
                                        _ => Event {
                                            port:       port_name.clone(),
                                            event_type: EventType::Unknown,
                                        },
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