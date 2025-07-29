use std::time::Duration;
use crossbeam_channel::{unbounded, Sender};
use tokio::time::interval;
use tokio::task;
use std::io::BufRead;
use std::thread;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc as StdArc;
use ctrlc;


const PORTS: [&str; 8] = [
    "/dev/ttyUSB0", "/dev/ttyUSB1", "/dev/ttyUSB2", "/dev/ttyUSB3",
    "/dev/ttyUSB4", "/dev/ttyUSB5", "/dev/ttyUSB6", "/dev/ttyUSB7",
];

#[derive(Debug, serde::Deserialize)]
struct MeshMessage {
    from: Option<String>,
    to: Option<String>,
    text: Option<String>,
}

#[derive(Debug)]
struct PortMessage {
    port: String,
    raw: String,
    parsed: Option<MeshMessage>,
}

#[tokio::main]
async fn main() {
    let (tx, rx) = unbounded::<PortMessage>();

    // Für Shutdown Signal
    let running = StdArc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("Ctrl-C: Beende...");
        r.store(false, Ordering::SeqCst);
    }).expect("Fehler beim Setzen des Ctrl-C-Handlers");

    // Spawn Listener für Ports mit Reconnect-Logik
    for port_name in PORTS {
        let tx = tx.clone();
        let running = running.clone();
        let port = port_name.to_string();
        thread::spawn(move || listen_with_reconnect(port, tx, running));
    }

    // Empfangene Nachrichten verarbeiten
    let rx_task = task::spawn_blocking(move || {
        while let Ok(msg) = rx.recv() {
            println!("[Sternschnuppe] Von {}: {}", msg.port, msg.raw);
            if let Some(parsed) = msg.parsed {
                println!("Parsed: {:?}", parsed);
            }
            // hier Trigger an Visualisierung weitergeben
        }
    });

    // NodeInfo Task
    let running2 = running.clone();
    let node_ports = PORTS.map(|p| p.to_string());
    let nodeinfo_task = task::spawn(async move {
        let mut interval = interval(Duration::from_secs(600));
        while running2.load(Ordering::SeqCst) {
            interval.tick().await;
            for port in &node_ports {
                match serialport::new(port, 921600)
                    .timeout(Duration::from_secs(2))
                    .open()
                {
                    Ok(mut serial) => {
                        if let Err(e) = serial.write(b"{\"request\": \"node_info\"}\n") {
                            eprintln!("[NodeInfo Anfrage] Fehler beim Schreiben an {}: {:?}", port, e);
                        } else {
                            println!("[NodeInfo Anfrage] gesendet an {}", port);
                        }
                    }
                    Err(e) => {
                        eprintln!("[NodeInfo Anfrage] Port {} nicht offen: {:?}", port, e);
                    }
                }
            }
        }
        println!("NodeInfo Task beendet");
    });

    // Warten bis Signal zum Beenden kommt
    while running.load(Ordering::SeqCst) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    println!("Haupt-Task beendet. Warte auf Neben-Tasks...");
    rx_task.abort();
    nodeinfo_task.abort();
}

fn listen_with_reconnect(port_name: String, sender: Sender<PortMessage>, running: StdArc<AtomicBool>) {
    while running.load(Ordering::SeqCst) {
        match serialport::new(&port_name, 921600)
            .timeout(Duration::from_secs(1))
            .open()
        {
            Ok(port) => {
                println!("[{}] Verbunden", port_name);
                let mut reader = std::io::BufReader::new(port);
                loop {
                    let mut buffer = String::new();
                    match reader.read_line(&mut buffer) {
                        Ok(n) if n > 0 => {
                            let text = buffer.trim().to_string();
                            if !text.is_empty() {
                                // Versuch zu parsen (optional)
                                let parsed = serde_json::from_str::<MeshMessage>(&text).ok();
                                let msg = PortMessage {
                                    port: port_name.clone(),
                                    raw: text,
                                    parsed,
                                };
                                if let Err(e) = sender.send(msg) {
                                    eprintln!("[{}] Fehler beim Senden in Channel: {:?}", port_name, e);
                                }
                            }
                        }
                        Ok(_) => {} // Leerzeile oder Timeout
                        Err(e) => {
                            eprintln!("[{}] Fehler beim Lesen: {:?}", port_name, e);
                            break; // Bei Fehler: Port neu öffnen
                        }
                    }
                    if !running.load(Ordering::SeqCst) {
                        break;
                    }
                }
                println!("[{}] Verbindung verloren, versuche erneut...", port_name);
            }
            Err(e) => {
                eprintln!("[{}] Kann Port nicht öffnen: {:?}", port_name, e);
                // Warte etwas vor neuem Versuch
                thread::sleep(Duration::from_secs(2));
            }
        }
        if !running.load(Ordering::SeqCst) {
            break;
        }
    }
    println!("[{}] Listener beendet", port_name);
}
