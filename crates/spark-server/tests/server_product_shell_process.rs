use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

#[test]
fn serve_binary_process_exposes_product_shell_and_mount_boundaries() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ui_dir = temp.path().join("ui");
    let assets_dir = ui_dir.join("assets");
    fs::create_dir_all(&assets_dir).expect("assets");
    fs::write(
        ui_dir.join("index.html"),
        "<!doctype html><div id=\"root\">process ui</div>\n",
    )
    .expect("index");
    fs::write(
        assets_dir.join("compat-app.js"),
        "window.__sparkProcessAsset = true;\n",
    )
    .expect("script");
    fs::write(
        assets_dir.join("spark-app-icon.png"),
        b"spark-process-favicon\n",
    )
    .expect("favicon");

    let server = RunningServer::start(temp.path(), &ui_dir);

    let index = http_get(server.port, "/");
    assert_eq!(index.status, 200);
    assert_eq!(
        index.header("content-type"),
        Some("text/html; charset=utf-8")
    );
    assert_eq!(
        index.body_text(),
        "<!doctype html><div id=\"root\">process ui</div>\n"
    );

    let script = http_get(server.port, "/assets/compat-app.js");
    assert_eq!(script.status, 200);
    assert_eq!(
        script.header("content-type"),
        Some("text/javascript; charset=utf-8")
    );
    assert_eq!(script.body_text(), "window.__sparkProcessAsset = true;\n");

    let favicon = http_get(server.port, "/favicon.ico");
    assert_eq!(favicon.status, 200);
    assert_eq!(favicon.header("content-type"), Some("image/png"));
    assert_eq!(favicon.body, b"spark-process-favicon\n");

    let workspace_base = http_get(server.port, "/workspace");
    let expected_workspace_location = format!("http://127.0.0.1:{}/workspace/", server.port);
    assert_eq!(workspace_base.status, 307);
    assert_eq!(
        workspace_base.header("location"),
        Some(expected_workspace_location.as_str())
    );
    assert!(workspace_base.header("content-type").is_none());
    assert!(workspace_base.body.is_empty());

    let workspace_slash = http_get(server.port, "/workspace/");
    assert_eq!(workspace_slash.status, 404);
    assert_eq!(
        workspace_slash.header("content-type"),
        Some("application/json")
    );
    assert_eq!(workspace_slash.body_text(), r#"{"detail":"Not Found"}"#);

    let attractor_base = http_get(server.port, "/attractor");
    let expected_attractor_location = format!("http://127.0.0.1:{}/attractor/", server.port);
    assert_eq!(attractor_base.status, 307);
    assert_eq!(
        attractor_base.header("location"),
        Some(expected_attractor_location.as_str())
    );
    assert!(attractor_base.header("content-type").is_none());
    assert!(attractor_base.body.is_empty());

    let attractor_slash = http_get(server.port, "/attractor/");
    assert_eq!(attractor_slash.status, 404);
    assert_eq!(
        attractor_slash.header("content-type"),
        Some("application/json")
    );
    assert_eq!(attractor_slash.body_text(), r#"{"detail":"Not Found"}"#);

    let api_missing = http_get(server.port, "/workspace/api/missing");
    assert_eq!(api_missing.status, 404);
    assert_eq!(api_missing.header("content-type"), Some("application/json"));
    assert_eq!(api_missing.body_text(), r#"{"detail":"Not Found"}"#);
    assert!(!api_missing.body_text().contains("process ui"));
}

struct RunningServer {
    child: Child,
    port: u16,
}

impl RunningServer {
    fn start(root: &Path, ui_dir: &Path) -> Self {
        let port = free_tcp_port();
        let data_dir = root.join("spark-home");
        let flows_dir = root.join("flows");
        let port_text = port.to_string();
        let data_dir_text = data_dir.to_string_lossy().into_owned();
        let flows_dir_text = flows_dir.to_string_lossy().into_owned();
        let ui_dir_text = ui_dir.to_string_lossy().into_owned();
        let mut child = Command::new(env!("CARGO_BIN_EXE_spark-server"))
            .args([
                "serve",
                "--host",
                "127.0.0.1",
                "--port",
                port_text.as_str(),
                "--data-dir",
                data_dir_text.as_str(),
                "--flows-dir",
                flows_dir_text.as_str(),
                "--ui-dir",
                ui_dir_text.as_str(),
            ])
            .current_dir(repo_root())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn spark-server serve");

        for _ in 0..100 {
            if let Some(status) = child.try_wait().expect("poll spark-server") {
                let mut stderr = String::new();
                if let Some(mut stream) = child.stderr.take() {
                    let _ = stream.read_to_string(&mut stderr);
                }
                panic!("spark-server exited early with {status}: {stderr}");
            }
            match try_http_get(port, "/") {
                Ok(response) if response.status == 200 => return Self { child, port },
                _ => thread::sleep(Duration::from_millis(50)),
            }
        }

        let _ = child.kill();
        let _ = child.wait();
        panic!("spark-server did not become ready on port {port}");
    }
}

impl Drop for RunningServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .get(&name.to_ascii_lowercase())
            .map(String::as_str)
    }

    fn body_text(&self) -> String {
        String::from_utf8(self.body.clone()).expect("utf-8 response body")
    }
}

fn http_get(port: u16, path: &str) -> HttpResponse {
    try_http_get(port, path).expect("http response")
}

fn try_http_get(port: u16, path: &str) -> std::io::Result<HttpResponse> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"
    )?;

    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes)?;
    parse_http_response(&bytes)
}

fn parse_http_response(bytes: &[u8]) -> std::io::Result<HttpResponse> {
    let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "HTTP response did not include a header terminator",
        ));
    };
    let header_text = String::from_utf8_lossy(&bytes[..header_end]);
    let mut lines = header_text.lines();
    let status = lines
        .next()
        .expect("status line")
        .split_whitespace()
        .nth(1)
        .expect("status code")
        .parse::<u16>()
        .expect("numeric status");
    let headers = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_ascii_lowercase(), value.trim().to_string()))
        })
        .collect::<BTreeMap<_, _>>();
    let raw_body = &bytes[header_end + 4..];
    let body = if headers
        .get("transfer-encoding")
        .is_some_and(|value| value.to_ascii_lowercase().contains("chunked"))
    {
        decode_chunked_body(raw_body)
    } else {
        raw_body.to_vec()
    };

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

fn decode_chunked_body(raw_body: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::new();
    let mut cursor = 0;
    while cursor < raw_body.len() {
        let Some(line_end) = raw_body[cursor..]
            .windows(2)
            .position(|window| window == b"\r\n")
            .map(|offset| cursor + offset)
        else {
            break;
        };
        let size_text = String::from_utf8_lossy(&raw_body[cursor..line_end]);
        let size = usize::from_str_radix(size_text.split(';').next().unwrap_or("").trim(), 16)
            .expect("chunk size");
        cursor = line_end + 2;
        if size == 0 {
            break;
        }
        decoded.extend_from_slice(&raw_body[cursor..cursor + size]);
        cursor += size + 2;
    }
    decoded
}

fn free_tcp_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .expect("bind free port")
        .local_addr()
        .expect("local address")
        .port()
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}
