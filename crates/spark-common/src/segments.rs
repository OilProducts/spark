//! Provider-neutral projection of [`TurnStreamEvent`]s into transcript
//! segments — the shared shape rendered for both conversation turns and run
//! nodes. The projection mutates only a top-level `"segments"` array on the
//! supplied container and threads timestamps in from the caller, so replaying
//! identical events with identical timestamps is deterministic (chat passes
//! wall-clock; run journals pass each event's `emitted_at`).

use serde_json::{json, Map, Value};

use crate::events::{TurnStreamChannel, TurnStreamEvent, TurnStreamEventKind};

pub fn find_segment<'a>(snapshot: &'a Value, segment_id: &str) -> Option<&'a Value> {
    snapshot
        .get("segments")
        .and_then(Value::as_array)?
        .iter()
        .find(|segment| segment.get("id").and_then(Value::as_str) == Some(segment_id))
}

pub fn upsert_segment(snapshot: &mut Value, segment: Value) {
    let segment_id = segment.get("id").and_then(Value::as_str).unwrap_or("");
    if segment_id.is_empty() {
        return;
    }
    if let Some(segments) = snapshot
        .as_object_mut()
        .and_then(|object| object.get_mut("segments"))
        .and_then(Value::as_array_mut)
    {
        if let Some(existing) = segments
            .iter_mut()
            .find(|candidate| candidate.get("id").and_then(Value::as_str) == Some(segment_id))
        {
            *existing = segment;
        } else {
            segments.push(segment);
        }
    } else if let Some(object) = snapshot.as_object_mut() {
        object.insert("segments".to_string(), json!([segment]));
    }
}

