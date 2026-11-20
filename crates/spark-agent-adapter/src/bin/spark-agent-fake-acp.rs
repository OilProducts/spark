//! Fake ACP v1 agent for process contracts: one session, one prompt, scripted
//! by the JSON in `SPARK_FAKE_ACP`. Every message it receives is appended to
//! the script's `log` file, after a first line with its pid, argv, working
//! directory, the `log_env` variables and the contents of file arguments.
//! `answers[n]` answers the log's nth prompt, across launches. `forms` are `elicitation/create` requests sent in turn
//! during the prompt, each waiting for its reply unless it `exit`s or is
//! `withdraw`n with `$/cancel_request`. `auth_methods` are offered at
//! initialize, terminal ones only to a client with `auth.terminal`;
//! `authenticate` creates the `signed_in` file, after which `new_error` no
//! longer applies, as a harness keeps its own sign-in, or with
//! `authenticate_waits` never answers, as a sign-in the user leaves.

use serde_json::{json, Value};
use std::io::{BufRead, Write};

fn main() {
    // Bound blocked contracts so a missing reply fails instead of hanging the gate.
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(20));
        eprintln!("fake ACP agent timed out");
        std::process::exit(3);
    });
    let script: Value = std::env::var("SPARK_FAKE_ACP")
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    let mut agent = Agent {
        log: script["log"].as_str().map(str::to_string),
        lines: std::io::stdin().lock().lines(),
        session_id: script["session_id"]
            .as_str()
            .unwrap_or("fake-session")
            .to_string(),
        script,
        cancelled: false,
    };
    agent.startup();
    while let Some(message) = agent.read() {
        agent.handle(message);
    }
}

struct Agent {
    script: Value,
    log: Option<String>,
    lines: std::io::Lines<std::io::StdinLock<'static>>,
    session_id: String,
    /// `session/cancel` arrived while a form waited.
    cancelled: bool,
}

impl Agent {
    fn startup(&self) {
        let argv = std::env::args().skip(1).collect::<Vec<_>>();
        let env = self.script["log_env"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|name| (name.to_string(), json!(std::env::var(name).ok())))
            .collect::<serde_json::Map<_, _>>();
        let files = argv
            .iter()
            .filter_map(|arg| Some((arg.clone(), json!(std::fs::read_to_string(arg).ok()?))))
            .collect::<serde_json::Map<_, _>>();
        self.append(json!({
            "pid": std::process::id(),
            "argv": argv,
            "cwd": std::env::current_dir().ok(),
            "env": env,
            "files": files,
        }));
    }

    fn append(&self, entry: Value) {
        if let Some(path) = &self.log {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .expect("fake ACP log");
            // One write per line, so concurrent fakes sharing a log never interleave.
            file.write_all(format!("{entry}\n").as_bytes())
                .expect("fake ACP log write");
        }
    }

    fn read(&mut self) -> Option<Value> {
        let line = self.lines.next()?.ok()?;
        let message: Value = serde_json::from_str(&line).expect("client sent JSON");
        self.append(json!({"received": message}));
        Some(message)
    }

    fn send(&self, message: Value) {
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{message}").unwrap();
        stdout.flush().unwrap();
    }

