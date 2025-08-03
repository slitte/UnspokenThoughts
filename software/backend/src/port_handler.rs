// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs

use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::codec::{FramedRead, LengthDelimitedCodec};
use futures::StreamExt;
use prost::Message;
use std::time::Duration;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32 = 921600;

pub async fn read_port(port_name: String, tx: UnboundedSender<Event>) {
    loop {
        log::debug!(" [{}] Outer loop start – ready to (re)open serial port", port_name);
        log::info!("Versuche Port \"{}\" mit {} Baud zu öffnen…", port_name, BAUDRATE);

        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(port) => {
                log::info!("[{}] Port geöffnet, konfiguriere LengthDelimitedCodec…", port_name);
                
                // WICHTIG: 2-Byte Little-Endian Präfix, kein Varint!
                let codec = LengthDelimitedCodec::builder()
                    .length_field_length(2)   // genau 2 Byte Länge
                    .little_endian()          // little-endian
                    .new_codec();
                log::debug!("[{}] Codec: 2-Byte LE Length-Delimited", port_name);

                let mut frames = FramedRead::new(port, codec);

                while let Some(frame_result) = frames.next().await {
                    log::debug!("[{}] Received Stream item", port_name);
                    match frame_result {
                        Ok(buf) => {
                            let len = buf.len();
                            log::debug!(
                                "[{}] ✔️ FramedRead delivered {} bytes (ohne Prefix)",
                                port_name, len
                            );
                            log::trace!(
                                "[{}] Frame bytes (hex, first up to 32B): {:02X?}",
                                port_name,
                                &buf[..std::cmp::min(len, 32)]
                            );

                            log::debug!("[{}] Decoding Protobuf message…", port_name);
                            match mesh_proto::FromRadio::decode(buf.freeze()) {
                                Ok(msg) => {
                                    log::debug!("[{}] Prost-Decode successful: {:?}", port_name, msg);
                                    if let Some(variant) = msg.payload_variant {
                                        log::debug!("[{}] PayloadVariant present: {:?}", port_name, variant);
                                        let event = match variant {
                                            PayloadVariant::Packet(p) => {
                                                log::info!(
                                                    "[{}] Packet erhalten: from={} to={} hop_limit={}",
                                                    port_name, p.from, p.to, p.hop_limit
                                                );
                                                let ty = if p.hop_limit > 0 {
                                                    log::debug!("[{}] Klassifiziere als RelayedMesh", port_name);
                                                    EventType::RelayedMesh { from: p.from, to: p.to }
                                                } else {
                                                    log::debug!("[{}] Klassifiziere als DirectMesh", port_name);
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
                                                log::warn!(
                                                    "[{}] Unbehandelter PayloadVariant::{:?}",
                                                    port_name, other
                                                );
                                                Event { port: port_name.clone(), event_type: EventType::Unknown }
                                            }
                                        };
                                        if let Err(e) = tx.send(event) {
                                            log::error!("[{}] Fehler beim Senden des Events: {:?}", port_name, e);
                                        } else {
                                            log::debug!("[{}] Event erfolgreich gesendet", port_name);
                                        }
                                    } else {
                                        log::debug!("[{}] Nachricht ohne PayloadVariant erhalten", port_name);
                                    }
                                }
                                Err(e) => {
                                    log::warn!("[{}] Prost-Decode-Error: {:?}", port_name, e);
                                }
                            }
                        }
                        Err(e) => {
                            log::warn!("[{}] Framing-Error im LengthDelimitedCodec: {:?}", port_name, e);
                            log::debug!("[{}] Warte 100 ms vor nächstem Versuch…", port_name);
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }

                log::info!("[{}]  FramedRead-Stream beendet, reconnect in 2 s…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Öffnen fehlgeschlagen: {:?}, retry in 2 s…", port_name, e);
                log::debug!("[{}] Sleeping 2 s before next open attempt", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