pub fn next_turn_segment_order(snapshot: &Value, turn_id: &str) -> i64 {
    snapshot
        .get("segments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|segment| segment.get("turn_id").and_then(Value::as_str) == Some(turn_id))
        .filter_map(|segment| segment.get("order").and_then(Value::as_i64))
        .max()
        .unwrap_or(0)
        + 1
}

pub fn set_value(target: &mut Value, key: &str, value: Value) {
    if let Some(object) = target.as_object_mut() {
        object.insert(key.to_string(), value);
    }
}

pub fn set_string_value(target: &mut Value, key: &str, value: &str) {
    set_value(target, key, json!(value));
}

pub fn remove_key(target: &mut Value, key: &str) {
    if let Some(object) = target.as_object_mut() {
        object.remove(key);
    }
}

pub fn non_empty_string(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub fn truncate_utf8(value: &str, byte_limit: usize) -> (String, bool) {
    if value.len() <= byte_limit {
        return (value.to_string(), false);
    }
    let mut end = 0;
    for (index, ch) in value.char_indices() {
        let next = index + ch.len_utf8();
        if next > byte_limit {
            break;
        }
        end = next;
    }
    (value[..end].to_string(), true)
}

pub fn normalize_assistant_text(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn event_text(event: &TurnStreamEvent) -> Option<String> {
    event
        .content_delta
        .as_deref()
        .and_then(non_empty_string)
        .or_else(|| event.message.as_deref().and_then(non_empty_string))
}

// Turn-content fallback for ContentCompleted events without text. Containers
// without a "turns" array (run-node projections) make this a no-op.
fn find_turn<'a>(snapshot: &'a Value, turn_id: &str) -> Option<&'a Value> {
    snapshot
        .get("turns")
        .and_then(Value::as_array)?
        .iter()
        .find(|turn| turn.get("id").and_then(Value::as_str) == Some(turn_id))
}

pub fn materialize_segment_for_event(
    snapshot: &mut Value,
    assistant_turn_id: &str,
    event: &TurnStreamEvent,
    now: &str,
) -> Option<Value> {
    match &event.kind {
        TurnStreamEventKind::ContentDelta
            if event.channel == Some(TurnStreamChannel::Reasoning) =>
        {
            let segment_id = reasoning_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "reasoning",
                        "assistant",
                        "streaming",
                        now,
                        build_segment_source(event, None),
                    )
                });
            append_segment_content(&mut segment, event.content_delta.as_deref().unwrap_or(""));
            set_string_value(&mut segment, "status", "streaming");
            set_string_value(&mut segment, "updated_at", now);
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentDelta
            if event.channel == Some(TurnStreamChannel::Assistant) =>
        {
            let segment_id = assistant_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    let mut segment = segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "assistant_message",
                        "assistant",
                        "streaming",
                        now,
                        build_segment_source(event, None),
                    );
                    if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                        set_string_value(&mut segment, "phase", &phase);
                    }
                    segment
                });
            append_segment_content(&mut segment, event.content_delta.as_deref().unwrap_or(""));
            set_string_value(&mut segment, "status", "streaming");
            set_string_value(&mut segment, "updated_at", now);
            remove_key(&mut segment, "error");
            remove_key(&mut segment, "error_code");
            remove_key(&mut segment, "details");
            if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentDelta if event.channel == Some(TurnStreamChannel::Plan) => {
            let segment_id = plan_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "plan",
                        "assistant",
                        "streaming",
                        now,
                        build_segment_source(event, None),
                    )
                });
            append_segment_content(&mut segment, event.content_delta.as_deref().unwrap_or(""));
            set_string_value(&mut segment, "status", "streaming");
            set_string_value(&mut segment, "updated_at", now);
            remove_key(&mut segment, "error");
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentCompleted
            if event.channel == Some(TurnStreamChannel::Assistant) =>
        {
            let segment_id = assistant_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "assistant_message",
                        "assistant",
                        "complete",
                        now,
                        build_segment_source(event, None),
                    )
                });
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            } else if let Some(turn_content) = find_turn(snapshot, assistant_turn_id)
                .and_then(|turn| turn.get("content"))
                .and_then(Value::as_str)
                .and_then(non_empty_string)
            {
                set_string_value(&mut segment, "content", &turn_content);
            }
            set_string_value(&mut segment, "status", "complete");
            set_string_value(&mut segment, "updated_at", now);
            set_string_value(&mut segment, "completed_at", now);
            remove_key(&mut segment, "error");
            remove_key(&mut segment, "error_code");
            remove_key(&mut segment, "details");
            if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentCompleted
            if event.channel == Some(TurnStreamChannel::Reasoning) =>
        {
            let segment_id = reasoning_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "reasoning",
                        "assistant",
                        "complete",
                        now,
                        build_segment_source(event, None),
                    )
                });
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            }
            set_string_value(&mut segment, "status", "complete");
            set_string_value(&mut segment, "updated_at", now);
            set_string_value(&mut segment, "completed_at", now);
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContentCompleted if event.channel == Some(TurnStreamChannel::Plan) => {
            let segment_id = plan_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "plan",
                        "assistant",
                        "complete",
                        now,
                        build_segment_source(event, None),
                    )
                });
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            }
            set_string_value(&mut segment, "status", "complete");
            set_string_value(&mut segment, "updated_at", now);
            set_string_value(&mut segment, "completed_at", now);
            remove_key(&mut segment, "error");
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ContextCompactionStarted
        | TurnStreamEventKind::ContextCompactionCompleted => {
            let complete = event.kind == TurnStreamEventKind::ContextCompactionCompleted;
            let segment_id = context_compaction_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "context_compaction",
                        "system",
                        if complete { "complete" } else { "running" },
                        now,
                        build_segment_source(event, None),
                    )
                });
            set_string_value(
                &mut segment,
                "content",
                if complete {
                    "Context compacted to continue the turn."
                } else {
                    "Compacting conversation context..."
                },
            );
            set_string_value(
                &mut segment,
                "status",
                if complete { "complete" } else { "running" },
            );
            set_string_value(&mut segment, "updated_at", now);
            if complete {
                set_string_value(&mut segment, "completed_at", now);
            }
            remove_key(&mut segment, "error");
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::RequestUserInputRequested => {
            let request = event
                .request_user_input
                .as_ref()
                .and_then(normalize_request_user_input_payload)?;
            let segment_id = request_user_input_segment_id(assistant_turn_id, event, &request);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "request_user_input",
                        "system",
                        "pending",
                        now,
                        build_segment_source(event, None),
                    )
                });
            if segment.get("status").and_then(Value::as_str) == Some("complete") {
                return None;
            }
            set_string_value(&mut segment, "status", "pending");
            set_string_value(&mut segment, "updated_at", now);
            set_string_value(
                &mut segment,
                "content",
                &request_user_input_segment_content(&request),
            );
            remove_key(&mut segment, "completed_at");
            remove_key(&mut segment, "error");
            set_value(&mut segment, "request_user_input", Value::Object(request));
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::Other(kind) if is_model_tool_call_kind(kind) => {
            let tool_call = normalize_tool_call_payload(event.tool_call.as_ref()?);
            let segment_id = model_tool_segment_id(assistant_turn_id, event, &tool_call);
            let status = model_tool_call_status(kind, &tool_call);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "model_tool_call",
                        "assistant",
                        &status,
                        now,
                        build_segment_source(event, tool_call_id(&tool_call).as_deref()),
                    )
                });
            set_string_value(&mut segment, "status", &status);
            set_string_value(&mut segment, "updated_at", now);
            set_value(
                &mut segment,
                "source",
                build_segment_source(event, tool_call_id(&tool_call).as_deref()),
            );
            set_value(&mut segment, "tool_call", tool_call);
            if status == "complete" || status == "failed" {
                set_string_value(&mut segment, "completed_at", now);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::ToolCallStarted
        | TurnStreamEventKind::ToolCallUpdated
        | TurnStreamEventKind::ToolCallCompleted
        | TurnStreamEventKind::ToolCallFailed => {
            let tool_call = normalize_tool_call_payload(event.tool_call.as_ref()?);
            let segment_id = tool_segment_id(assistant_turn_id, event, &tool_call);
            let status = tool_call_status(event, &tool_call);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "tool_call",
                        "system",
                        &status,
                        now,
                        build_segment_source(event, tool_call_id(&tool_call).as_deref()),
                    )
                });
            set_string_value(&mut segment, "status", &status);
            set_string_value(&mut segment, "updated_at", now);
            set_value(&mut segment, "tool_call", tool_call);
            if status != "running" {
                set_string_value(&mut segment, "completed_at", now);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::TurnCompleted => {
            let segment_id = agent_event_segment_id(snapshot, assistant_turn_id, event);
            let mut segment = agent_event_segment_shell(
                &segment_id,
                assistant_turn_id,
                next_turn_segment_order(snapshot, assistant_turn_id),
                "processing",
                "complete",
                now,
                event,
            );
            if let Some(status) = event.status.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "event_status", &status);
            }
            if let Some(phase) = event.phase.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::Other(kind) if is_agent_event_kind(kind) => {
            let segment_id = agent_event_segment_id(snapshot, assistant_turn_id, event);
            let (category, status) = match kind.as_str() {
                "session_start" => ("lifecycle", "running"),
                "session_end" => ("lifecycle", "complete"),
                "warning" => ("warning", "complete"),
                _ => ("session", "complete"),
            };
            let mut segment = agent_event_segment_shell(
                &segment_id,
                assistant_turn_id,
                next_turn_segment_order(snapshot, assistant_turn_id),
                category,
                status,
                now,
                event,
            );
            if let Some(event_status) = event.status.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "event_status", &event_status);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        TurnStreamEventKind::Error => {
            let segment_id = assistant_segment_id(assistant_turn_id, event);
            let mut segment = find_segment(snapshot, &segment_id)
                .cloned()
                .unwrap_or_else(|| {
                    segment_shell(
                        &segment_id,
                        assistant_turn_id,
                        next_turn_segment_order(snapshot, assistant_turn_id),
                        "assistant_message",
                        "assistant",
                        "failed",
                        now,
                        build_segment_source(event, None),
                    )
                });
            let message = event
                .error
                .clone()
                .or_else(|| event.message.clone())
                .unwrap_or_else(|| "Conversation turn failed.".to_string());
            if let Some(text) = event_text(event) {
                set_string_value(&mut segment, "content", &text);
            }
            set_string_value(&mut segment, "status", "failed");
            set_string_value(&mut segment, "error", &message);
            if let Some(error_code) = event.error_code.as_deref().and_then(non_empty_string) {
                set_string_value(&mut segment, "error_code", &error_code);
            }
            if let Some(details) = event.details.as_ref() {
                set_value(&mut segment, "details", details.clone());
            }
            set_string_value(&mut segment, "updated_at", now);
            set_string_value(&mut segment, "completed_at", now);
            if let Some(phase) = normalized_assistant_phase(event.phase.as_deref()) {
                set_string_value(&mut segment, "phase", &phase);
            }
            upsert_segment(snapshot, segment.clone());
            Some(segment)
        }
        _ => None,
    }
}

