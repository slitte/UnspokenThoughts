use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_stream::StreamExt;
use tokio::io::AsyncWriteExt;
use crate::message::{MeshMessage, PortMessage};
use crate::event::Event;
use tokio::sync::mpsc::UnboundedSender;

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
                    if let Err(e) = self.read_loop(port).await {
                        log::warn!("[{}] Lesefehler: {:?}", self.port_name, e);
                    }
                }
                Err(e) => {
                    log::warn!("[{}] Öffnen fehlgeschlagen: {:?}", self.port_name, e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                }
            }
        }
    }

    async fn read_loop(&mut self, port: SerialStream) -> tokio::io::Result<()> {
        let mut lines = FramedRead::new(port, LinesCodec::new());

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
                    return Err(e);
                }
            }
        }

        Ok(())
    }
}
