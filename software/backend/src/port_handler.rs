use tokio_serial::{SerialPortBuilderExt, SerialStream};
use tokio_util::codec::{FramedRead, LinesCodec};
use tokio_stream::StreamExt;
use tokio::io::{AsyncWriteExt, BufWriter};
use crate::message::{MeshMessage, PortMessage};
use crate::event::Event;
use tokio::sync::mpsc::UnboundedSender;
use tokio::time::{sleep, Duration};

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

                    // Aufteilen in Reader + Writer
                    let (reader, writer) = tokio::io::split(port);
                    let framed = FramedRead::new(reader, LinesCodec::new());
                    let mut writer = BufWriter::new(writer);

                    // Task für NodeInfo-Anfragen starten
                    let mut writer_clone = writer.clone();
                    let port_name_clone = self.port_name.clone();
                    tokio::spawn(async move {
                        let mut interval = tokio::time::interval(Duration::from_secs(600));
                        loop {
                            interval.tick().await;
                            let cmd = b"{\"request\": \"node_info\"}\n";
                            match writer_clone.write_all(cmd).await {
                                Ok(_) => log::info!("[{}] NodeInfo angefragt", port_name_clone),
                                Err(e) => log::warn!("[{}] Fehler bei NodeInfo-Schreiben: {:?}", port_name_clone, e),
                            }
                            if writer_clone.flush().await.is_err() {
                                break;
                            }
                        }
                    });

                    // Hauptleseschleife
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
                    return Err(e);
                }
            }
        }

        Ok(())
    }
}