pub fn should_emit_segment_upsert_for_event(event: &TurnStreamEvent) -> bool {
    match &event.kind {
        TurnStreamEventKind::ContentDelta | TurnStreamEventKind::ToolCallUpdated => false,
        TurnStreamEventKind::Other(kind) if kind == "model_tool_call_delta" => false,
        _ => true,
    }
}

pub fn is_agent_event_kind(kind: &str) -> bool {
    matches!(kind, "session_start" | "session_end" | "warning")
}

pub fn agent_event_segment_shell(
    segment_id: &str,
    assistant_turn_id: &str,
    order: i64,
    category: &str,
    status: &str,
    timestamp: &str,
    event: &TurnStreamEvent,
) -> Value {
    let mut segment = segment_shell(
        segment_id,
        assistant_turn_id,
        order,
        "agent_event",
        "system",
        status,
        timestamp,
        build_segment_source(event, None),
    );
    let event_kind = event
        .source
        .raw_kind
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| event.kind.as_str().to_string());
    set_string_value(&mut segment, "event_kind", &event_kind);
    set_string_value(&mut segment, "category", category);
    if let Some(message) = event.message.as_deref().and_then(non_empty_string) {
        set_string_value(&mut segment, "content", &message);
        set_string_value(&mut segment, "message", &message);
    } else {
        set_string_value(&mut segment, "content", &event_kind);
    }
    if let Some(details) = event.details.as_ref() {
        set_value(&mut segment, "details", details.clone());
    }
    set_string_value(&mut segment, "updated_at", timestamp);
    set_string_value(&mut segment, "completed_at", timestamp);
    segment
}

