use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

fn main() -> std::io::Result<()> {
    let address = backend_address();
    let listener = TcpListener::bind(&address)?;

    println!("sanchoris-backend listening on http://{address}");

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => handle_connection(stream)?,
            Err(error) => eprintln!("failed to accept connection: {error}"),
        }
    }

    Ok(())
}

fn backend_address() -> String {
    std::env::var("SANCHORIS_BACKEND_ADDR").unwrap_or_else(|_| {
        std::env::var("PORT")
            .map(|port| format!("127.0.0.1:{port}"))
            .unwrap_or_else(|_| "127.0.0.1:3000".to_string())
    })
}

fn handle_connection(mut stream: TcpStream) -> std::io::Result<()> {
    let mut buffer = [0; 1024];
    let bytes_read = stream.read(&mut buffer)?;
    let request = String::from_utf8_lossy(&buffer[..bytes_read]);
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    let (status, content_type, body) = match path {
        "/health" | "/api/v1/health" => (
            "200 OK",
            "application/json",
            r#"{"status":"ok","service":"sanchoris-backend"}"#.to_string(),
        ),
        _ => (
            "200 OK",
            "text/plain; charset=utf-8",
            "sanchoris-backend is running".to_string(),
        ),
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())
}
