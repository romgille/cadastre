//! A tiny stand-in for the Docker Engine API, served over local TCP.
//!
//! It implements just the endpoints cadastre uses, including a chunked
//! `/events` stream, so the client and the daemon can be tested end to end
//! without a Docker daemon.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};

/// What the mock currently serves. Tests change it while the mock runs.
#[derive(Default)]
pub struct State {
    /// Inspect documents by container ID.
    pub containers: BTreeMap<String, Value>,
    /// IDs that are listed but answer 404 when inspected (a container that
    /// was removed in between).
    pub ghosts: Vec<String>,
    /// Answer every request with this status instead.
    pub status: Option<u16>,
    /// Event lines sent, one chunk each, to the next `/events` client.
    pub events: Vec<String>,
    /// Number of `/events` connections opened so far.
    pub event_connections: usize,
    /// Number of `/containers/json` requests so far.
    pub list_requests: usize,
    /// Every request path seen, in order.
    pub requests: Vec<String>,
}

pub struct MockDocker {
    pub url: String,
    pub state: Arc<Mutex<State>>,
}

impl MockDocker {
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State::default()));
        let shared = Arc::clone(&state);
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let shared = Arc::clone(&shared);
                thread::spawn(move || {
                    let _ = serve(stream, &shared);
                });
            }
        });
        Self { url, state }
    }

    pub fn with_containers(documents: &[Value]) -> Self {
        let mock = Self::start();
        for document in documents {
            mock.add(document.clone());
        }
        mock
    }

    pub fn add(&self, document: Value) {
        let id = document["Id"].as_str().unwrap().to_string();
        self.state.lock().unwrap().containers.insert(id, document);
    }

    pub fn remove(&self, id: &str) {
        self.state.lock().unwrap().containers.remove(id);
    }

    pub fn set_status(&self, status: Option<u16>) {
        self.state.lock().unwrap().status = status;
    }

    pub fn push_event(&self, kind: &str, action: &str) {
        let line = json!({"Type": kind, "Action": action}).to_string();
        self.state.lock().unwrap().events.push(line);
    }

    pub fn event_connections(&self) -> usize {
        self.state.lock().unwrap().event_connections
    }

    pub fn list_requests(&self) -> usize {
        self.state.lock().unwrap().list_requests
    }
}

fn serve(mut stream: TcpStream, state: &Mutex<State>) -> std::io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    reader.read_line(&mut request_line)?;
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header.trim().is_empty() {
            break;
        }
    }
    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();
    let route = path.split('?').next().unwrap_or("/").to_string();

    let forced = {
        let mut state = state.lock().unwrap();
        state.requests.push(path.clone());
        state.status
    };
    if let Some(status) = forced {
        return respond(&mut stream, status, "{}");
    }

    if route == "/containers/json" {
        let body = {
            let mut state = state.lock().unwrap();
            state.list_requests += 1;
            let ids = state.containers.keys().chain(state.ghosts.iter());
            Value::Array(ids.map(|id| json!({ "Id": id })).collect()).to_string()
        };
        return respond(&mut stream, 200, &body);
    }
    if let Some(id) = route
        .strip_prefix("/containers/")
        .and_then(|rest| rest.strip_suffix("/json"))
    {
        let document = state.lock().unwrap().containers.get(id).cloned();
        return match document {
            Some(document) => respond(&mut stream, 200, &document.to_string()),
            None => respond(&mut stream, 404, r#"{"message":"No such container"}"#),
        };
    }
    if route == "/events" {
        let events = {
            let mut state = state.lock().unwrap();
            state.event_connections += 1;
            std::mem::take(&mut state.events)
        };
        return stream_events(&mut stream, &events, state);
    }
    respond(&mut stream, 404, "{}")
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

/// Sends the queued events as chunks, then keeps the connection open for a
/// while, sending events that are queued later, like a real daemon.
fn stream_events(
    stream: &mut TcpStream,
    first: &[String],
    state: &Mutex<State>,
) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
         Transfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
    )?;
    let chunk = |stream: &mut TcpStream, line: &str| -> std::io::Result<()> {
        let data = format!("{line}\n");
        write!(stream, "{:x}\r\n{data}\r\n", data.len())?;
        stream.flush()
    };
    for line in first {
        chunk(stream, line)?;
    }
    // Poll for events queued afterwards. A closed client makes the write fail,
    // which ends this thread.
    for _ in 0..600 {
        thread::sleep(Duration::from_millis(50));
        let queued = std::mem::take(&mut state.lock().unwrap().events);
        for line in queued {
            chunk(stream, &line)?;
        }
    }
    write!(stream, "0\r\n\r\n")
}
