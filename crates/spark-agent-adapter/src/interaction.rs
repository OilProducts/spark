//! The native agent's interaction tools: `request_user_input` asks the user,
//! `submit_plan` ends a plan turn with its plan. The session answers them
//! itself; neither is in a provider's tool registry.
use std::collections::BTreeMap;

use serde_json::{json, Value};
use unified_llm_adapter::{Tool, ToolCall};

pub const REQUEST_USER_INPUT_TOOL: &str = "request_user_input";
pub const SUBMIT_PLAN_TOOL: &str = "submit_plan";

pub(crate) const NO_QUESTION_HANDLER: &str =
    "No one can answer questions during this turn. Proceed on your best judgment and state the assumptions you made.";
pub(crate) const PLAN_SUBMITTED: &str =
    "Your plan was submitted to the user for review. Stop here; do not implement it.";
const PLAN_MISSING: &str = "submit_plan needs your plan in `plan`.";

/// The tools a plan turn keeps: none of them changes the workspace.
pub(crate) const READ_ONLY_TOOLS: &[&str] = &[
    "read_file",
    "read_many_files",
    "grep",
    "glob",
    "list_dir",
    "web_search",
    "web_fetch",
];

pub(crate) const PLAN_INSTRUCTIONS: &str = "You are in plan mode. Explore with the read-only tools available, then call submit_plan with your complete implementation plan in Markdown. Do not change files or run commands that change state; the plan is reviewed before anything is implemented.";

pub(crate) fn request_user_input_tool() -> Tool {
    Tool::passive_with_schema(
        REQUEST_USER_INPUT_TOOL,
        Some("Ask the user questions and wait for their answers. Use it only for unresolved questions about the user's intent that affect the outcome, after inspecting the available evidence.".to_string()),
        Some(json!({
            "type": "object",
            "properties": {"questions": {"type": "array", "minItems": 1, "items": {
                "type": "object",
                "properties": {
                    "header": {"type": "string", "description": "A short label for the question."},
                    "question": {"type": "string", "minLength": 1},
                    "options": {"type": "array", "items": {
                        "type": "object",
                        "properties": {"label": {"type": "string", "minLength": 1}, "description": {"type": "string"}},
                        "required": ["label"]
                    }}
                },
                "required": ["question"]
            }}},
            "required": ["questions"]
        })),
    )
    .expect("request_user_input is a valid tool")
}

pub(crate) fn submit_plan_tool() -> Tool {
    Tool::passive_with_schema(
        SUBMIT_PLAN_TOOL,
        Some("Submit your finished plan for the user to review. This ends the turn.".to_string()),
        Some(json!({
            "type": "object",
            "properties": {"plan": {"type": "string", "minLength": 1, "description": "The complete plan, in Markdown."}},
            "required": ["plan"]
        })),
    )
    .expect("submit_plan is a valid tool")
}

pub(crate) fn arguments(tool_call: &ToolCall) -> Value {
    match (&tool_call.raw_arguments, &tool_call.arguments) {
        (Some(raw), _) | (None, Value::String(raw)) => {
            serde_json::from_str(raw).unwrap_or(Value::Null)
        }
        (None, arguments) => arguments.clone(),
    }
}

/// Questions as the request-user-input segment and the flow clarification
/// handler take them, each with a `question-N` id.
pub(crate) fn normalized_questions(arguments: &Value) -> Result<Vec<Value>, String> {
    let questions = arguments["questions"]
        .as_array()
        .filter(|questions| !questions.is_empty())
        .ok_or("request_user_input needs at least one question in `questions`")?;
    questions
        .iter()
        .enumerate()
        .map(|(index, question)| {
            let prompt = question["question"]
                .as_str()
                .map(str::trim)
                .filter(|prompt| !prompt.is_empty())
                .ok_or("every question needs its text in `question`")?;
            let mut question = question.clone();
            question["id"] = json!(format!("question-{}", index + 1));
            question["question"] = json!(prompt);
            question["allow_other"] = json!(true);
            Ok(question)
        })
        .collect()
}

pub(crate) fn answer_text(questions: &[Value], answers: &BTreeMap<String, String>) -> String {
    questions
        .iter()
        .filter_map(|question| {
            let answer = answers.get(question["id"].as_str()?)?;
            Some(format!(
                "{}\nAnswer: {answer}",
                question["question"].as_str()?
            ))
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub(crate) fn plan(arguments: &Value) -> Result<String, String> {
    arguments["plan"]
        .as_str()
        .map(str::trim)
        .filter(|plan| !plan.is_empty())
        .map(str::to_string)
        .ok_or_else(|| PLAN_MISSING.to_string())
}
