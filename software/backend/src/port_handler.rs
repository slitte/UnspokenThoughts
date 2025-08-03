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

const BAUDRATE: u32      = 921600;
const MAX_FRAME: usize   = 1024;      // maximal plausibles Paket
const MIN_FRAME: usize   = 1;         // minimal plausibles Paket

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
                log::info!("[{}] Port geöffnet, lese Raw-Proto-Frames…", port_name);
                let mut reader = BufReader::new(port);

                loop {
                    // 1) Peek 2 bytes für die Länge
                    let mut header = [0u8; 2];
                    if let Err(e) = reader.read_exact(&mut header).await {
                        log::warn!("[{}] EOF oder I/O-Error beim Lesen des Headers: {:?}", port_name, e);
                        break;
                    }

                    // 2) Interpretiere Big-Endian zuerst
                    let mut len = u16::from_be_bytes(header) as usize;
                    // falls unplausibel, versuch Little-Endian
                    if len < MIN_FRAME || len > MAX_FRAME {
                        let len_le = u16::from_le_bytes(header) as usize;
                        if len_le >= MIN_FRAME && len_le <= MAX_FRAME {
                            log::debug!("[{}] Länge per BE invalid ({}), nutze LE={}", port_name, len, len_le);
                            len = len_le;
                        } else {
                            log::warn!(
                                "[{}] Ungültige Länge beider Endians: BE={} LE={} → resync 1 Byte",
                                port_name, len, len_le
                            );
                            // schiebe um 1 Byte weiter und versuche neu
                            // (hier: einfach weiter im Loop, das Header-Byte 1 haben wir schon verbraucht)
                            continue;
                        }
                    }

                    log::debug!("[{}] Framing-Länge erkannt: {} Bytes", port_name, len);

                    // 3) Lese genau `len` Payload-Bytes
                    let mut payload = vec![0u8; len];
                    if let Err(e) = reader.read_exact(&mut payload).await {
                        log::warn!("[{}] I/O-Error beim Lesen des Payloads: {:?}", port_name, e);
                        break;
                    }

                    // 4) Prost-Decode
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

                log::info!("[{}] Loop beendet, reconnect in 2s…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Öffnen fehlgeschlagen: {:?}, retry in 2s…", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}