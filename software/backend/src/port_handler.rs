// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs

// src/port_handler.rs

// MPL-Header unverändert…

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
        log::info!("Öffne {} mit {} Baud (Raw-Proto)…", port_name, BAUDRATE);
        let builder = tokio_serial::new(&port_name, BAUDRATE)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None);

        match builder.open_native_async() {
            Ok(port) => {
                log::info!("[{}] Port geöffnet, lese TLV-Frames…", port_name);
                let mut reader = BufReader::new(port);

                loop {
                    // === 1) Lese 2-Byte-Prefix (Little-Endian) ===
                    let mut prefix = [0u8; 2];
                    if let Err(e) = reader.read_exact(&mut prefix).await {
                        log::warn!("[{}] EOF oder I/O-Error beim Lesen des Prefix: {:?}", port_name, e);
                        break;
                    }
                    // Länge interpretieren
                    let len = u16::from_le_bytes(prefix) as usize;
                    if !(MIN_FRAME..=MAX_FRAME).contains(&len) {
                        log::warn!(
                            "[{}] Ungültige Länge {} (außerhalb {}–{}), resync…",
                            port_name, len, MIN_FRAME, MAX_FRAME
                        );
                        // Wir schieben hier nicht extra Byte-weise zurück—
                        // wir versuchen einfach, neu zu lesen.
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
                log::warn!("[{}] Öffnen fehlgeschlagen: {:?}, retry in 2s…", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
