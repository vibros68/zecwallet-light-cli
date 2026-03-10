use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;

pub fn call_server(
    rpcbind: &str,
    rpcport: u16,
    rpcuser: &str,
    rpcpassword: &str,
    command: &str,
    params: Vec<String>,
) -> Result<String, String> {
    let addr = format!("{}:{}", rpcbind, rpcport);
    let mut stream = TcpStream::connect(&addr)
        .map_err(|e| format!("Failed to connect to server at {}: {}", addr, e))?;

    // Build JSON-RPC body
    let params_json: Vec<String> = params
        .iter()
        .map(|p| json::stringify(p.as_str()))
        .collect();
    let body = format!(
        r#"{{"jsonrpc":"2.0","method":"{}","params":[{}],"id":1}}"#,
        command,
        params_json.join(",")
    );

    // Authorization header
    let auth = base64::encode(format!("{}:{}", rpcuser, rpcpassword));

    // Send HTTP POST
    let request = format!(
        "POST / HTTP/1.1\r\nHost: {}\r\nAuthorization: Basic {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        addr, auth, body.len(), body
    );

    stream
        .write_all(request.as_bytes())
        .map_err(|e| format!("Failed to send request: {}", e))?;

    let mut reader = BufReader::new(&stream);

    // Read status line
    let mut status_line = String::new();
    reader
        .read_line(&mut status_line)
        .map_err(|e| format!("Failed to read response: {}", e))?;

    if !status_line.contains("200") {
        return Err(format!("Server returned: {}", status_line.trim()));
    }

    // Read headers
    let mut content_length: usize = 0;
    loop {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read headers: {}", e))?;
        let line_trimmed = line.trim();
        if line_trimmed.is_empty() {
            break;
        }
        let lower = line_trimmed.to_lowercase();
        if lower.starts_with("content-length: ") {
            content_length = line_trimmed["content-length: ".len()..]
                .parse()
                .unwrap_or(0);
        }
    }

    // Read body
    let mut body_bytes = vec![0u8; content_length];
    reader
        .read_exact(&mut body_bytes)
        .map_err(|e| format!("Failed to read response body: {}", e))?;
    let body_str = String::from_utf8_lossy(&body_bytes);

    // Parse JSON-RPC response
    let parsed = json::parse(&body_str)
        .map_err(|e| format!("Failed to parse response JSON: {}", e))?;

    if !parsed["error"].is_null() {
        return Err(format!("RPC error: {}", parsed["error"].dump()));
    }

    let result = &parsed["result"];
    Ok(if result.is_string() {
        result.as_str().unwrap().to_string()
    } else {
        result.dump()
    })
}
