use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub fn start_server(
    bind: &str,
    port: u16,
    rpcuser: String,
    rpcpassword: String,
    channel: Arc<Mutex<(Sender<(String, Vec<String>)>, Receiver<String>)>>,
    sync_interval: u64,
) {
    // Spawn background sync thread
    let sync_channel = channel.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(sync_interval));
        let lock = sync_channel.lock().unwrap();
        let (tx, rx) = &*lock;
        let _ = tx.send(("sync".to_string(), vec![]));
        let _ = rx.recv();
    });

    let addr = format!("{}:{}", bind, port);
    let listener = match TcpListener::bind(&addr) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("Error: failed to bind RPC server to {}: {}", addr, e);
            if e.kind() == std::io::ErrorKind::AddrInUse {
                eprintln!("Hint: another instance may already be running. Check with `ss -tlnp | grep {}` or change rpcport in your config.", port);
            }
            std::process::exit(1);
        }
    };
    println!("RPC server listening on {}", addr);

    let expected_auth = format!(
        "Basic {}",
        base64::encode(format!("{}:{}", rpcuser, rpcpassword))
    );

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let channel = channel.clone();
                let expected_auth = expected_auth.clone();
                std::thread::spawn(move || {
                    handle_connection(stream, expected_auth, channel);
                });
            }
            Err(e) => {
                eprintln!("Connection error: {}", e);
            }
        }
    }
}

fn handle_connection(
    stream: TcpStream,
    expected_auth: String,
    channel: Arc<Mutex<(Sender<(String, Vec<String>)>, Receiver<String>)>>,
) {
    let mut reader = BufReader::new(stream.try_clone().expect("Failed to clone stream"));
    let writer = stream;

    // Read request line
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }

    // Read headers
    let mut auth_header: Option<String> = None;
    let mut content_length: usize = 0;

    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).is_err() {
            break;
        }
        let line_trimmed = line.trim();
        if line_trimmed.is_empty() {
            break;
        }
        // Case-insensitive header matching
        let lower = line_trimmed.to_lowercase();
        if lower.starts_with("authorization: ") {
            auth_header = Some(line_trimmed["authorization: ".len()..].to_string());
        } else if lower.starts_with("content-length: ") {
            content_length = line_trimmed["content-length: ".len()..]
                .parse()
                .unwrap_or(0);
        }
    }

    // Check auth
    if auth_header.as_deref() != Some(expected_auth.as_str()) {
        let response = "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"zecwallet\"\r\nContent-Length: 0\r\n\r\n";
        let _ = (&writer).write_all(response.as_bytes());
        return;
    }

    // Read body
    let mut body_bytes = vec![0u8; content_length];
    if reader.read_exact(&mut body_bytes).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&body_bytes);

    // Parse JSON-RPC 2.0
    let parsed = match json::parse(&body) {
        Ok(v) => v,
        Err(_) => {
            let response = "HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n";
            let _ = (&writer).write_all(response.as_bytes());
            return;
        }
    };

    let id = parsed["id"].dump();
    let method = parsed["method"].as_str().unwrap_or("").to_string();
    let params: Vec<String> = parsed["params"]
        .members()
        .map(|p| {
            if p.is_string() {
                p.as_str().unwrap().to_string()
            } else {
                p.dump()
            }
        })
        .collect();

    // Execute command via channel (mutex serializes wallet access)
    let result = {
        let lock = channel.lock().unwrap();
        let (tx, rx) = &*lock;
        let _ = tx.send((method.clone(), params));
        rx.recv().unwrap_or_else(|_| "Error: channel closed".to_string())
    };

    // Build JSON-RPC response body — try to inline result as JSON, else wrap as string
    let result_json = match json::parse(&result) {
        Ok(v) => v.dump(),
        Err(_) => json::stringify(result.as_str()),
    };
    let response_body = format!(r#"{{"jsonrpc":"2.0","result":{},"id":{}}}"#, result_json, id);

    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        response_body.len(),
        response_body
    );

    let _ = (&writer).write_all(response.as_bytes());
}
