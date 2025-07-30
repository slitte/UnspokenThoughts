use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

pub async fn start_tcp_server(clients: Arc<Mutex<Vec<TcpStream>>>) {
    let listener = TcpListener::bind("127.0.0.1:9000").await
        .expect("TCP-Server konnte nicht starten");
    log::info!("[TCP] Lausche auf 127.0.0.1:9000");

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                log::info!("[TCP] Neuer Client: {}", addr);
                stream.set_nodelay(true).ok();
                clients.lock().await.push(stream);
            }
            Err(e) => log::warn!("[TCP] Verbindungsfehler: {:?}", e),
        }
    }
}