    fn reply(&self, id: &Value, result: Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}));
    }

    fn fail(&self, id: &Value, error: &Value) {
        self.send(json!({"jsonrpc": "2.0", "id": id, "error": error}));
    }

    fn update(&self, update: &Value) {
        self.send(json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {"sessionId": self.session_id, "update": update},
        }));
    }

    fn setup_result(&self) -> Value {
        let mut result = json!({"sessionId": self.session_id});
        for (key, field) in [("config_options", "configOptions"), ("modes", "modes")] {
            if let Some(value) = self.script.get(key) {
                result[field] = value.clone();
            }
        }
        result
    }

    fn handle(&mut self, message: Value) {
        let id = message["id"].clone();
        let script = self.script.clone();
        match message["method"].as_str().unwrap_or_default() {
            "initialize" => {
                // As ACP requires, terminal methods only for a client that runs them.
                let terminal = message["params"]["clientCapabilities"]["auth"]["terminal"] == true;
                let methods = script["auth_methods"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|method| terminal || method["type"] != "terminal")
                    .cloned()
                    .collect::<Vec<_>>();
                let mut session = json!({});
                if script["system_prompt"].as_bool().unwrap_or(false) {
                    session["systemPrompt"] = json!({});
                }
                if script["resume"].as_bool().unwrap_or(false) {
                    session["resume"] = json!({});
                }
                self.reply(
                    &id,
                    json!({
                        "protocolVersion": script["protocol_version"].as_u64().unwrap_or(1),
                        "agentCapabilities": {
                            "loadSession": script["load"].as_bool().unwrap_or(false),
                            "sessionCapabilities": session,
                        },
                        "authMethods": methods,
                    }),
                );
            }
            method @ ("session/new" | "session/load" | "session/resume") => {
                let kind = method.trim_start_matches("session/");
                let signed_in = script["signed_in"]
                    .as_str()
                    .is_some_and(|path| std::path::Path::new(path).exists());
                if let Some(error) = script
                    .get(&format!("{kind}_error"))
                    .filter(|_| !(kind == "new" && signed_in))
                {
                    return self.fail(&id, error);
                }
                if kind != "new" {
                    self.session_id = message["params"]["sessionId"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                }
                if kind == "load" {
                    for update in script["replay"].as_array().into_iter().flatten() {
                        self.update(update);
                    }
                }
                self.reply(&id, self.setup_result());
            }
            "session/set_config_option" => {
                let (config, value) = (
                    message["params"]["configId"].clone(),
                    message["params"]["value"].clone(),
                );
                // `on_set["id=value"]` is the full set a choice leads to, as
                // when a model offers its own efforts.
                let chosen = format!(
                    "{}={}",
                    config.as_str().unwrap_or_default(),
                    value.as_str().unwrap_or_default()
                );
                let mut options = script["on_set"]
                    .get(&chosen)
                    .unwrap_or(&script["config_options"])
                    .clone();
                for option in options.as_array_mut().into_iter().flatten() {
                    if option["id"] == config {
                        option["currentValue"] = value.clone();
                    }
                }
                self.script["config_options"] = options.clone();
                self.reply(&id, json!({"configOptions": options}));
            }
            "authenticate" => {
                if let Some(error) = script.get("authenticate_error") {
                    return self.fail(&id, error);
                }
                if script["authenticate_waits"] == true {
                    return;
                }
                if let Some(path) = script["signed_in"].as_str() {
                    std::fs::write(path, "").expect("fake ACP sign-in");
                }
                self.reply(&id, json!({}));
            }
            "session/prompt" => self.prompt(&id),
            _ => {}
        }
    }

    fn prompt(&mut self, id: &Value) {
        let script = self.script.clone();
        if let Some(exit) = script.get("exit_on_prompt") {
            eprintln!(
                "{}",
                exit["stderr"].as_str().unwrap_or("fake ACP agent exiting")
            );
            std::process::exit(exit["code"].as_i64().unwrap_or(1) as i32);
        }
        if script["malformed_on_prompt"].as_bool().unwrap_or(false) {
            println!("this is not JSON-RPC");
            std::io::stdout().flush().unwrap();
        }
        for update in script["updates"].as_array().into_iter().flatten() {
            self.update(update);
        }
        if let Some(answers) = script["answers"].as_array() {
            let log = std::fs::read_to_string(script["log"].as_str().unwrap()).unwrap();
            let prompts = log.matches("\"method\":\"session/prompt\"").count();
            let text = &answers[(prompts - 1).min(answers.len() - 1)];
            self.update(&json!({"sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": text}}));
        }
        for (index, form) in script["forms"].as_array().into_iter().flatten().enumerate() {
            let id = format!("form-{}", index + 1);
            let mut params = form["params"].clone();
            params["sessionId"] = json!(self.session_id);
            self.send(json!({"jsonrpc": "2.0", "id": id, "method": "elicitation/create", "params": params}));
            // Exits, as a crashed harness would, once the file `exit` names exists.
            if let Some(exit) = form["exit"].as_str() {
                while !std::path::Path::new(exit).exists() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                std::process::exit(4);
            }
            if form["withdraw"].as_bool().unwrap_or(false) {
                self.send(json!({"jsonrpc": "2.0", "method": "$/cancel_request", "params": {"requestId": id}}));
            }
            // The reply is logged as it is read.
            while let Some(message) = self.read() {
                self.cancelled |= message["method"] == "session/cancel";
                if message["id"] == id && message.get("method").is_none() {
                    break;
                }
            }
        }
        if let Some(permission) = script.get("permission") {
            self.send(json!({
                "jsonrpc": "2.0",
                "id": "permission-1",
                "method": "session/request_permission",
                "params": {
                    "sessionId": self.session_id,
                    "toolCall": permission["toolCall"],
                    "options": permission["options"],
                },
            }));
            // The reply is logged as it is read.
            while let Some(message) = self.read() {
                if message["id"] == "permission-1" {
                    break;
                }
            }
        }
        if let Some(gate) = script["gate"].as_str() {
            while !std::path::Path::new(gate).exists() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        if let Some(on_cancel) = script["await_cancel"].as_str() {
            while !self.cancelled {
                let Some(message) = self.read() else { break };
                self.cancelled = message["method"] == "session/cancel";
            }
            if on_cancel == "ignore" {
                while self.read().is_some() {}
                return;
            }
            return self.reply(id, json!({"stopReason": "cancelled"}));
        }
        if let Some(error) = script.get("prompt_error") {
            return self.fail(id, error);
        }
        let mut result = json!({
            "stopReason": script["stop_reason"].as_str().unwrap_or("end_turn"),
        });
        if let Some(usage) = script.get("usage") {
            result["usage"] = usage.clone();
        }
        self.reply(id, result);
        if let Some(on_cancel) = script["post_prompt_await_cancel"].as_str() {
            while let Some(message) = self.read() {
                if message["method"] == "session/cancel" {
                    if on_cancel == "ignore" {
                        while self.read().is_some() {}
                        return;
                    }
                    break;
                }
            }
        }
        if let Some(gate) = script["post_prompt_gate"].as_str() {
            while !std::path::Path::new(gate).exists() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
        for update in script["post_prompt_updates"]
            .as_array()
            .into_iter()
            .flatten()
        {
            self.update(update);
        }
    }
}
