use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use crossbeam_channel::Sender;

use super::client;
use super::types::WsInMessage;

/// Embedded HTML control surface.
/// In debug mode, try to read from filesystem for hot-reload; fall back to embedded.
fn get_html_content() -> String {
    #[cfg(debug_assertions)]
    {
        let path = crate::effect::loader::assets_dir().join("web/control.html");
        if let Ok(content) = std::fs::read_to_string(&path) {
            return content;
        }
    }
    include_str!("../../../../assets/web/control.html").to_string()
}

/// Uploads streamed at the same time. More get 503 and the browser retries.
const MAX_UPLOADS: usize = 2;

/// Spawn the accept loop thread. Returns (shutdown_flag, thread_handle).
pub fn spawn_accept_loop(
    port: u16,
    inbound_tx: Sender<WsInMessage>,
    clients: Arc<Mutex<Vec<Sender<String>>>>,
    latest_state: Arc<Mutex<String>>,
    shutdown: Arc<AtomicBool>,
    dropped: Arc<AtomicU64>,
) -> anyhow::Result<JoinHandle<()>> {
    let addr = format!("127.0.0.1:{port}");
    let listener = TcpListener::bind(&addr)?;
    listener.set_nonblocking(false)?;
    // Short accept timeout so we can check shutdown flag
    let _ = listener.set_nonblocking(false);
    log::info!("Web control server listening on http://127.0.0.1:{port}");

    let client_counter = Arc::new(AtomicUsize::new(0));
    let uploads = Arc::new(AtomicUsize::new(0));

    let handle = thread::Builder::new()
        .name("fosfora-web-accept".into())
        .spawn(move || {
            // Set the listener to have a timeout for accept
            let _ = listener.set_nonblocking(true);

            while !shutdown.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, addr)) => {
                        log::debug!("Web connection from {addr}");
                        let _ = stream.set_nonblocking(false);
                        // Each request runs on its own thread so a large upload
                        // never holds up the page, thumbnails or a reconnect.
                        let inbound_tx = inbound_tx.clone();
                        let clients = clients.clone();
                        let latest_state = latest_state.clone();
                        let shutdown = shutdown.clone();
                        let client_counter = client_counter.clone();
                        let dropped = dropped.clone();
                        let uploads = uploads.clone();
                        let spawned = thread::Builder::new()
                            .name("fosfora-web-request".into())
                            .spawn(move || {
                                handle_connection(
                                    stream,
                                    addr,
                                    &inbound_tx,
                                    &clients,
                                    &latest_state,
                                    &shutdown,
                                    &client_counter,
                                    &dropped,
                                    &uploads,
                                );
                            });
                        if let Err(err) = spawned {
                            log::warn!("Web request thread did not start: {err}");
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No pending connection — sleep briefly before retrying
                        thread::sleep(Duration::from_millis(50));
                    }
                    Err(e) => {
                        if !shutdown.load(Ordering::Relaxed) {
                            log::error!("Web accept error: {e}");
                        }
                        break;
                    }
                }
            }
            log::info!("Web accept thread shutting down");
        })?;

    Ok(handle)
}

fn handle_connection(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    inbound_tx: &Sender<WsInMessage>,
    clients: &Arc<Mutex<Vec<Sender<String>>>>,
    latest_state: &Arc<Mutex<String>>,
    shutdown: &Arc<AtomicBool>,
    client_counter: &Arc<AtomicUsize>,
    dropped: &Arc<AtomicU64>,
    uploads: &Arc<AtomicUsize>,
) {
    // Peek at the first bytes to determine if this is a WebSocket upgrade or plain HTTP
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));

    let mut buf = [0u8; 4096];
    let n = match stream.read(&mut buf) {
        Ok(n) if n > 0 => n,
        _ => return,
    };

    let request = String::from_utf8_lossy(&buf[..n]);

    if request_line_is_media_upload(&request) {
        if !super::upload::peer_is_loopback(peer) {
            let _ = write_http(
                &mut stream,
                "403 Forbidden",
                "text/plain",
                "media upload is only accepted from this computer",
            );
            return;
        }
        // At most two uploads stream at once; the browser sends two at a time.
        if uploads.fetch_add(1, Ordering::AcqRel) >= MAX_UPLOADS {
            uploads.fetch_sub(1, Ordering::AcqRel);
            let _ = write_http(
                &mut stream,
                "503 Service Unavailable",
                "text/plain",
                "two uploads are already running; try again when one finishes",
            );
            return;
        }
        handle_media_upload(&mut stream, &buf[..n], inbound_tx);
        uploads.fetch_sub(1, Ordering::AcqRel);
        return;
    }

    if let Some(path) = request_path(&request) {
        if path == "/api/media/finder-drag" {
            if !super::upload::peer_is_loopback(peer) {
                let _ = write_http(
                    &mut stream,
                    "403 Forbidden",
                    "text/plain",
                    "media paths are only available on this computer",
                );
                return;
            }
            let paths = crate::media::library::finder_drag_paths();
            let body = serde_json::to_string(&paths).unwrap_or_else(|_| "[]".into());
            let _ = write_http(&mut stream, "200 OK", "application/json", &body);
            return;
        }
        if path == "/api/media" || path.starts_with("/api/media/thumb/") {
            serve_library_get(&mut stream, path);
            return;
        }
    }

    if is_websocket_upgrade(&request) {
        // Set read timeout for interleaved read/write in client handler
        let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
        // Replay already-read bytes then continue from the stream
        let replay = ReplayStream::new(buf[..n].to_vec(), stream);
        match tungstenite::accept(replay) {
            Ok(ws) => {
                let client_id = client_counter.fetch_add(1, Ordering::Relaxed);
                let (outbound_tx, outbound_rx) = crossbeam_channel::bounded(256);

                // Get latest state for initial sync — recover from poisoned mutex
                // rather than panicking the accept thread during a live performance.
                let state = latest_state
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();

                // Register client
                clients
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(outbound_tx);

                let tx = inbound_tx.clone();
                let flag = shutdown.clone();
                let dropped = dropped.clone();

                thread::Builder::new()
                    .name(format!("fosfora-web-client-{client_id}"))
                    .spawn(move || {
                        client::run_client(ws, tx, outbound_rx, state, flag, client_id, dropped);
                    })
                    .ok();
            }
            Err(e) => {
                log::debug!("WebSocket handshake failed: {e}");
            }
        }
    } else {
        // Plain HTTP — serve the control surface HTML
        serve_http(&mut stream, &request);
    }
}

