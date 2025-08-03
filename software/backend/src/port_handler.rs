// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
// 
// src/port_handler.rs

use tokio_serial::{SerialPortBuilderExt, DataBits, Parity, StopBits, FlowControl};
use tokio::sync::mpsc::UnboundedSender;
use tokio::io::{AsyncReadExt, BufReader};
use prost::Message;
use std::time::Duration;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32    = 921600;
const MAX_FRAME: usize = 1024;  // Maximal plausibles Paket
const MIN_FRAME: usize =   1;   // Minimal plausibles Paket

pub async fn read_port(port_name: String, tx: UnboundedSender<Event>) {
    loop {
        log::info!("Öffne {} mit {} Baud (Meshtastic-Protobuf)…", port_name, BAUDRATE);
        let builder = tokio_serial::new(&port_name, BAUDRATE)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None);

        match builder.open_native_async() {
            Ok(port) => {
                log::info!("[{}] Port geöffnet, lese Meshtastic-Frames…", port_name);
                let mut reader = BufReader::new(port);

                loop {
                    // === 1) Header lesen (4 Bytes, Big-Endian framing) ===
                    let mut header = [0u8; 4];
                    if let Err(e) = reader.read_exact(&mut header).await {
                        log::warn!("[{}] EOF/I/O-Error beim Header-Lesen: {:?}", port_name, e);
                        break;
                    }
                    log::debug!(
                        "[{}] RAW HEADER: {:02X} {:02X} {:02X} {:02X}",
                        port_name, header[0], header[1], header[2], header[3]
                    );

                    // 1a) Marker prüfen
                    if header[0] != 0x94 || header[1] != 0xC3 {
                        log::warn!(
                            "[{}] Ungültiger Header-Marker {:02X}{:02X}, resync…",
                            port_name, header[0], header[1]
                        );
                        continue;
                    }

                    // 1b) Länge aus Big-Endian-Bytes extrahieren
                    let len = u16::from_be_bytes([header[2], header[3]]) as usize;
                    if !(MIN_FRAME..=MAX_FRAME).contains(&len) {
                        log::warn!(
                            "[{}] Ungültige Länge {} (außerhalb {}–{}), resync…",
                            port_name, len, MIN_FRAME, MAX_FRAME
                        );
                        continue;
                    }
                    log::debug!("[{}] Erkanntes Payload-Length: {} Bytes", port_name, len);

                    // === 2) Payload lesen ===
                    let mut payload = vec![0u8; len];
                    if let Err(e) = reader.read_exact(&mut payload).await {
                        log::warn!("[{}] I/O-Error beim Payload-Lesen: {:?}", port_name, e);
                        break;
                    }

                    // === 3) Protobuf dekodieren ===
                    match mesh_proto::FromRadio::decode(&*payload) {
                        Ok(msg) => {
                            if let Some(variant) = msg.payload_variant {
                                // 4) Event erzeugen und senden
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
                                        log::info!("[{}] NodeInfo: node_id={}", port_name, info.num);
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
                                log::debug!("[{}] Nachricht ohne PayloadVariant", port_name);
                            }
                        }
                        Err(e) => {
                            log::warn!("[{}] Prost-Decode-Error: {:?}", port_name, e);
                        }
                    }
                }

                log::info!("[{}] Lese-Loop beendet, reconnect in 2s…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Port-Öffnen fehlgeschlagen: {:?}, retry in 2s…", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
