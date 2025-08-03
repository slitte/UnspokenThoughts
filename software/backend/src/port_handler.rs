// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/
// 
// src/port_handler.rs

use tokio_serial::{SerialPortBuilderExt, DataBits, Parity, StopBits, FlowControl};
use tokio::sync::mpsc::UnboundedSender;
use tokio::io::{AsyncRead, AsyncReadExt, BufReader};
use prost::Message;
use std::{io, time::Duration};

use crate::{Event, mesh_proto};
use crate::event::EventType;
use crate::mesh_proto::from_radio::PayloadVariant;

const BAUDRATE: u32    = 921600;
const MAX_FRAME: usize = 1024;  // Maximal plausibles Paket
const MIN_FRAME: usize =   1;   // Minimal plausibles Paket

/// Synchronisiert bis zum nächsten 0x94C3-Header.
/// Liest byte-weise und rückt das 2-Byte-Fenster immer um eins weiter.
async fn sync_to_header<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    port_name: &str
) -> io::Result<()> {
    let mut window = [0u8; 2];
    // Erst mal zwei Bytes einlesen
    reader.read_exact(&mut window).await?;
    // Schiebe so lange, bis wir [0x94, 0xC3] haben
    while window != [0x94, 0xC3] {
        // neues Byte ans Ende holen
        window[0] = window[1];
        reader.read_exact(&mut window[1..2]).await?;
    }
    log::debug!(
        "[{}] Header-Marker synchronisiert: {:02X}{:02X}",
        port_name,
        window[0],
        window[1]
    );
    Ok(())
}

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
                    // === 1) Sync auf den 0x94C3-Header ===
                    if let Err(e) = sync_to_header(&mut reader, &port_name).await {
                        log::warn!("[{}] Fehler beim Header-Sync: {:?}", port_name, e);
                        break;
                    }

                    // === 2) Länge aus den nächsten 2 Bytes (Big-Endian) ===
                    let mut len_bytes = [0u8; 2];
                    if let Err(e) = reader.read_exact(&mut len_bytes).await {
                        log::warn!("[{}] EOF/I/O-Error beim Length-Read: {:?}", port_name, e);
                        break;
                    }
                    let len = u16::from_be_bytes(len_bytes) as usize;
                    log::debug!(
                        "[{}] Gelesene Payload-Länge: {} (Bytes {:02X}{:02X})",
                        port_name,
                        len,
                        len_bytes[0],
                        len_bytes[1]
                    );
                    if !(MIN_FRAME..=MAX_FRAME).contains(&len) {
                        log::warn!(
                            "[{}] Ungültige Länge {} (außerhalb {}–{}), resync…",
                            port_name,
                            len,
                            MIN_FRAME,
                            MAX_FRAME
                        );
                        continue;
                    }

                    // === 3) Payload lesen ===
                    let mut payload = vec![0u8; len];
                    if let Err(e) = reader.read_exact(&mut payload).await {
                        log::warn!("[{}] I/O-Error beim Payload-Lesen: {:?}", port_name, e);
                        break;
                    }

                    // === 4) Protobuf dekodieren ===
                    match mesh_proto::FromRadio::decode(&*payload) {
                        Ok(msg) => {
                            if let Some(variant) = msg.payload_variant {
                                // === 5) Event erzeugen und senden ===
                                let event = match variant {
                                    PayloadVariant::Packet(p) => {
                                        log::info!(
                                            "[{}] Packet: from={} to={} hop_limit={}",
                                            port_name,
                                            p.from,
                                            p.to,
                                            p.hop_limit
                                        );
                                        let ty = if p.hop_limit > 0 {
                                            EventType::RelayedMesh { from: p.from, to: p.to }
                                        } else {
                                            EventType::DirectMesh { from: p.from, to: p.to }
                                        };
                                        Event { port: port_name.clone(), event_type: ty }
                                    }
                                    PayloadVariant::NodeInfo(info) => {
                                        log::info!(
                                            "[{}] NodeInfo: node_id={}",
                                            port_name,
                                            info.num
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
                log::warn!(
                    "[{}] Port-Öffnen fehlgeschlagen: {:?}, retry in 2s…",
                    port_name,
                    e
                );
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