#[allow(clippy::too_many_arguments)]
pub fn segment_shell(
    segment_id: &str,
    turn_id: &str,
    order: i64,
    kind: &str,
    role: &str,
    status: &str,
    timestamp: &str,
    source: Value,
) -> Value {
    json!({
        "id": segment_id,
        "turn_id": turn_id,
        "order": order,
        "kind": kind,
        "role": role,
        "status": status,
        "timestamp": timestamp,
        "updated_at": timestamp,
        "content": "",
        "source": source,
    })
}

pub fn append_segment_content(segment: &mut Value, delta: &str) {
    if delta.is_empty() {
        return;
    }
    let mut content = segment
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    content.push_str(delta);
    set_string_value(segment, "content", &content);
}

pub fn build_segment_source(event: &TurnStreamEvent, call_id: Option<&str>) -> Value {
    let mut source = Map::new();
    if let Some(value) = event
        .source
        .backend
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("backend".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .session_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("session_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .app_thread_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("app_thread_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .app_turn_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("app_turn_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .item_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("item_id".to_string(), json!(value));
    }
    if let Some(value) = event.source.summary_index {
        source.insert("summary_index".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .response_id
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("response_id".to_string(), json!(value));
    }
    if let Some(value) = event
        .source
        .raw_kind
        .as_ref()
        .and_then(|value| non_empty_string(value))
    {
        source.insert("raw_kind".to_string(), json!(value));
    }
    if let Some(value) = call_id.and_then(non_empty_string) {
        source.insert("call_id".to_string(), json!(value));
    }
    Value::Object(source)
}

pub fn reasoning_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let item_id = event
        .source
        .item_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| "reasoning".to_string());
    let summary_index = event.source.summary_index.unwrap_or(0);
    format!("segment-reasoning-{app_turn_id}-{item_id}-{summary_index}")
}

pub fn assistant_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    match (
        event
            .source
            .app_turn_id
            .as_deref()
            .and_then(non_empty_string),
        event.source.item_id.as_deref().and_then(non_empty_string),
    ) {
        (Some(app_turn_id), Some(item_id)) => format!("segment-assistant-{app_turn_id}-{item_id}"),
        _ => format!("segment-assistant-{turn_id}"),
    }
}

