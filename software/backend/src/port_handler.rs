// src/port_handler.rs

use bytes::{BytesMut, Buf};
use tokio::io::AsyncReadExt;
use tokio_serial::SerialPortBuilderExt;
use tokio::sync::mpsc;
use prost::Message;
use crate::mesh_proto;
use crate::Event;
use crate::event::EventType;                       // ← neu
use crate::mesh_proto::from_radio::PayloadVariant; // ← neu

const BAUDRATE: u32 = 921600;

pub async fn read_port(port_name: String, tx: mpsc::UnboundedSender<Event>) {
    loop {
        match tokio_serial::new(&port_name, BAUDRATE).open_native_async() {
            Ok(mut port) => {
                let mut buffer = BytesMut::with_capacity(4096);
                log::info!("Lausche auf {}", port_name);

                loop {
                    let mut temp = [0u8; 512];
                    let n = match port.read(&mut temp).await {
                        Ok(0) => { log::warn!("[{}] Port geschlossen", port_name); break; }
                        Ok(n) => n,
                        Err(e) => { log::warn!("[{}] Lesefehler: {:?}", port_name, e); break; }
                    };
                    buffer.extend_from_slice(&temp[..n]);

                    while buffer.len() > 2 {
                        let len = u16::from_le_bytes([buffer[0], buffer[1]]) as usize;
                        if buffer.len() < 2 + len { break; }
                        let msg_bytes = &buffer[2..2+len];

                        match mesh_proto::FromRadio::decode(msg_bytes) {
                            Ok(msg) => {
                                if let Some(variant) = msg.payload_variant {
                                    let event = match variant {
                                        PayloadVariant::Packet(packet) => {
                                            let from = packet.from;
                                            let to   = packet.to;
                                            if packet.hop_limit > 0 {
                                                Event { port: port_name.clone(),
                                                        event_type: EventType::RelayedMesh { from, to } }
                                            } else {
                                                Event { port: port_name.clone(),
                                                        event_type: EventType::DirectMesh  { from, to } }
                                            }
                                        }
                                        PayloadVariant::NodeInfo(info) => {
                                            // Annahme: NodeInfo hat Feld `node_id: u32`
                                            Event { port: port_name.clone(),
                                                    event_type: EventType::NodeInfo { node_id: info.num } }
                                        }
                                        _ => {
                                            log::debug!("[{}] Unbekannter Payload-Typ, übersprungen", port_name);
                                            Event { port: port_name.clone(),
                                                    event_type: EventType::Unknown }
                                        }
                                    };
                                    let _ = tx.send(event);
                                }
                            }
                            Err(e) => {
                                log::warn!("[{}] Dekodierungsfehler: {:?}", port_name, e);
                            }
                        }
                        buffer.advance(2 + len);
                    }
                }
                log::info!("[{}] Warte auf Reconnect...", port_name);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            Err(e) => {
                log::warn!("[{}] Konnte Port nicht öffnen: {:?}", port_name, e);
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}
