//! An OpenAI-compatible chat endpoint for native agent nodes. Each request is
//! answered over SSE by the test's handler, on its own thread.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

pub struct Sse(TcpStream);

impl Sse {
    fn send(&mut self, chunk: Value) -> std::io::Result<()> {
        write!(self.0, "data: {chunk}\n\n")?;
        self.0.flush()
    }

    pub fn text(&mut self, text: &str) {
        let _ = self.send(
            json!({"id": "fake", "object": "chat.completion.chunk", "model": "test-model",
            "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": null}]}),
        );
    }

    pub fn tool_call(&mut self, id: &str, name: &str, arguments: Value) {
        let _ = self.send(
            json!({"id": "fake", "object": "chat.completion.chunk", "model": "test-model",
            "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": id, "type": "function",
            "function": {"name": name, "arguments": arguments.to_string()}}]}, "finish_reason": null}]}),
        );
    }

    pub fn finish(&mut self) {
        let _ = self.send(
            json!({"id": "fake", "object": "chat.completion.chunk", "model": "test-model",
            "choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]}),
        );
        let _ = write!(self.0, "data: [DONE]\n\n");
    }

    /// Keeps the response open until the client hangs up; returns once it has.
    pub fn hold(&mut self) {
        while write!(self.0, ": waiting\n\n")
            .and_then(|_| self.0.flush())
            .is_ok()
        {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// Serves `handler` on a local port; returns the `base_url` to configure.
pub fn serve(handler: impl Fn(Value, &mut Sse) + Send + Sync + 'static) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let handler = std::sync::Arc::new(handler);
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let handler = handler.clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse::<usize>().unwrap();
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                let mut sse = Sse(stream);
                let _ = write!(
                    sse.0,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
                );
                handler(serde_json::from_slice(&body).unwrap(), &mut sse);
            });
        }
    });
    base_url
}

/// Points the native `openai_compatible` provider at `base_url`.
pub fn configure(config_dir: &std::path::Path, base_url: &str) {
    std::fs::create_dir_all(config_dir).unwrap();
    std::fs::write(
        config_dir.join("spark.toml"),
        format!(
            "[models]\nprovider='openai_compatible'\nmodel='test-model'\n[providers.openai_compatible]\nbase_url='{base_url}'\n"
        ),
    )
    .unwrap();
}