pub fn plan_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    match (
        event
            .source
            .app_turn_id
            .as_deref()
            .and_then(non_empty_string),
        event.source.item_id.as_deref().and_then(non_empty_string),
    ) {
        (Some(app_turn_id), Some(item_id)) => format!("segment-plan-{app_turn_id}-{item_id}"),
        _ => format!("segment-plan-{turn_id}"),
    }
}

pub fn agent_event_segment_id(snapshot: &Value, turn_id: &str, event: &TurnStreamEvent) -> String {
    let kind = event
        .source
        .raw_kind
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| event.kind.as_str().to_string());
    let sequence = next_turn_segment_order(snapshot, turn_id);
    match (
        event
            .source
            .app_turn_id
            .as_deref()
            .and_then(non_empty_string),
        event.source.item_id.as_deref().and_then(non_empty_string),
    ) {
        (Some(app_turn_id), Some(item_id)) => {
            format!("segment-agent-event-{app_turn_id}-{kind}-{item_id}")
        }
        _ => format!("segment-agent-event-{turn_id}-{kind}-{sequence}"),
    }
}

pub fn context_compaction_segment_id(turn_id: &str, event: &TurnStreamEvent) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    format!("segment-context-compaction-{app_turn_id}")
}

pub fn tool_segment_id(turn_id: &str, event: &TurnStreamEvent, tool_call: &Value) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let call_id = tool_call_id(tool_call)
        .or_else(|| event.source.item_id.as_deref().and_then(non_empty_string))
        .unwrap_or_else(|| "tool".to_string());
    format!("segment-tool-{app_turn_id}-{call_id}")
}

pub fn model_tool_segment_id(turn_id: &str, event: &TurnStreamEvent, tool_call: &Value) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let call_id = tool_call_id(tool_call)
        .or_else(|| event.source.item_id.as_deref().and_then(non_empty_string))
        .unwrap_or_else(|| "model-tool".to_string());
    format!("segment-model-tool-{app_turn_id}-{call_id}")
}

pub fn request_user_input_segment_id(
    turn_id: &str,
    event: &TurnStreamEvent,
    request: &Map<String, Value>,
) -> String {
    let app_turn_id = event
        .source
        .app_turn_id
        .as_deref()
        .and_then(non_empty_string)
        .unwrap_or_else(|| turn_id.to_string());
    let request_id = request
        .get("request_id")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| event.source.item_id.as_deref().and_then(non_empty_string))
        .unwrap_or_else(|| "request".to_string());
    format!("segment-request-user-input-{app_turn_id}-{request_id}")
}

/// Canonicalizes a tool-call payload for display. rust_llm payloads already
/// carry `title`/`kind`/`file_paths` and pass through untouched; raw codex
/// app-server items (`type`/`command`/`aggregatedOutput`) gain the canonical
/// display fields without losing any raw keys.
pub fn normalize_tool_call_payload(raw: &Value) -> Value {
    let Some(object) = raw.as_object() else {
        return raw.clone();
    };
    if object.contains_key("title") {
        return raw.clone();
    }
    let mut out = object.clone();
    let item_type = object
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !out.contains_key("kind") {
        let kind = match item_type {
            "commandExecution" => "command_execution",
            "fileChange" => "file_change",
            _ => "dynamic_tool",
        };
        out.insert("kind".to_string(), json!(kind));
    }
    let command = object
        .get("command")
        .and_then(Value::as_str)
        .and_then(non_empty_string);
    let title = command
        .as_deref()
        .and_then(|command| command.lines().next())
        .map(str::trim)
        .and_then(non_empty_string)
        .or_else(|| {
            object
                .get("name")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
        })
        .or_else(|| non_empty_string(item_type))
        .unwrap_or_else(|| "Tool call".to_string());
    out.insert("title".to_string(), json!(title));
    if !out.contains_key("output") {
        if let Some(output) = object
            .get("aggregatedOutput")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            out.insert("output".to_string(), json!(output));
        }
    }
    if !out.contains_key("file_paths") {
        let paths: Vec<String> = object
            .get("changes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|change| {
                change
                    .get("path")
                    .and_then(Value::as_str)
                    .and_then(non_empty_string)
                    .or_else(|| change.as_str().and_then(non_empty_string))
            })
            .collect();
        out.insert("file_paths".to_string(), json!(paths));
    }
    if let Some(status) = object.get("status").and_then(Value::as_str) {
        let normalized = match status {
            "in_progress" | "inProgress" | "started" | "pending" => Some("running"),
            "declined" | "errored" => Some("failed"),
            _ => None,
        };
        if let Some(normalized) = normalized {
            out.insert("status".to_string(), json!(normalized));
        }
    }
    Value::Object(out)
}

