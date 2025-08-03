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

const BAUDRATE: u32   = 921600;
const START1:  u8     = 0x94;
const START2:  u8     = 0xC3;
const MAX_PAYLOAD: usize = 512;

pub async fn read_port(port_name: String, tx: UnboundedSender<Event>) {
    loop {
        log::info!("Versuche {}, {} Baud (8N1, no flow)…", port_name, BAUDRATE);
        let builder = tokio_serial::new(&port_name, BAUDRATE)
            .data_bits(DataBits::Eight)
            .parity(Parity::None)
            .stop_bits(StopBits::One)
            .flow_control(FlowControl::None);

        match builder.open_native_async() {
            Ok(port) => {
                log::info!("[{}] Port geöffnet, starte Streaming-Protokoll…", port_name);
                let mut reader = BufReader::new(port);

                'inner: loop {
                    // ——— 1) Suche START1 ———
                    let mut byte = [0u8; 1];
                    loop {
                        if let Err(e) = reader.read_exact(&mut byte).await {
                            log::warn!("[{}] I/O beim Lesen: {:?}", port_name, e);
                            break 'inner;
                        }
                        if byte[0] == START1 {
                            break;
                        } else {
                            // Debug-Text
                            log::debug!("[{}] Debug-Byte: 0x{:02X}", port_name, byte[0]);
                        }
                    }

                    // ——— 2) Prüfe START2 ———
                    if let Err(e) = reader.read_exact(&mut byte).await {
                        log::warn!("[{}] I/O beim Lesen von START2: {:?}", port_name, e);
                        break 'inner;
                    }
                    if byte[0] != START2 {
                        log::warn!(
                            "[{}] Ungültiges START2: 0x{:02X}, resync…",
                            port_name,
                            byte[0]
                        );
                        continue 'inner;
                    }

                    // ——— 3) Lese 2-Byte Länge (Big-Endian) ———
                    let mut len_bytes = [0u8; 2];
                    if let Err(e) = reader.read_exact(&mut len_bytes).await {
                        log::warn!("[{}] I/O beim Lesen der Länge: {:?}", port_name, e);
                        break 'inner;
                    }
                    let len = u16::from_be_bytes(len_bytes) as usize;
                    if len == 0 || len > MAX_PAYLOAD {
                        log::warn!(
                            "[{}] Unplausible Länge {} (max {}), resync…",
                            port_name,
                            len,
                            MAX_PAYLOAD
                        );
                        continue 'inner;
                    }
                    log::debug!("[{}] Framing-Länge: {} Bytes", port_name, len);

                    // ——— 4) Lese Payload ———
                    let mut payload = vec![0u8; len];
                    if let Err(e) = reader.read_exact(&mut payload).await {
                        log::warn!("[{}] I/O beim Lesen des Payloads: {:?}", port_name, e);
                        break 'inner;
                    }

                    // ——— 5) Protobuf dekodieren ———
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

                log::info!("[{}] Inner Loop beendet, reconnect in 2 Sekunden…", port_name);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Öffnen fehlgeschlagen: {:?}, retry in 2 Sekunden…", port_name, e);
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
