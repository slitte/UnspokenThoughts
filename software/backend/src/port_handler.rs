use std::sync::Arc;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{mpsc::UnboundedSender, Mutex};
use tokio::time::{sleep, Duration};
use tokio_stream::StreamExt;
use tokio_serial::{SerialPortBuilderExt};
use tokio_util::codec::{FramedRead, LinesCodec};

use crate::event::Event;
use crate::message::{MeshMessage, PortMessage};

pub struct PortHandler {
    port_name: String,
    sender: UnboundedSender<Event>,
}

impl PortHandler {
    pub fn new(port_name: String, sender: UnboundedSender<Event>) -> Self {
        Self { port_name, sender }
    }

    pub async fn run(&mut self) {
        loop {
            match tokio_serial::new(&self.port_name, 921600)
                .open_native_async()
            {
                Ok(port) => {
                    log::info!("[{}] Verbunden", self.port_name);

                    let (reader, writer) = tokio::io::split(port);
                    let framed = FramedRead::new(reader, LinesCodec::new());
                    let writer = Arc::new(Mutex::new(BufWriter::new(writer)));

                    // NodeInfo-Task starten
                    let writer_clone = Arc::clone(&writer);
                    let port_name_clone = self.port_name.clone();
                    // tokio::spawn(async move {
                    //     let mut interval = tokio::time::interval(Duration::from_secs(600));
                    //     loop {
                    //         interval.tick().await;
                    //         let cmd = b"{\"request\": \"node_info\"}\n";

                    //         let mut w = writer_clone.lock().await;
                    //         if let Err(e) = w.write_all(cmd).await {
                    //             log::warn!("[{}] Fehler beim Schreiben: {:?}", port_name_clone, e);
                    //             break;
                    //         }
                    //         if let Err(e) = w.flush().await {
                    //             log::warn!("[{}] Fehler beim Flush: {:?}", port_name_clone, e);
                    //             break;
                    //         }

                    //         log::info!("[{}] NodeInfo angefragt", port_name_clone);
                    //     }
                    // });

                    // Lesen starten
                    if let Err(e) = self.read_loop(framed).await {
                        log::warn!("[{}] Lesefehler: {:?}", self.port_name, e);
                    }
                }
                Err(e) => {
                    log::warn!("[{}] Öffnen fehlgeschlagen: {:?}", self.port_name, e);
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    async fn read_loop<S>(&mut self, mut lines: FramedRead<S, LinesCodec>) -> tokio::io::Result<()>
    where
        S: tokio::io::AsyncRead + Unpin,
    {
        while let Some(line) = lines.next().await {
            match line {
                Ok(text) => {
                    if text.trim().is_empty() {
                        continue;
                    }
                    let parsed = serde_json::from_str::<MeshMessage>(&text).ok();
                    let msg = PortMessage {
                        port: self.port_name.clone(),
                        raw: text,
                        parsed,
                    };
                    let _ = self.sender.send(Event::MeshMessage(msg));
                }
                    Err(e) => {
        let _ = self.sender.send(Event::Error(format!(
            "[{}] Lese- oder Decodefehler: {:?}",
            self.port_name, e
        )));
        return Err(std::io::Error::new(std::io::ErrorKind::Other, e));
    }

            }
        }

        Ok(())
    }
}
