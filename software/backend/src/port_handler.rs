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
        log::debug!("🔄 [{}] Outer loop: open serial port…", port_name);
        log::info!("Versuche Port \"{}\" mit {} Baud zu öffnen…", port_name, BAUDRATE);

        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(mut port) => {
                log::info!("[{}] Port geöffnet, starte Lese-Loop", port_name);

                loop {
                    // --- 1) Bytes vom Port lesen ---
                    let mut tmp = [0u8; 512];
                    log::debug!("[{}] Vor read(): buffer.len() = {}", port_name, buffer.len());
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
                    log::debug!(
                        "[{}] Nach extend(): buffer.len() = {} (max {}B gepuffert)",
                        port_name,
                        buffer.len(),
                        buffer.capacity()
                    );
                    log::trace!(
                        "[{}] Buffer (hex, up to 64B): {:02X?}",
                        port_name,
                        &buffer[..std::cmp::min(buffer.len(), 64)]
                    );

                    // --- 2) JSON-NodeInfo abfangen (ganze Zeile “{…}\n”) ---
                    while buffer.first().copied() == Some(b'{') {
                        if let Some(pos) = buffer.iter().position(|&b| b == b'\n') {
                            let line = buffer.split_to(pos + 1);
                            let txt = match std::str::from_utf8(&line) {
                                Ok(s) => s.trim_end(),
                                Err(_) => {
                                    log::warn!("[{}] Ungültige UTF-8 in JSON-Zeile", port_name);
                                    continue;
                                }
                            };
                            log::debug!("[{}] JSON-Zeile empfangen: {}", port_name, txt);
                            match serde_json::from_str::<Value>(txt) {
                                Ok(json) => {
                                    let node_id = json.get("num")
                                        .and_then(Value::as_u64)
                                        .unwrap_or(0) as u32;
                                    log::info!("[{}] JSON-NodeInfo: num={}", port_name, node_id);
                                    let event = Event {
                                        port: port_name.clone(),
                                        event_type: EventType::NodeInfo { node_id },
                                    };
                                    if tx.send(event).is_err() {
                                        log::error!("[{}] Fehler beim Senden des JSON-Event", port_name);
                                    }
                                }
                                Err(e) => {
                                    log::warn!("[{}] Ungültiges JSON: {} – {}", port_name, txt, e);
                                }
                            }
                            log::debug!("[{}] Noch {} Bytes im Buffer nach JSON-Split", port_name, buffer.len());
                            continue;
                        }
                        break;
                    }

                    // --- 3) Protobuf-Frames entpacken ---
                    loop {
                        if buffer.len() < 2 {
                            log::debug!(
                                "[{}] Zu wenig Daten für Prefix ({} Bytes, brauche 2)",
                                port_name,
                                buffer.len()
                            );
                            break;
                        }
                        // Prefix lesen, aber nicht abschneiden
                        let prefix_bytes = [buffer[0], buffer[1]];
                        let len = u16::from_be_bytes(prefix_bytes) as usize; // BE- statt LE-Endian
                        log::debug!(
                            "[{}] Gefundenes Prefix: bytes={:02X?}, len={}",
                            port_name,
                            prefix_bytes,
                            len
                        );
                        if buffer.len() < 2 + len {
                            log::debug!(
                                "[{}] Unvollständiges Frame: have {} Bytes, need {} + 2 Prefix",
                                port_name,
                                buffer.len(),
                                len
                            );
                            break;
                        }
                        // Jetzt wirklich zuschneiden
                        let _ = buffer.split_to(2);           // Prefix entfernen
                        let msg_bytes = buffer.split_to(len); // Payload herausziehen
                        log::debug!(
                            "[{}] Extrahiertes Protobuf-Frame: {} Bytes",
                            port_name,
                            msg_bytes.len()
                        );
                        log::trace!(
                            "[{}] Frame-Bytes (hex, up to 32B): {:02X?}",
                            port_name,
                            &msg_bytes[..std::cmp::min(msg_bytes.len(), 32)]
                        );

                        // --- 4) Protobuf dekodieren ---
                        log::debug!("[{}] Decoding Protobuf message…", port_name);
                        match mesh_proto::FromRadio::decode(msg_bytes.freeze()) {
                            Ok(msg) => {
                                log::debug!("[{}] Prost-Decode erfolgreich: {:?}", port_name, msg);
                                if let Some(variant) = msg.payload_variant {
                                    log::debug!("[{}] PayloadVariant: {:?}", port_name, variant);
                                    let event = match variant {
                                        PayloadVariant::Packet(p) => {
                                            log::info!(
                                                "[{}] Packet: from={} to={} hop_limit={}",
                                                port_name, p.from, p.to, p.hop_limit
                                            );
                                            let ty = if p.hop_limit > 0 {
                                                log::debug!("[{}] -> RelayedMesh", port_name);
                                                EventType::RelayedMesh { from: p.from, to: p.to }
                                            } else {
                                                log::debug!("[{}] -> DirectMesh", port_name);
                                                EventType::DirectMesh { from: p.from, to: p.to }
                                            };
                                            Event { port: port_name.clone(), event_type: ty }
                                        }
                                        PayloadVariant::NodeInfo(info) => {
                                            log::info!("[{}] Protobuf NodeInfo: node_id={}", port_name, info.num);
                                            Event {
                                                port:       port_name.clone(),
                                                event_type: EventType::NodeInfo { node_id: info.num },
                                            }
                                        }
                                        other => {
                                            log::warn!("[{}] Unbehandelter Variant::{:?}", port_name, other);
                                            Event { port: port_name.clone(), event_type: EventType::Unknown }
                                        }
                                    };
                                    if tx.send(event).is_err() {
                                        log::error!("[{}] Fehler beim Senden des Protobuf-Event", port_name);
                                    }
                                } else {
                                    log::debug!("[{}] Nachricht ohne PayloadVariant", port_name);
                                }
                            }
                            Err(e) => {
                                log::warn!("[{}] Prost-Decode-Error: {:?}", port_name, e);
                            }
                        }

                        log::debug!("[{}] Verbleibende Bytes im Buffer: {}", port_name, buffer.len());
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
