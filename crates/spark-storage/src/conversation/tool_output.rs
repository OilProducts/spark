//! Tool-output externalization.
//!
//! Durable transcripts store a bounded tool-output preview; full outputs live
//! in immutable blobs under `conversations/<id>/tool-output/`, written and
//! synced before the transaction that references them and fetched on demand.
//! Each output version gets its own file name, so bytes a committed segment
//! references are never overwritten.

use serde_json::{json, Value};

use crate::error::Result;
use crate::write_text_atomic;

use super::records::TranscriptSegment;

/// Largest tool output stored inline in the durable transcript. Larger outputs
/// are externalized and the inline value becomes a preview of this size.
pub const TOOL_OUTPUT_INLINE_LIMIT_BYTES: usize = 8 * 1024;
const TOOL_OUTPUT_DIR_NAME: &str = "tool-output";

/// Externalize an oversized tool output from `segment` into a new blob.
///
/// No-ops when the segment has no string output, the output fits inline, the
/// output is already a preview (`output_truncated`), or the segment id is not
/// filesystem-safe.
pub(crate) fn externalize_segment_tool_output(
    root: &std::path::Path,
    segment: &mut TranscriptSegment,
) -> Result<()> {
    let Some(tool_call) = segment.tool_call.as_mut().and_then(Value::as_object_mut) else {
        return Ok(());
    };
    externalize_tool_output(root, &segment.id, tool_call)
}

/// Run transcripts: one file per segment, rewritten in place.
pub(crate) fn externalize_tool_output(
    root: &std::path::Path,
    artifact_id: &str,
    tool_call: &mut serde_json::Map<String, Value>,
) -> Result<()> {
    externalize(root, artifact_id, tool_call, false)
}

/// Conversations: a new immutable file per output version.
pub(crate) fn externalize_conversation_segment_tool_output(
    root: &std::path::Path,
    segment: &mut TranscriptSegment,
) -> Result<()> {
    let Some(tool_call) = segment.tool_call.as_mut().and_then(Value::as_object_mut) else {
        return Ok(());
    };
    externalize(root, &segment.id, tool_call, true)
}

pub(crate) fn externalize_conversation_tool_output(
    root: &std::path::Path,
    artifact_id: &str,
    tool_call: &mut serde_json::Map<String, Value>,
) -> Result<()> {
    externalize(root, artifact_id, tool_call, true)
}

fn externalize(
    root: &std::path::Path,
    artifact_id: &str,
    tool_call: &mut serde_json::Map<String, Value>,
    versioned: bool,
) -> Result<()> {
    if tool_call.get("output_truncated").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let Some(output) = tool_call.get("output").and_then(Value::as_str) else {
        return Ok(());
    };
    if output.len() <= TOOL_OUTPUT_INLINE_LIMIT_BYTES || !is_safe_segment_file_id(artifact_id) {
        return Ok(());
    }
    let output = output.to_string();
    let artifact = if versioned {
        format!(
            "{TOOL_OUTPUT_DIR_NAME}/{artifact_id}.{}.txt",
            uuid::Uuid::new_v4().simple()
        )
    } else {
        format!("{TOOL_OUTPUT_DIR_NAME}/{artifact_id}.txt")
    };
    write_text_atomic(root.join(&artifact), &output)?;
    let (preview, _) = truncate_utf8(&output, TOOL_OUTPUT_INLINE_LIMIT_BYTES);
    tool_call.insert("output".to_string(), json!(preview));
    tool_call.insert("output_size".to_string(), json!(output.len()));
    tool_call.insert("output_truncated".to_string(), json!(true));
    tool_call.insert("output_artifact".to_string(), json!(artifact));
    Ok(())
}

/// A stored `output_artifact` path when it names a file directly inside the
/// conversation's tool-output directory.
pub(crate) fn safe_tool_output_artifact(artifact: &str) -> Option<&str> {
    let name = artifact
        .strip_prefix(TOOL_OUTPUT_DIR_NAME)?
        .strip_prefix('/')?;
    is_safe_segment_file_id(name).then_some(artifact)
}

/// Guard sidecar file names against path traversal. Segment ids are generated
/// from safe alphabets today; anything else stays inline.
pub(crate) fn is_safe_segment_file_id(segment_id: &str) -> bool {
    !segment_id.is_empty()
        && !segment_id.starts_with('.')
        && !segment_id.contains("..")
        && segment_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
}

/// Truncate to a UTF-8-safe prefix of at most `byte_limit` bytes. Returns the
/// prefix and whether truncation occurred.
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