fn request_line_is_media_upload(request: &str) -> bool {
    request
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("POST /api/media") || line.starts_with("POST /media"))
}

fn request_path(request: &str) -> Option<&str> {
    let path = request.lines().next()?.split_whitespace().nth(1)?;
    Some(path.split('?').next().unwrap_or(path))
}

/// Keep a browser-supplied filename as a single path segment.
fn safe_upload_name(raw: &str) -> Option<String> {
    let name = raw.split(['/', '\\']).next_back()?.trim();
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() || cleaned.starts_with('.') {
        None
    } else {
        Some(cleaned)
    }
}

fn handle_media_upload(stream: &mut TcpStream, first: &[u8], inbound_tx: &Sender<WsInMessage>) {
    let header_end = first.windows(4).position(|w| w == b"\r\n\r\n");
    let Some(header_end) = header_end else {
        let _ = write_http(stream, "400 Bad Request", "text/plain", "missing headers");
        return;
    };
    let headers = String::from_utf8_lossy(&first[..header_end]);
    let Some(pack_root) = super::upload::show_pack_root() else {
        let _ = write_http(
            stream,
            "409 Conflict",
            "text/plain",
            "open a show pack before importing media",
        );
        return;
    };
    let content_type = header_value(&headers, "content-type").unwrap_or_default();
    let Some(content_length) =
        header_value(&headers, "content-length").and_then(|value| value.parse::<u64>().ok())
    else {
        let _ = write_http(
            stream,
            "411 Length Required",
            "text/plain",
            "content-length required",
        );
        return;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(120)));
    let body_at = header_end + 4;
    let prefix = if first.len() > body_at {
        first[body_at..].to_vec()
    } else {
        Vec::new()
    };
    let request_target = headers
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("")
        .to_string();
    let stored = {
        let reader = super::upload::PrefixedRead::new(prefix, &mut *stream);
        if content_type
            .to_ascii_lowercase()
            .contains("multipart/form-data")
        {
            super::upload::receive_multipart(reader, &content_type, content_length, &pack_root)
                .map(|reports| super::upload::reports_json(&reports))
        } else {
            match query_file_name(&request_target) {
                Some(name) => super::upload::receive_raw(reader, content_length, &pack_root, &name)
                    .map(|report| super::upload::reports_json(&[report])),
                None => Err("missing file name".to_string()),
            }
        }
    };
    match stored {
        Ok(json) => {
            let _ = inbound_tx.try_send(WsInMessage::RescanLibrary);
            let _ = write_http(stream, "200 OK", "application/json", &json);
        }
        Err(err) => {
            let status = if err.contains("512") || err.contains("20 GB") {
                "413 Payload Too Large"
            } else {
                "400 Bad Request"
            };
            let _ = write_http(stream, status, "text/plain", &err);
        }
    }
}

fn query_file_name(target: &str) -> Option<String> {
    let query = target.split_once('?')?.1;
    let raw = query
        .split('&')
        .find_map(|part| part.strip_prefix("name="))?;
    crate::media::library::safe_media_name(&urlencoding_decode(raw))
}

fn header_value(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        if key.eq_ignore_ascii_case(name) {
            Some(value.trim().to_string())
        } else {
            None
        }
    })
}

