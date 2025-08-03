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
use bytes::Buf;
use serde_json::Value;
use std::io;
use std::time::Duration;

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32 = 921600;
/// Maximale plausible Protobuf-Payload-Größe in Bytes (einstellbar)
const MAX_PAYLOAD: usize = 1024;

/// Entweder eine JSON-Zeile oder ein rohes Protobuf-Frame
enum Mixed {
    Json(Value),
    Proto(BytesMut),
}

/// Decoder, der zuerst JSON-Lines abfängt, sonst 2-Byte LE Protobuf-Frames
struct JsonThenProto {
    inner: LengthDelimitedCodec,
}

impl JsonThenProto {
    fn new() -> Self {
        let inner = LengthDelimitedCodec::builder()
            .length_field_length(2)   // genau 2 Byte Präfix
            .little_endian()          // Little-Endian wie euer Gerät
            .new_codec();
        JsonThenProto { inner }
    }
}

impl Decoder for JsonThenProto {
    type Item  = Mixed;
    type Error = io::Error;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, io::Error> {
        // --- 1) JSON abfangen ---
        if src.first().copied() == Some(b'{') {
            if let Some(pos) = src.iter().position(|&b| b == b'\n') {
                let line = src.split_to(pos+1);
                // immer ablösen, auch bei Fehlern
                match serde_json::from_slice::<Value>(&line) {
                    Ok(json)  => return Ok(Some(Mixed::Json(json))),
                    Err(e)    => {
                        log::warn!("Ungültiges JSON im Stream: {} – verwerfe Zeile", e);
                        // JSON-Zeile weg, dann neu versuchen
                        return Ok(None);
                    }
                }
            }
            // kein kompletter JSON-Line da → nix machen
            return Ok(None);
        }

        // --- 2) Prüfe auf plausibles 2-Byte-Prefix ---
        if src.len() >= 2 {
            let prefix = [src[0], src[1]];
            let len = u16::from_le_bytes(prefix) as usize;
            log::trace!("Candidate Prefix: {:02X?} → len={}", prefix, len);
            if len == 0 || len > MAX_PAYLOAD {
                // offensichtlich Müll im Buffer, einen Byte skippen und resync
                log::warn!("Unplausibles Prefix {:?} (len={}), resync +1 byte", prefix, len);
                src.advance(1);
                return Ok(None);
            }
        }

        // --- 3) Protobuf-Frame via inner decoder ---
        if let Some(frame) = self.inner.decode(src)? {
            return Ok(Some(Mixed::Proto(frame)));
        }

        Ok(None)
    }
}

/// Liest fortlaufend von der seriellen Schnittstelle, parst JSON und Protobuf
/// und sendet eure Events über `tx`.
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
