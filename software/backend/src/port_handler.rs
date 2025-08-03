// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs

use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc;
use prost::Message;

use tokio_util::codec::{FramedRead, LengthDelimitedCodec};
use futures::StreamExt;
use std::time::Duration;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32 = 921600;

pub async fn read_port(port_name: String, tx: mpsc::UnboundedSender<Event>) {
    loop {
        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(port) => {
                log::info!("Lausche auf {}", port_name);

                // ----------------------------
                // Hier bauen wir den Codec:
                let codec = LengthDelimitedCodec::builder()
                    .length_field_length(2)   // 2-Byte Längenfeld
                    .little_endian()          // little-endian
                    .new_codec();

                let (reader, _) = tokio::io::split(port);
                let mut lines = FramedRead::new(reader, codec);
                // ----------------------------

                while let Some(frame) = lines.next().await {
                    match frame {
                        Ok(bytes) => {
                            match mesh_proto::FromRadio::decode(bytes.as_ref()) {
                                Ok(msg) => {
                                    if let Some(variant) = msg.payload_variant {
                                        let event = match variant {
                                            PayloadVariant::Packet(packet) => {
                                                let from = packet.from;
                                                let to   = packet.to;
                                                let ty = if packet.hop_limit > 0 {
                                                    EventType::RelayedMesh { from, to }
                                                } else {
                                                    EventType::DirectMesh  { from, to }
                                                };
                                                Event { port: port_name.clone(), event_type: ty }
                                            }
                                            PayloadVariant::NodeInfo(info) => {
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
                                    log::warn!("[{}] Dekodierungsfehler: {:?}", port_name, e);
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("[{}] Framing-Fehler: {:?}", port_name, e);
                        }
                    }
                }

                log::info!("[{}] Stream beendet, warte auf Reconnect...", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Konnte Port nicht öffnen: {:?}", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}