//! Read-time projection of a run's journaled agent stream into transcript
//! segments — the same provider-neutral shape conversations render, derived
//! deterministically from the combined (parent + child) journal.
//!
//! Every codergen backend journals its `TurnStreamEvent`s inside
//! `CodergenAdapter` events (`payload.turn_stream_event`), regardless of
//! provider; content deltas additionally surface as flat `LLMContent` entries
//! that carry the same `turn_stream_event` passthrough. Replaying those events
//! through `spark_common::segments` with `now = emitted_at` yields identical
//! segments on every read.

use std::collections::BTreeMap;

use attractor_core::JournalEntry;
use serde_json::{json, Value};
use spark_common::events::TurnStreamEvent;
use spark_common::segments::{materialize_segment_for_event, set_value, upsert_segment};

#[derive(Debug, Clone, Default)]
pub struct RunSegmentProjection {
    /// Segments in first-touched order, each stamped with `node_id`,
    /// `attempt`, `turn_id`, source metadata, and `latest_sequence` (the
    /// combined-journal sequence of the last event that touched it).
    pub segments: Vec<Value>,
    /// Highest combined-journal sequence consumed, for cursor alignment.
    pub newest_sequence: u64,
}

/// Scope key for attempt tracking: child runs restart their own counters.
fn scope_run_id(entry: &JournalEntry) -> String {
    entry
        .payload
        .get("source_run_id")
        .and_then(Value::as_str)
        .unwrap_or("root")
        .to_string()
}

fn turn_stream_event_value(entry: &JournalEntry) -> Option<&Value> {
    // Flat LLMContent entries carry the passthrough at the top level; raw
    // CodergenAdapter entries nest it inside the adapter payload.
    entry.payload.get("turn_stream_event").or_else(|| {
        entry
            .payload
            .get("payload")
            .and_then(|payload| payload.get("turn_stream_event"))
    })
}

pub fn project_run_segments(entries: &[JournalEntry]) -> RunSegmentProjection {
    let mut state = SegmentProjectionState::default();
    let mut replay = entries.to_vec();
    replay.sort_by(|left, right| left.sequence.cmp(&right.sequence));
    for entry in &replay {
        state.apply(entry);
    }
    state.snapshot()
}

/// Incrementally maintained projection state: `apply` consumes journal
/// entries in combined-sequence order, so a cached consumer replays only
/// newly appended entries instead of the whole journal per read.
#[derive(Debug, Default)]
pub struct SegmentProjectionState {
    container: Option<Value>,
    attempts: BTreeMap<(String, String), u64>,
    order: Vec<String>,
    stamped: BTreeMap<String, Value>,
    newest_sequence: u64,
}

impl SegmentProjectionState {
    pub fn snapshot(&self) -> RunSegmentProjection {
        RunSegmentProjection {
            segments: self
                .order
                .iter()
                .filter_map(|segment_id| self.stamped.get(segment_id).cloned())
                .collect(),
            newest_sequence: self.newest_sequence,
        }
    }

    /// Segments whose latest touch is past the cursor, cloned for callers.
    pub fn segments_touched_after(&self, after: u64) -> Vec<Value> {
        self.order
            .iter()
            .filter_map(|segment_id| self.stamped.get(segment_id))
            .filter(|segment| {
                segment
                    .get("latest_sequence")
                    .and_then(Value::as_u64)
                    .is_some_and(|sequence| sequence > after)
            })
            .cloned()
            .collect()
    }

    pub fn apply(&mut self, entry: &JournalEntry) {
        let container = self.container.get_or_insert_with(|| json!({}));
        let attempts = &mut self.attempts;
        let order = &mut self.order;
        let stamped = &mut self.stamped;
        let newest_sequence = &mut self.newest_sequence;
        *newest_sequence = (*newest_sequence).max(entry.sequence);

        if entry.raw_type == "StageRetrying" {
            if let Some(node_id) = entry.node_id.as_deref() {
                let key = (scope_run_id(entry), node_id.to_string());
                let attempt = entry
                    .payload
                    .get("attempt")
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| attempts.get(&key).copied().unwrap_or(0) + 1);
                attempts.insert(key, attempt);
            }
            return;
        }

        if !matches!(entry.raw_type.as_str(), "CodergenAdapter" | "LLMContent") {
            return;
        }
        let Some(stream_event_value) = turn_stream_event_value(entry) else {
            return;
        };
        let Ok(event) = serde_json::from_value::<TurnStreamEvent>(stream_event_value.clone())
        else {
            return;
        };
        let Some(node_id) = entry.node_id.clone() else {
            return;
        };
        let scope = scope_run_id(entry);
        let attempt = attempts
            .get(&(scope.clone(), node_id.clone()))
            .copied()
            .unwrap_or(0);
        let turn_id = format!("{scope}:{node_id}:attempt-{attempt}");

        let Some(mut segment) =
            materialize_segment_for_event(container, &turn_id, &event, &entry.emitted_at)
        else {
            return;
        };
        set_value(&mut segment, "node_id", json!(node_id));
        set_value(&mut segment, "attempt", json!(attempt));
        set_value(&mut segment, "latest_sequence", json!(entry.sequence));
        set_value(&mut segment, "source_scope", json!(entry.source_scope));
        set_value(
            &mut segment,
            "source_parent_node_id",
            entry
                .source_parent_node_id
                .as_ref()
                .map_or(Value::Null, |value| json!(value)),
        );
        set_value(
            &mut segment,
            "source_flow_name",
            entry
                .source_flow_name
                .as_ref()
                .map_or(Value::Null, |value| json!(value)),
        );
        set_value(
            &mut segment,
            "source_run_id",
            if scope == "root" {
                Value::Null
            } else {
                json!(scope)
            },
        );
        // Persist the stamped copy so the next touch of this segment starts
        // from stamped state.
        upsert_segment(container, segment.clone());

        let segment_id = segment
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if !stamped.contains_key(&segment_id) {
            order.push(segment_id.clone());
        }
        stamped.insert(segment_id, segment);
    }
}
