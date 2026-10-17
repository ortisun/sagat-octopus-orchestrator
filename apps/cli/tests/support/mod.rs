#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

pub struct Reply {
    pub status: u16,
    pub body: String,
}

impl Reply {
    pub fn ok(body: impl Into<String>) -> Self {
        Self {
            status: 200,
            body: body.into(),
        }
    }
}

pub fn server(replies: Vec<Reply>) -> (String, mpsc::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("listen");
    let address = format!("http://{}", listener.local_addr().expect("address"));
    let (sent, received) = mpsc::channel();
    std::thread::spawn(move || {
        for reply in replies {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let header_end = loop {
                let read = stream.read(&mut chunk).expect("read request");
                assert!(read > 0, "request ended before its headers");
                request.extend_from_slice(&chunk[..read]);
                if let Some(at) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    break at + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().expect("content length"))
                })
                .unwrap_or(0);
            while request.len() < header_end + content_length {
                let read = stream.read(&mut chunk).expect("read body");
                assert!(read > 0, "request body ended early");
                request.extend_from_slice(&chunk[..read]);
            }
            sent.send(request).expect("record request");
            let reason = if reply.status == 200 { "OK" } else { "Refused" };
            write!(
                stream,
                "HTTP/1.1 {} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                reply.status,
                reply.body.len(),
                reply.body
            )
            .expect("write response");
        }
    });
    (address, received)
}

pub fn request_line(request: &[u8]) -> &str {
    std::str::from_utf8(request)
        .expect("request is utf8")
        .lines()
        .next()
        .expect("request line")
}

pub fn request_body(request: &[u8]) -> serde_json::Value {
    let at = request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .expect("header end")
        + 4;
    serde_json::from_slice(&request[at..]).expect("json request body")
}
