//! One loaded conversation's committed state, indexed by identity and order
//! so a commit touches only the entities it changes. Summary flags, the
//! preview and title candidates, launched runs and the in-flight turns are
//! maintained as entities change; nothing here scans history on a commit.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde_json::Value;

use super::attachments::ConversationAttachment;
use super::records::{
    ConversationArtifacts, ConversationMeta, ConversationRecord, Transcript, TranscriptSegment,
    TranscriptTurn,
};

/// Workflow events are artifact rows in this collection, in insertion order.
pub(crate) const EVENT_LOG_COLLECTION: &str = "event_log";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredAttachment {
    pub record: ConversationAttachment,
    /// Relative to the Spark home.
    pub blob_path: String,
    pub attached: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct StoredArtifact {
    /// Insertion order within its collection.
    pub position: i64,
    /// The turn the artifact belongs to, when known.
    pub owner: Option<String>,
    pub payload: Value,
}

/// Summary columns derived from the indexes.
pub(crate) struct Summary {
    pub preview: String,
    pub running: bool,
    pub needs_attention: bool,
    pub launched_run_ids: Vec<String>,
}

#[derive(Debug)]
pub(crate) struct LoadedConversation {
    pub meta: ConversationMeta,
    /// Last revision that changed any of this conversation's rows; the
    /// conversation revision clients and stale-settings checks compare.
    pub revision: i64,
    pub next_position: i64,
    pub applied_trace: HashMap<String, u64>,
    pub next_trace_sequence: u64,
    /// Newest durable trace sequence of each turn.
    pub trace_last_by_turn: HashMap<String, u64>,
    pub attachments: BTreeMap<String, StoredAttachment>,
    turns: HashMap<String, (i64, TranscriptTurn)>,
    by_position: BTreeMap<i64, String>,
    segments: HashMap<String, TranscriptSegment>,
    /// (order, segment id) per turn.
    turn_segments: HashMap<String, BTreeSet<(i64, String)>>,
    /// Keyed by (collection, artifact id).
    artifacts: HashMap<(String, String), StoredArtifact>,
    collection_order: HashMap<String, BTreeMap<i64, String>>,
    turn_artifacts: HashMap<String, BTreeSet<(String, String)>>,
    /// Artifact id to the turn of a segment anchoring it.
    anchors: HashMap<String, String>,
    /// Pending, streaming and queued turns by position.
    in_flight: BTreeMap<i64, String>,
    /// Assistant turns pending or streaming.
    running: BTreeSet<String>,
    /// Flow run requests pending and plans pending review.
    attention: BTreeSet<(String, String)>,
    /// Flow launch position to its run id.
    launched: BTreeMap<i64, String>,
    /// Newest message turn with preview text: (position, turn id).
    preview: Option<(i64, String)>,
    /// Oldest user message turn: (position, turn id).
    first_user: Option<(i64, String)>,
    completed_replies: usize,
}

pub(crate) fn is_active_status(status: &str) -> bool {
    matches!(status, "pending" | "streaming")
}

fn is_in_flight(turn: &TranscriptTurn) -> bool {
    is_active_status(&turn.status) || turn.status == "queued"
}

fn is_message(turn: &TranscriptTurn) -> bool {
    turn.turn_kind() == "message"
}

pub(crate) fn preview_text(turn: &TranscriptTurn) -> String {
    super::commit::truncate_text(&turn.content, 120)
}

fn needs_attention(collection: &str, artifact: &Value) -> bool {
    let status = artifact.get("status").and_then(Value::as_str);
    match collection {
        "flow_run_requests" => status == Some("pending"),
        "proposed_plans" => status == Some("pending_review"),
        _ => false,
    }
}

fn launched_run_id(collection: &str, artifact: &Value) -> Option<String> {
    if collection != "flow_launches" {
        return None;
    }
    artifact
        .get("run_id")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|run_id| !run_id.is_empty())
        .map(str::to_string)
}

impl LoadedConversation {
    pub fn new(conversation_id: &str, project_path: &str) -> Self {
        Self {
            meta: ConversationMeta::new(conversation_id, project_path),
            revision: 0,
            next_position: 0,
            applied_trace: HashMap::new(),
            next_trace_sequence: 1,
            trace_last_by_turn: HashMap::new(),
            attachments: BTreeMap::new(),
            turns: HashMap::new(),
            by_position: BTreeMap::new(),
            segments: HashMap::new(),
            turn_segments: HashMap::new(),
            artifacts: HashMap::new(),
            collection_order: HashMap::new(),
            turn_artifacts: HashMap::new(),
            anchors: HashMap::new(),
            in_flight: BTreeMap::new(),
            running: BTreeSet::new(),
            attention: BTreeSet::new(),
            launched: BTreeMap::new(),
            preview: None,
            first_user: None,
            completed_replies: 0,
        }
    }

    pub fn turn(&self, turn_id: &str) -> Option<&TranscriptTurn> {
        self.turns.get(turn_id).map(|(_, turn)| turn)
    }

