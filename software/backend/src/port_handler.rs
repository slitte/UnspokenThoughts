// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs

use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio::io::{AsyncBufReadExt,  BufReader};
use tokio::time::{timeout, Duration};
use prost::Message;
use serde_json::Value;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32 = 921600;

// SLIP special bytes
const SLIP_END: u8     = 0xC0;
const SLIP_ESC: u8     = 0xDB;
const SLIP_ESC_END: u8 = 0xDC;
const SLIP_ESC_ESC: u8 = 0xDD;

/// Entfernt SLIP-Escape-Sequenzen und gibt das reine Payload-Bytearray zurück.
fn slip_unescape(input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    let mut i = 0;
    while i < input.len() {
        match input[i] {
            SLIP_ESC if i + 1 < input.len() => {
                match input[i + 1] {
                    SLIP_ESC_END => {
                        out.push(SLIP_END);
                        i += 2;
                    }
                    SLIP_ESC_ESC => {
                        out.push(SLIP_ESC);
                        i += 2;
                    }
                    _ => {
                        // Ungültige Escape-Sequenz: einfach das ESC-Byte mitnehmen
                        out.push(SLIP_ESC);
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Liest fortlaufend von der seriellen Schnittstelle, unframed SLIP-Pakete,
/// parst JSON- oder Protobuf-Frames und sendet entsprechende Events.
pub async fn read_port(port_name: String, tx: UnboundedSender<Event>) {
    loop {
        log::info!("Versuche Port \"{}\" mit {} Baud zu öffnen…", port_name, BAUDRATE);
        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(port) => {
                log::info!("[{}] Port geöffnet, starte SLIP-Framing…", port_name);
                let mut reader = BufReader::new(port);

                loop {
                    // 1) SLIP-Paket bis zum End-Marker einlesen, mit Timeout
                    log::debug!("[{}] Warten auf SLIP-Paket bis 0xC0…", port_name);
                    let mut slip_buf = Vec::new();
                    match timeout(Duration::from_secs(5), reader.read_until(SLIP_END, &mut slip_buf)).await {
                        Ok(Ok(0)) => {
                            log::warn!("[{}] EOF beim SLIP-Lesen", port_name);
                            break;
                        }
                        Ok(Ok(_n)) => {
                            log::debug!("[{}] SLIP read_until hat Bytes geliefert: {}", port_name, slip_buf.len());
                            // SLIP_END abschneiden, wenn vorhanden
                            if slip_buf.last() == Some(&SLIP_END) {
                                slip_buf.pop();
                            }
                        }
                        Ok(Err(e)) => {
                            log::warn!("[{}] I/O-Fehler im SLIP-Lesen: {:?}", port_name, e);
                            break;
                        }
                        Err(_) => {
                            log::warn!("[{}] SLIP-Lesen Timeout (5s), keine Daten", port_name);
                            continue;
                        }
                    }

                    if slip_buf.is_empty() {
                        // Keep-alive, kein Inhalt
                        continue;
                    }

                    // 2) SLIP-Unescape
                    let frame = slip_unescape(&slip_buf);

                    // 3) Entscheiden, ob JSON oder Protobuf
                    if frame.first() == Some(&b'{') {
                        // JSON-Zeile
                        match serde_json::from_slice::<Value>(&frame) {
                            Ok(val) => {
                                log::debug!("[{}] JSON empfangen: {}", port_name, val);
                                let event = Event {
                                    port:       port_name.clone(),
                                    event_type: EventType::NodeInfoJson(val),
                                };
                                let _ = tx.send(event);
                            }
                            Err(e) => {
                                log::warn!("[{}] Ungültiges JSON: {}", port_name, e);
                            }
                        }
                    } else {
                        // Protobuf-Frame
                        match mesh_proto::FromRadio::decode(&*frame) {
                            Ok(msg) => {
                                if let Some(variant) = msg.payload_variant {
                                    let event = match variant {
                                        PayloadVariant::Packet(p) => {
                                            log::info!(
                                                "[{}] Packet: from={} to={} hop_limit={}",
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
                                            log::info!(
                                                "[{}] Protobuf NodeInfo: node_id={}",
                                                port_name, info.num
                                            );
                                            Event {
                                                port:       port_name.clone(),
                                                event_type: EventType::NodeInfo { node_id: info.num },
                                            }
                                        }
                                        _ => {
                                            log::debug!("[{}] Unbehandelter PayloadVariant", port_name);
                                            Event { port: port_name.clone(), event_type: EventType::Unknown }
                                        }
                                    };
                                    let _ = tx.send(event);
                                } else {
                                    log::debug!("[{}] Protobuf ohne PayloadVariant", port_name);
                                }
                            }
                            Err(e) => {
                                log::warn!("[{}] Prost-Decode-Error: {:?}", port_name, e);
                            }
                        }
                    }
                }

                log::info!("[{}] Inner Loop beendet, reconnect in 2s…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Öffnen fehlgeschlagen: {:?}, retry in 2s…", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
