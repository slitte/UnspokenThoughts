// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
//
// src/port_handler.rs

use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc::UnboundedSender;
use tokio_util::codec::{Decoder, FramedRead, LengthDelimitedCodec};
use futures::StreamExt;
use prost::Message;
use bytes::BytesMut;
use serde_json::Value;
use std::io;
use std::time::Duration;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32 = 921600;

/// Ein Stream-Item: Entweder ein JSON-Value oder rohes Protobuf-Frame
enum Mixed {
    Json(Value),
    Proto(BytesMut),
}

/// Kombinierter Decoder: fängt JSON-Lines ab, ansonsten Protobuf-Frames mit 2-Byte BE-Präfix
struct JsonThenProto {
    inner: LengthDelimitedCodec,
}

impl JsonThenProto {
    fn new() -> Self {
        let inner = LengthDelimitedCodec::builder()
            .length_field_length(2)   // 2-Byte Präfix
            .little_endian()             // Big-Endian
            .new_codec();
        JsonThenProto { inner }
    }
}

impl Decoder for JsonThenProto {
    type Item  = Mixed;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, io::Error> {
        // 1) JSON abfangen: wenn Buffer mit '{' beginnt
        if src.first().copied() == Some(b'{') {
            if let Some(pos) = src.iter().position(|&b| b == b'\n') {
                let line = src.split_to(pos + 1);
                match serde_json::from_slice::<Value>(&line) {
                    Ok(json) => return Ok(Some(Mixed::Json(json))),
                    Err(e) => {
                        log::warn!("Ungültiges JSON im Stream: {} – überspringe", e);
                        return Ok(None);
                    }
                }
            }
            return Ok(None);
        }

        // 2) sonst Protobuf-Frame
        if let Some(frame) = self.inner.decode(src)? {
            return Ok(Some(Mixed::Proto(frame)));
        }
        Ok(None)
    }
}

/// Liest kontinuierlich von der seriellen Schnittstelle, parst JSON- und Protobuf-Nachrichten
/// und sendet entsprechende Events über `tx`.
pub async fn read_port(port_name: String, tx: UnboundedSender<Event>) {
    loop {
        log::info!("Versuche Port \"{}\" mit {} Baud zu öffnen…", port_name, BAUDRATE);
        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(port) => {
                log::info!("[{}] Port geöffnet, starte FramedRead…", port_name);
                let mut frames = FramedRead::new(port, JsonThenProto::new());

                while let Some(item) = frames.next().await {
                    match item {
                        Ok(Mixed::Json(val)) => {
                            // JSON als EventType::NodeInfoJson
                            log::debug!("[{}] JSON empfangen: {}", port_name, val);
                            let event = Event {
                                port:       port_name.clone(),
                                event_type: EventType::NodeInfoJson(val),
                            };
                            let _ = tx.send(event);
                        }
                        Ok(Mixed::Proto( buf)) => {
                            log::debug!("[{}] Protobuf-Frame erhalten: {} Bytes", port_name, buf.len());
                            match mesh_proto::FromRadio::decode(buf.freeze()) {
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
                        Err(e) => {
                            log::warn!("[{}] Stream-Error: {:?}", port_name, e);
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }

                log::info!("[{}] Stream beendet, reconnect in 2s…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Öffnen fehlgeschlagen: {:?}, retry in 2s…", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
