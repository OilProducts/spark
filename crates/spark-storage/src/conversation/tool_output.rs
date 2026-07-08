//! Tool-output externalization.
//!
//! Durable transcripts store a bounded tool-output preview; full outputs live
//! in per-segment sidecar files under `conversations/<id>/tool-output/`,
//! written at the commit boundary (and during legacy migration) and fetched on
//! demand. This keeps `transcript.json` and journal lines small no matter how
//! large tool outputs get.

use serde_json::{json, Value};

use crate::error::Result;
use crate::write_text_atomic;

use super::records::TranscriptSegment;
use super::store::ConversationRecordPaths;

/// Largest tool output stored inline in the durable transcript. Larger outputs
/// are externalized and the inline value becomes a preview of this size.
pub const TOOL_OUTPUT_INLINE_LIMIT_BYTES: usize = 8 * 1024;

/// Externalize an oversized tool output from `segment` into its sidecar file.
///
/// No-ops when the segment has no string output, the output fits inline, the
/// output is already a preview (`output_truncated`), or the segment id is not
/// filesystem-safe. Never overwrites a sidecar with an existing preview.
pub(crate) fn externalize_segment_tool_output(
    paths: &ConversationRecordPaths,
    segment: &mut TranscriptSegment,
) -> Result<()> {
    let Some(tool_call) = segment.tool_call.as_mut().and_then(Value::as_object_mut) else {
        return Ok(());
    };
    if tool_call.get("output_truncated").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let Some(output) = tool_call.get("output").and_then(Value::as_str) else {
        return Ok(());
    };
    if output.len() <= TOOL_OUTPUT_INLINE_LIMIT_BYTES || !is_safe_segment_file_id(&segment.id) {
        return Ok(());
    }
    let output = output.to_string();
    write_text_atomic(paths.tool_output_file(&segment.id), &output)?;
    let (preview, _) = truncate_utf8(&output, TOOL_OUTPUT_INLINE_LIMIT_BYTES);
    tool_call.insert("output".to_string(), json!(preview));
    tool_call.insert("output_size".to_string(), json!(output.len()));
    tool_call.insert("output_truncated".to_string(), json!(true));
    Ok(())
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
