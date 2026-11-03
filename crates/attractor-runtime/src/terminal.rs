use attractor_core::{ExitResult, FlowDefinition};
use serde::{Deserialize, Serialize};

/// A run's terminal record fields: the exit's declared outcome, its result
/// name, and the evidence of how the run got there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalWorkflowOutcome {
    pub outcome: String,
    /// The declared terminal result name, such as `completed` or `blocked`.
    pub outcome_reason_code: Option<String>,
    pub outcome_reason_message: Option<String>,
}

/// The terminal outcome of reaching `exit_id`, reached by `origin`'s
/// decision (node, decision, notes) when known.
pub fn exit_terminal_outcome(
    flow: &FlowDefinition,
    exit_id: &str,
    origin: Option<(&str, &str, &str)>,
) -> Option<TerminalWorkflowOutcome> {
    let ExitResult { name, outcome } = flow.exit_result(exit_id)?.clone();
    let message = origin.map(|(node, decision, notes)| {
        let notes = notes.trim();
        if notes.is_empty() {
            format!("{node} chose {decision}")
        } else {
            format!("{node} chose {decision}: {notes}")
        }
    });
    Some(TerminalWorkflowOutcome {
        outcome: outcome.as_str().to_string(),
        outcome_reason_code: Some(name),
        outcome_reason_message: message,
    })
}