pub fn tool_call_id(tool_call: &Value) -> Option<String> {
    tool_call
        .get("id")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
}

pub fn tool_call_status(event: &TurnStreamEvent, tool_call: &Value) -> String {
    let raw_status = tool_call
        .get("status")
        .and_then(Value::as_str)
        .and_then(non_empty_string)
        .or_else(|| event.status.as_deref().and_then(non_empty_string));
    if let Some(raw_status) = raw_status {
        return match raw_status.as_str() {
            "started" | "updated" | "pending" | "streaming" => "running".to_string(),
            "completed" => "complete".to_string(),
            other => other.to_string(),
        };
    }
    match &event.kind {
        TurnStreamEventKind::ToolCallStarted | TurnStreamEventKind::ToolCallUpdated => {
            "running".to_string()
        }
        TurnStreamEventKind::ToolCallFailed => "failed".to_string(),
        _ => "complete".to_string(),
    }
}

pub fn is_model_tool_call_kind(kind: &str) -> bool {
    matches!(
        kind,
        "model_tool_call_start" | "model_tool_call_delta" | "model_tool_call_end"
    )
}

pub fn model_tool_call_status(kind: &str, tool_call: &Value) -> String {
    let raw_status = tool_call
        .get("status")
        .and_then(Value::as_str)
        .and_then(non_empty_string);
    if let Some(raw_status) = raw_status {
        return match raw_status.as_str() {
            "completed" | "complete" => "complete".to_string(),
            "failed" => "failed".to_string(),
            _ => "running".to_string(),
        };
    }
    match kind {
        "model_tool_call_end" => "complete".to_string(),
        _ => "running".to_string(),
    }
}

pub fn normalized_assistant_phase(phase: Option<&str>) -> Option<String> {
    phase.and_then(non_empty_string).map(|value| {
        value
            .trim()
            .to_lowercase()
            .replace('-', "_")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("_")
    })
}

