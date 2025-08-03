// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs

use tokio_serial::SerialPortBuilderExt;
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::UnboundedSender;
use prost::Message;
use std::time::Duration;
use bytes::BytesMut;
use serde_json::Value;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32 = 921600;

pub async fn read_port(port_name: String, tx: UnboundedSender<Event>) {
    // Gemeinsamer Buffer für JSON- und Protobuf-Daten
    let mut buffer = BytesMut::with_capacity(4096);

    loop {
        log::info!("Versuche Port \"{}\" mit {} Baud zu öffnen…", port_name, BAUDRATE);
        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(mut port) => {
                log::info!("[{}] Port geöffnet, starte Lese-Loop", port_name);

                loop {
                    // 1) Bytes vom Port lesen
                    let mut tmp = [0u8; 512];
                    let n = match port.read(&mut tmp).await {
                        Ok(0) => {
                            log::warn!("[{}] EOF empfangen – breche Lese-Loop ab", port_name);
                            break;
                        }
                        Ok(n) => {
                            log::debug!("[{}] {} Bytes eingelesen", port_name, n);
                            n
                        }
                        Err(e) => {
                            log::warn!("[{}] Lesefehler: {:?}", port_name, e);
                            break;
                        }
                    };
                    buffer.extend_from_slice(&tmp[..n]);

                    // 2) JSON-NodeInfo abfangen (ganze Zeile “{…}\n”)
                    while buffer.first().copied() == Some(b'{') {
                        if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                            let line = buffer.split_to(pos + 1);
                            if let Ok(txt) = std::str::from_utf8(&line) {
                                log::debug!("[{}] JSON-Zeile empfangen: {}", port_name, txt.trim());
                                if let Ok(json) = serde_json::from_str::<Value>(txt.trim()) {
                                    let node_id = json.get("num")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0) as u32;
                                    log::info!("[{}] JSON-NodeInfo: num={}", port_name, node_id);
                                    let event = Event {
                                        port: port_name.clone(),
                                        event_type: EventType::NodeInfo { node_id },
                                    };
                                    let _ = tx.send(event);
                                } else {
                                    log::warn!("[{}] Ungültiges JSON: {}", port_name, txt.trim());
                                }
                            }
                            continue; // prüfe, ob noch weitere JSON-Zeilen anstehen
                        }
                        break;
                    }

                    // 3) Protobuf-Frames entpacken
                    while buffer.len() >= 2 {
                        // 3a) Längenpräfix (u16, little-endian)
                        let len = u16::from_le_bytes([buffer[0], buffer[1]]) as usize;
                        if buffer.len() < 2 + len {
                            log::debug!("[{}] Warte auf kompletten Frame ({} Bytes erwartet)", port_name, len);
                            break;
                        }

                        // 3b) Präfix abschneiden und Paket aus dem Buffer nehmen
                        let _ = buffer.split_to(2);
                        let msg_bytes = buffer.split_to(len);
                        log::debug!("[{}] Protobuf-Frame ({} Bytes) extrahiert", port_name, len);

                        // 4) Protobuf dekodieren (BytesMut → Bytes, weil Bytes implementiert Buf)
                        let bytes = msg_bytes.freeze();
                        match mesh_proto::FromRadio::decode(bytes) {
                            Ok(msg) => {
                                if let Some(variant) = msg.payload_variant {
                                    let event = match variant {
                                        PayloadVariant::Packet(p) => {
                                            log::info!(
                                                "[{}] Packet erhalten: from={} to={} hop_limit={}",
                                                port_name, p.from, p.to, p.hop_limit
                                            );
                                            let ty = if p.hop_limit > 0 {
                                                EventType::RelayedMesh { from: p.from, to: p.to }
                                            } else {
                                                EventType::DirectMesh  { from: p.from, to: p.to }
                                            };
                                            Event { port: port_name.clone(), event_type: ty }
                                        }
                                        PayloadVariant::NodeInfo(info) => {
                                            log::info!("[{}] Protobuf NodeInfo (node_id={})", port_name, info.num);
                                            Event {
                                                port:       port_name.clone(),
                                                event_type: EventType::NodeInfo { node_id: info.num },
                                            }
                                        }
                                        other => {
                                            log::debug!("[{}] Unbehandelter PayloadVariant::{:?}", port_name, other);
                                            Event {
                                                port:       port_name.clone(),
                                                event_type: EventType::Unknown,
                                            }
                                        }
                                    };
                                    let _ = tx.send(event);
                                } else {
                                    log::debug!("[{}] Nachricht ohne PayloadVariant erhalten", port_name);
                                }
                            }
                            Err(e) => {
                                log::warn!("[{}] Prost-Decode-Error: {:?}", port_name, e);
                            }
                        }
                    }
                }

                log::info!("[{}] Lese-Loop beendet, reconnect in 2s…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Öffnen fehlgeschlagen: {:?}, retry in 2s…", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}