fn serve_library_get(stream: &mut TcpStream, path: &str) {
    let Some(pack_root) = super::upload::show_pack_root() else {
        let _ = write_http(stream, "409 Conflict", "text/plain", "no show pack");
        return;
    };
    if let Some(name) = path.strip_prefix("/api/media/thumb/") {
        match crate::media::library::thumb_path(&pack_root, name) {
            Some(file) => match std::fs::read(file) {
                Ok(bytes) => {
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: image/jpeg\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = stream.write_all(header.as_bytes());
                    let _ = stream.write_all(&bytes);
                    let _ = stream.flush();
                }
                Err(_) => {
                    let _ = write_http(stream, "404 Not Found", "text/plain", "no thumbnail");
                }
            },
            None => {
                let _ = write_http(stream, "404 Not Found", "text/plain", "no thumbnail");
            }
        }
        return;
    }
    match std::fs::read_to_string(crate::media::library::library_json_path(&pack_root)) {
        Ok(json) => {
            let _ = write_http(stream, "200 OK", "application/json", &json);
        }
        Err(_) => {
            let _ = write_http(stream, "200 OK", "application/json", "[]");
        }
    }
}

fn urlencoding_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hex = &raw[i + 1..i + 3];
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write_http(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &str,
) -> std::io::Result<()> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes())?;
    stream.flush()
}

fn is_websocket_upgrade(request: &str) -> bool {
    // Check for WebSocket upgrade headers (case-insensitive)
    let lower = request.to_lowercase();
    lower.contains("upgrade: websocket") || lower.contains("upgrade:websocket")
}

/// Pages the controller keeps in the address bar. A refresh of `/layers` must
/// receive the same document as `/`, or the browser is sent back to Layers' root.
fn is_control_document(path: &str) -> bool {
    let path = path.split(['?', '#']).next().unwrap_or(path);
    let path = if path.len() > 1 {
        path.trim_end_matches('/')
    } else {
        path
    };
    matches!(
        path,
        "/" | "/index.html"
            | "/control"
            | "/layers"
            | "/effects"
            | "/look"
            | "/picture"
            | "/library"
            | "/show"
    )
}

fn serve_http(stream: &mut TcpStream, request: &str) {
    // Parse the request path
    let path = request
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    if is_control_document(path) {
        let html = get_html_content();
        let _ = write_http(stream, "200 OK", "text/html; charset=utf-8", &html);
        return;
    }

    let bare = path.split('?').next().unwrap_or(path);
    let (status, content_type, body) = match bare {
        "/health" => (
            "200 OK",
            "application/json",
            r#"{"status":"ok"}"#.to_string(),
        ),
        _ => {
            // Redirect everything else to /
            let response =
                "HTTP/1.1 302 Found\r\nLocation: /\r\nContent-Length: 0\r\n\r\n".to_string();
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
            return;
        }
    };

    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-cache\r\n\
         Connection: close\r\n\
         \r\n",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
}

/// A wrapper that replays buffered data before reading from the underlying stream.
struct ReplayStream {
    buffer: Vec<u8>,
    pos: usize,
    stream: TcpStream,
}

impl ReplayStream {
    fn new(buffer: Vec<u8>, stream: TcpStream) -> Self {
        Self {
            buffer,
            pos: 0,
            stream,
        }
    }
}

impl Read for ReplayStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.pos < self.buffer.len() {
            let remaining = &self.buffer[self.pos..];
            let n = remaining.len().min(buf.len());
            buf[..n].copy_from_slice(&remaining[..n]);
            self.pos += n;
            Ok(n)
        } else {
            self.stream.read(buf)
        }
    }
}

impl Write for ReplayStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.stream.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.stream.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_controller_page_is_served_in_place() {
        for path in [
            "/",
            "/layers",
            "/effects",
            "/look",
            "/picture",
            "/library",
            "/show",
            "/layers?kept=1",
            "/control",
        ] {
            assert!(is_control_document(path), "{path}");
        }
        assert!(!is_control_document("/health"));
        assert!(!is_control_document("/api/media"));
        assert!(!is_control_document("/ws"));
    }

    #[test]
    fn upload_names_stay_a_single_segment() {
        assert_eq!(safe_upload_name("clip.mp4").as_deref(), Some("clip.mp4"));
        assert_eq!(
            safe_upload_name("../../etc/passwd").as_deref(),
            Some("passwd")
        );
        assert_eq!(
            safe_upload_name("ice loop.png").as_deref(),
            Some("ice_loop.png")
        );
        assert!(safe_upload_name("..").is_none());
        assert!(safe_upload_name(".hidden").is_none());
    }

    #[test]
    fn percent_decoding_keeps_spaces_and_utf8() {
        assert_eq!(urlencoding_decode("ice%20loop.mp4"), "ice loop.mp4");
        assert_eq!(urlencoding_decode("a+b.png"), "a b.png");
    }
}