pub fn normalize_request_user_input_payload(payload: &Value) -> Option<Map<String, Value>> {
    let object = payload.as_object()?;
    let request_id = object
        .get("request_id")
        .or_else(|| object.get("itemId"))
        .and_then(Value::as_str)
        .and_then(non_empty_string)?;
    let raw_questions = object.get("questions").and_then(Value::as_array)?;
    let mut questions = Vec::new();
    for (index, raw_question) in raw_questions.iter().enumerate() {
        let Some(question) = raw_question.as_object() else {
            continue;
        };
        let prompt = question
            .get("question")
            .and_then(Value::as_str)
            .and_then(non_empty_string);
        let Some(prompt) = prompt else {
            continue;
        };
        let options = question
            .get("options")
            .and_then(Value::as_array)
            .map(|options| {
                options
                    .iter()
                    .filter_map(|option| {
                        let option = option.as_object()?;
                        let label = option
                            .get("label")
                            .and_then(Value::as_str)
                            .and_then(non_empty_string)?;
                        let mut output = Map::new();
                        output.insert("label".to_string(), json!(label));
                        if let Some(description) = option
                            .get("description")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                        {
                            output.insert("description".to_string(), json!(description));
                        }
                        Some(Value::Object(output))
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let question_type = question
            .get("question_type")
            .and_then(Value::as_str)
            .and_then(non_empty_string)
            .unwrap_or_else(|| {
                if options.is_empty() {
                    "FREEFORM".to_string()
                } else {
                    "MULTIPLE_CHOICE".to_string()
                }
            });
        questions.push(json!({
            "id": question
                .get("id")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
                .unwrap_or_else(|| format!("question-{}", index + 1)),
            "header": question
                .get("header")
                .and_then(Value::as_str)
                .and_then(non_empty_string)
                .unwrap_or_else(|| format!("Question {}", index + 1)),
            "question": prompt,
            "question_type": question_type,
            "options": options,
            "allow_other": question
                .get("allow_other")
                .or_else(|| question.get("isOther"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "is_secret": question
                .get("is_secret")
                .or_else(|| question.get("isSecret"))
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }));
    }
    if questions.is_empty() {
        return None;
    }
    let answers = object
        .get("answers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let mut output = Map::new();
    output.insert("request_id".to_string(), json!(request_id));
    output.insert(
        "status".to_string(),
        json!(normalize_request_user_input_status(
            object.get("status").and_then(Value::as_str)
        )),
    );
    output.insert("questions".to_string(), Value::Array(questions));
    output.insert("answers".to_string(), Value::Object(answers));
    for key in ["app_thread_id", "app_turn_id"] {
        if let Some(value) = object
            .get(key)
            .and_then(Value::as_str)
            .and_then(non_empty_string)
        {
            output.insert(key.to_string(), json!(value));
        }
    }
    if let Some(submitted_at) = object.get("submitted_at").and_then(Value::as_str) {
        output.insert("submitted_at".to_string(), json!(submitted_at));
    }
    Some(output)
}

pub fn normalize_request_user_input_status(status: Option<&str>) -> &'static str {
    match status {
        Some("answered") => "answered",
        Some("expired") => "expired",
        _ => "pending",
    }
}

pub fn request_user_input_segment_content(request: &Map<String, Value>) -> String {
    match request.get("status").and_then(Value::as_str) {
        Some("answered" | "expired") => request_user_input_answer_summary(request),
        _ => request_user_input_prompt_summary(request),
    }
}

pub fn request_user_input_prompt_summary(request: &Map<String, Value>) -> String {
    let prompts = request
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            question
                .get("question")
                .and_then(Value::as_str)
                .map(normalize_assistant_text)
                .filter(|value| !value.is_empty())
        })
        .collect::<Vec<_>>();
    match prompts.len() {
        0 => "User input requested.".to_string(),
        1 => prompts[0].clone(),
        count => format!("{count} questions need user input."),
    }
}

pub fn request_user_input_answer_summary(request: &Map<String, Value>) -> String {
    let answers = request
        .get("answers")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let lines = request
        .get("questions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|question| {
            let prompt = question
                .get("question")
                .and_then(Value::as_str)
                .map(normalize_assistant_text)
                .unwrap_or_default();
            let question_id = question.get("id").and_then(Value::as_str)?;
            let answer = answers
                .get(question_id)
                .and_then(Value::as_str)
                .map(normalize_assistant_text)
                .unwrap_or_default();
            (!prompt.is_empty() && !answer.is_empty())
                .then(|| format!("{prompt}\nAnswer: {answer}"))
        })
        .collect::<Vec<_>>();
    if lines.is_empty() {
        request_user_input_prompt_summary(request)
    } else {
        lines.join("\n\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant_delta(text: &str, item_id: Option<&str>) -> TurnStreamEvent {
        let mut event = TurnStreamEvent::content_delta(TurnStreamChannel::Assistant, text);
        event.source.item_id = item_id.map(str::to_string);
        event
    }

    #[test]
    fn replaying_identical_events_and_timestamps_is_deterministic() {
        let events = vec![
            assistant_delta("Hello", None),
            assistant_delta(" world", None),
        ];
        let project = || {
            let mut container = serde_json::json!({});
            for (index, event) in events.iter().enumerate() {
                let now = format!("2026-07-08T10:00:{:02}Z", index);
                materialize_segment_for_event(&mut container, "turn-1", event, &now);
            }
            container
        };
        assert_eq!(project(), project());
    }

    #[test]
    fn segment_orders_are_scoped_per_turn_id() {
        let mut container = serde_json::json!({});
        materialize_segment_for_event(
            &mut container,
            "turn-a",
            &assistant_delta("a", Some("item-a")),
            "2026-07-08T10:00:00Z",
        );
        materialize_segment_for_event(
            &mut container,
            "turn-b",
            &assistant_delta("b", Some("item-b")),
            "2026-07-08T10:00:01Z",
        );
        let segments = container["segments"].as_array().expect("segments");
        assert_eq!(segments.len(), 2);
        // Each turn gets its own order counter starting at 1.
        assert_eq!(segments[0]["order"], 1);
        assert_eq!(segments[1]["order"], 1);
    }
}
