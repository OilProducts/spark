//! Conversation attachments on agent turns. The current turn's attachments
//! ride `AgentTurnRequest::metadata`; earlier ones ride the user turns in
//! `history`, so a fresh provider session still learns about them.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::history::{HistoryTurn, TurnContent};
use unified_llm_adapter::{ContentPart, ImageData};

/// Metadata key holding the current turn's `Vec<AgentAttachment>`.
pub const AGENT_ATTACHMENTS_METADATA_KEY: &str = "spark.agent.attachments";

/// Image formats Codex and Claude Code both accept as native image input.
pub const NATIVE_IMAGE_MEDIA_TYPES: [&str; 4] =
    ["image/png", "image/jpeg", "image/gif", "image/webp"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAttachment {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: u64,
    /// Absolute host path of the stored bytes.
    pub path: String,
}

impl AgentAttachment {
    pub fn is_native_image(&self) -> bool {
        NATIVE_IMAGE_MEDIA_TYPES.contains(&self.media_type.as_str())
    }

    fn reference_line(&self) -> String {
        format!(
            "- {} ({}, {} bytes): {}",
            self.filename, self.media_type, self.size_bytes, self.path
        )
    }
}

pub fn request_attachments(metadata: &BTreeMap<String, Value>) -> Vec<AgentAttachment> {
    metadata
        .get(AGENT_ATTACHMENTS_METADATA_KEY)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

pub fn history_attachments(history: &[HistoryTurn]) -> Vec<AgentAttachment> {
    history
        .iter()
        .flat_map(|turn| match turn {
            HistoryTurn::User(turn) => turn.attachments.clone(),
            _ => Vec::new(),
        })
        .collect()
}

pub(crate) const ATTACHED_FILES_HEADING: &str =
    "Attached files (local copies; open them with your file-reading tools):";
const EARLIER_FILES_HEADING: &str = "Files attached earlier in this conversation (local copies):";

/// The turn text a provider receives: the prompt, then local references for
/// attachments that are not native images, then (for a fresh provider
/// session) every attachment from earlier turns. Text-only turns are
/// returned unchanged.
pub fn prompt_with_attachment_references(
    prompt: &str,
    attachments: &[AgentAttachment],
    earlier: &[AgentAttachment],
) -> String {
    let files = attachments
        .iter()
        .filter(|attachment| !attachment.is_native_image())
        .collect::<Vec<_>>();
    let mut text = prompt.to_string();
    for (heading, entries) in [
        (ATTACHED_FILES_HEADING, files),
        (EARLIER_FILES_HEADING, earlier.iter().collect()),
    ] {
        if entries.is_empty() {
            continue;
        }
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        text.push_str(&reference_block(heading, entries));
    }
    text
}

/// A message with every attachment as a local file reference, for agents
/// that take text only, such as a message delivered into a running turn.
pub fn message_with_attachment_references(
    message: &str,
    attachments: &[AgentAttachment],
) -> String {
    if attachments.is_empty() {
        return message.to_string();
    }
    let block = reference_block(ATTACHED_FILES_HEADING, attachments);
    if message.is_empty() {
        block
    } else {
        format!("{message}\n\n{block}")
    }
}

/// A native turn's input: every attachment as a file reference, which any
/// model can open with its tools, plus image parts for a vision model.
pub fn native_turn_input(
    prompt: &str,
    attachments: &[AgentAttachment],
    vision: bool,
) -> TurnContent {
    let text = message_with_attachment_references(prompt, attachments);
    let images = attachments
        .iter()
        .filter(|attachment| vision && attachment.is_native_image())
        .filter_map(|attachment| {
            let bytes = std::fs::read(&attachment.path).ok()?;
            Some(ContentPart::Image {
                image: ImageData::data(bytes, Some(attachment.media_type.clone())),
            })
        })
        .collect::<Vec<_>>();
    if images.is_empty() {
        return TurnContent::Text(text);
    }
    let mut parts = vec![ContentPart::text(text)];
    parts.extend(images);
    TurnContent::Parts(parts)
}

pub(crate) fn reference_block<'a>(
    heading: &str,
    attachments: impl IntoIterator<Item = &'a AgentAttachment>,
) -> String {
    let mut text = heading.to_string();
    for attachment in attachments {
        text.push('\n');
        text.push_str(&attachment.reference_line());
    }
    text
}