    pub fn position(&self, turn_id: &str) -> Option<i64> {
        self.turns.get(turn_id).map(|(position, _)| *position)
    }

    pub fn segment(&self, segment_id: &str) -> Option<&TranscriptSegment> {
        self.segments.get(segment_id)
    }

    pub fn artifact(&self, collection: &str, artifact_id: &str) -> Option<&StoredArtifact> {
        self.artifacts
            .get(&(collection.to_string(), artifact_id.to_string()))
    }

    /// The turn of a segment anchoring `artifact_id`, if any.
    pub fn anchor_turn(&self, artifact_id: &str) -> Option<&str> {
        self.anchors.get(artifact_id).map(String::as_str)
    }

    /// Next `order` within a turn: its max existing order + 1, starting at 1.
    pub fn next_segment_order(&self, turn_id: &str) -> i64 {
        self.turn_segments
            .get(turn_id)
            .and_then(|segments| segments.last())
            .map_or(0, |(order, _)| *order)
            + 1
    }

    /// One collection's artifacts in insertion order.
    pub fn artifacts_in<'a>(
        &'a self,
        collection: &'a str,
    ) -> impl Iterator<Item = (&'a str, &'a StoredArtifact)> + 'a {
        self.collection_order
            .get(collection)
            .into_iter()
            .flat_map(|order| order.values())
            .map(move |id| {
                let artifact = &self.artifacts[&(collection.to_string(), id.clone())];
                (id.as_str(), artifact)
            })
    }

    /// Completed assistant message turns.
    pub fn completed_replies(&self) -> usize {
        self.completed_replies
    }

    pub fn next_artifact_position(&self, collection: &str) -> i64 {
        self.collection_order
            .get(collection)
            .and_then(|order| order.keys().next_back())
            .map_or(1, |position| position + 1)
    }

    pub fn put_turn(
        &mut self,
        turn: TranscriptTurn,
        position: i64,
    ) -> Option<(i64, TranscriptTurn)> {
        let previous = self.remove_turn(&turn.id);
        let id = turn.id.clone();
        self.by_position.insert(position, id.clone());
        if is_in_flight(&turn) {
            self.in_flight.insert(position, id.clone());
        }
        if turn.role == "assistant" && is_active_status(&turn.status) {
            self.running.insert(id.clone());
        }
        if turn.role == "assistant" && is_message(&turn) && turn.status == "complete" {
            self.completed_replies += 1;
        }
        if is_message(&turn)
            && !preview_text(&turn).is_empty()
            && self.preview.as_ref().is_none_or(|(at, _)| position >= *at)
        {
            self.preview = Some((position, id.clone()));
        }
        if turn.role == "user"
            && is_message(&turn)
            && self
                .first_user
                .as_ref()
                .is_none_or(|(at, _)| position <= *at)
        {
            self.first_user = Some((position, id.clone()));
        }
        self.next_position = self.next_position.max(position + 1);
        self.turns.insert(id, (position, turn));
        previous
    }

    pub fn remove_turn(&mut self, turn_id: &str) -> Option<(i64, TranscriptTurn)> {
        let (position, turn) = self.turns.remove(turn_id)?;
        self.by_position.remove(&position);
        self.in_flight.remove(&position);
        self.running.remove(turn_id);
        if turn.role == "assistant" && is_message(&turn) && turn.status == "complete" {
            self.completed_replies -= 1;
        }
        // ponytail: losing the current candidate rescans turns newest/oldest
        // first; only a moved queued turn or a rolled-back commit does that.
        if self.preview.as_ref().is_some_and(|(_, id)| id == turn_id) {
            self.preview = self.by_position.iter().rev().find_map(|(at, id)| {
                let (_, turn) = &self.turns[id];
                (is_message(turn) && !preview_text(turn).is_empty()).then(|| (*at, id.clone()))
            });
        }
        if self
            .first_user
            .as_ref()
            .is_some_and(|(_, id)| id == turn_id)
        {
            self.first_user = self.by_position.iter().find_map(|(at, id)| {
                let (_, turn) = &self.turns[id];
                (turn.role == "user" && is_message(turn)).then(|| (*at, id.clone()))
            });
        }
        Some((position, turn))
    }

    pub fn put_segment(&mut self, segment: TranscriptSegment) -> Option<TranscriptSegment> {
        let previous = self.remove_segment(&segment.id);
        self.turn_segments
            .entry(segment.turn_id.clone())
            .or_default()
            .insert((segment.order, segment.id.clone()));
        if let Some(artifact_id) = segment.artifact_id.as_ref().filter(|id| !id.is_empty()) {
            self.anchors
                .insert(artifact_id.clone(), segment.turn_id.clone());
        }
        self.segments.insert(segment.id.clone(), segment);
        previous
    }

    pub fn remove_segment(&mut self, segment_id: &str) -> Option<TranscriptSegment> {
        let segment = self.segments.remove(segment_id)?;
        if let Some(segments) = self.turn_segments.get_mut(&segment.turn_id) {
            segments.remove(&(segment.order, segment.id.clone()));
        }
        if let Some(artifact_id) = segment.artifact_id.as_ref() {
            if self.anchors.get(artifact_id) == Some(&segment.turn_id) {
                self.anchors.remove(artifact_id);
            }
        }
        Some(segment)
    }

    pub fn put_artifact(
        &mut self,
        collection: &str,
        artifact_id: &str,
        artifact: StoredArtifact,
    ) -> Option<StoredArtifact> {
        let previous = self.remove_artifact(collection, artifact_id);
        let key = (collection.to_string(), artifact_id.to_string());
        self.collection_order
            .entry(collection.to_string())
            .or_default()
            .insert(artifact.position, artifact_id.to_string());
        if let Some(owner) = &artifact.owner {
            self.turn_artifacts
                .entry(owner.clone())
                .or_default()
                .insert(key.clone());
        }
        if needs_attention(collection, &artifact.payload) {
            self.attention.insert(key.clone());
        }
        if let Some(run_id) = launched_run_id(collection, &artifact.payload) {
            self.launched.insert(artifact.position, run_id);
        }
        self.artifacts.insert(key, artifact);
        previous
    }

    pub fn remove_artifact(
        &mut self,
        collection: &str,
        artifact_id: &str,
    ) -> Option<StoredArtifact> {
        let key = (collection.to_string(), artifact_id.to_string());
        let artifact = self.artifacts.remove(&key)?;
        if let Some(order) = self.collection_order.get_mut(collection) {
            order.remove(&artifact.position);
        }
        if let Some(owner) = &artifact.owner {
            if let Some(owned) = self.turn_artifacts.get_mut(owner) {
                owned.remove(&key);
            }
        }
        self.attention.remove(&key);
        if collection == "flow_launches" {
            self.launched.remove(&artifact.position);
        }
        Some(artifact)
    }

    pub fn summary(&self) -> Summary {
        Summary {
            preview: self
                .preview
                .as_ref()
                .map(|(_, id)| preview_text(&self.turns[id].1))
                .unwrap_or_default(),
            running: !self.running.is_empty(),
            needs_attention: !self.attention.is_empty(),
            launched_run_ids: self.launched.values().cloned().collect(),
        }
    }

    /// The title derived from the first user message.
    pub fn derived_title(&self) -> String {
        self.first_user
            .as_ref()
            .map(|(_, id)| super::commit::truncate_text(&self.turns[id].1.content, 64))
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| "New thread".to_string())
    }

    /// The complete record: only for explicit history and export consumers,
    /// never commits, listings or live notifications.
    pub fn record(&self) -> ConversationRecord {
        let turn_ids: Vec<&String> = self.by_position.values().collect();
        self.project(&turn_ids, self.artifacts.keys().cloned().collect())
    }

    /// The metadata, in-flight turns with their segments and artifacts, and
    /// artifacts awaiting review: what accepting a message or projecting a
    /// running turn reads, independent of history size.
    pub fn head(&self) -> ConversationRecord {
        let turn_ids: Vec<&String> = self.in_flight.values().collect();
        let mut artifacts: BTreeSet<(String, String)> = self.attention.clone();
        for turn_id in &turn_ids {
            if let Some(owned) = self.turn_artifacts.get(*turn_id) {
                artifacts.extend(owned.iter().cloned());
            }
        }
        self.project(&turn_ids, artifacts.into_iter().collect())
    }

    fn project(
        &self,
        turn_ids: &[&String],
        artifacts: Vec<(String, String)>,
    ) -> ConversationRecord {
        let mut meta = self.meta.clone();
        meta.revision = self.revision;
        let turns = turn_ids
            .iter()
            .map(|id| self.turns[*id].1.clone())
            .collect();
        let segments = turn_ids
            .iter()
            .filter_map(|id| self.turn_segments.get(*id))
            .flatten()
            .map(|(_, id)| self.segments[id].clone())
            .collect();
        let mut ordered: Vec<(&str, i64, &Value)> = artifacts
            .iter()
            .filter_map(|key| {
                let artifact = self.artifacts.get(key)?;
                Some((key.0.as_str(), artifact.position, &artifact.payload))
            })
            .collect();
        ordered.sort_by_key(|(collection, position, _)| (*collection, *position));
        let mut collections = ConversationArtifacts::default();
        for (collection, _, payload) in ordered {
            let target = match collection {
                EVENT_LOG_COLLECTION => &mut collections.event_log,
                "flow_run_requests" => &mut collections.flow_run_requests,
                "flow_launches" => &mut collections.flow_launches,
                "run_recoveries" => &mut collections.run_recoveries,
                "proposed_plans" => &mut collections.proposed_plans,
                _ => continue,
            };
            target.push(payload.clone());
        }
        ConversationRecord {
            meta,
            transcript: Transcript { turns, segments },
            artifacts: collections,
        }
    }
}
