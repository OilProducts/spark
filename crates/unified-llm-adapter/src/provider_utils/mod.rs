use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStreamPayloadError {
    pub message: String,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderStreamRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sse_event: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_event: Option<String>,
    #[serde(default)]
    pub data: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload_error: Option<ProviderStreamPayloadError>,
    #[serde(default)]
    pub done: bool,
}

impl ProviderStreamRecord {
    pub fn from_json(payload: Value) -> Self {
        let json_event = provider_json_event_name(&payload);
        Self {
            event: json_event.clone(),
            sse_event: None,
            json_event,
            data: payload.to_string(),
            retry: None,
            payload: Some(payload),
            payload_error: None,
            done: false,
        }
    }

    pub fn from_sse_data(sse_event: Option<String>, data: String, retry: Option<u64>) -> Self {
        let trimmed = data.trim();
        if trimmed == "[DONE]" {
            return Self {
                event: sse_event.clone(),
                sse_event,
                json_event: None,
                data,
                retry,
                payload: None,
                payload_error: None,
                done: true,
            };
        }

        match serde_json::from_str::<Value>(&data) {
            Ok(payload) => {
                let json_event = provider_json_event_name(&payload);
                let event = json_event.clone().or_else(|| sse_event.clone());
                Self {
                    event,
                    sse_event,
                    json_event,
                    data,
                    retry,
                    payload: Some(payload),
                    payload_error: None,
                    done: false,
                }
            }
            Err(error) => Self {
                event: sse_event.clone(),
                sse_event,
                json_event: None,
                payload: None,
                payload_error: Some(ProviderStreamPayloadError {
                    message: error.to_string(),
                    raw: data.clone(),
                }),
                data,
                retry,
                done: false,
            },
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SseParser {
    event: Option<String>,
    data_lines: Vec<String>,
    retry: Option<u64>,
    pending_line: String,
}

impl SseParser {
    pub fn push_str(&mut self, chunk: &str) -> Vec<ProviderStreamRecord> {
        self.pending_line.push_str(chunk);
        let mut records = Vec::new();

        while let Some(newline_index) = self.pending_line.find('\n') {
            let mut line = self.pending_line[..newline_index].to_string();
            self.pending_line.drain(..=newline_index);
            if line.ends_with('\r') {
                line.pop();
            }
            self.process_line(&line, &mut records);
        }

        records
    }

    pub fn finish(&mut self) -> Vec<ProviderStreamRecord> {
        let mut records = Vec::new();
        if !self.pending_line.is_empty() {
            let line = std::mem::take(&mut self.pending_line);
            let line = line.strip_suffix('\r').unwrap_or(&line).to_string();
            self.process_line(&line, &mut records);
        }
        self.dispatch_event(&mut records);
        records
    }

    pub fn has_pending_input(&self) -> bool {
        !self.pending_line.is_empty()
            || self.event.is_some()
            || !self.data_lines.is_empty()
            || self.retry.is_some()
    }

    fn process_line(&mut self, line: &str, records: &mut Vec<ProviderStreamRecord>) {
        if line.is_empty() {
            self.dispatch_event(records);
            return;
        }

        if line.starts_with(':') {
            return;
        }

        let (field, value) = match line.split_once(':') {
            Some((field, value)) => {
                let value = value.strip_prefix(' ').unwrap_or(value);
                (field, value)
            }
            None => (line, ""),
        };

        match field {
            "event" => self.event = Some(value.to_string()),
            "data" => self.data_lines.push(value.to_string()),
            "retry" => {
                if let Ok(retry) = value.parse::<u64>() {
                    self.retry = Some(retry);
                }
            }
            _ => {}
        }
    }

    fn dispatch_event(&mut self, records: &mut Vec<ProviderStreamRecord>) {
        if self.data_lines.is_empty() {
            self.event = None;
            self.retry = None;
            return;
        }

        let data = self.data_lines.join("\n");
        records.push(ProviderStreamRecord::from_sse_data(
            self.event.take(),
            data,
            self.retry.take(),
        ));
        self.data_lines.clear();
    }
}

pub fn parse_sse_stream(input: &str) -> Vec<ProviderStreamRecord> {
    let mut parser = SseParser::default();
    let mut records = parser.push_str(input);
    records.extend(parser.finish());
    records
}

pub fn provider_json_event_name(payload: &Value) -> Option<String> {
    payload
        .get("type")
        .and_then(Value::as_str)
        .filter(|event| !event.is_empty())
        .or_else(|| {
            payload
                .get("event")
                .and_then(Value::as_str)
                .filter(|event| !event.is_empty())
        })
        .map(str::to_string)
}
