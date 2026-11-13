//! The drift guard: every `spark` command the guides, roles and prompts
//! teach parses against the real command tree.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use spark_cli::{request_plan_with_args_env_and_stdin, EXIT_USAGE_ERROR};

/// The Rust sources whose prompt strings teach `spark` commands.
const PROMPT_SOURCES: [&str; 5] = [
    "crates/spark-workspace/src/conversations.rs",
    "crates/spark-workspace/src/missions.rs",
    "crates/spark-workspace/src/roles.rs",
    "crates/spark-workspace/src/delegations.rs",
    "crates/spark-storage/src/conversation/operations.rs",
];

#[test]
fn every_taught_spark_command_parses() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut files = PROMPT_SOURCES.map(|path| root.join(path)).to_vec();
    for dir in ["guides", "roles"] {
        files.extend(markdown_files(
            &root.join("crates/spark-assets/assets").join(dir),
        ));
    }
    let env = BTreeMap::from([(
        "SPARK_API_BASE_URL".to_string(),
        "http://127.0.0.1:9".to_string(),
    )]);
    let mut checked = 0;
    let mut broken = Vec::new();
    for file in &files {
        let text = fs::read_to_string(file).expect("read teaching source");
        for command in taught_commands(&text) {
            checked += 1;
            let args = sample_arguments(&command);
            // A plan, help, or a local error such as a missing file all
            // parse; only a usage error means the teaching drifted.
            if let Err(output) = request_plan_with_args_env_and_stdin(&args, &env, "{}") {
                if output.exit_code == EXIT_USAGE_ERROR {
                    broken.push(format!(
                        "{}: `{command}` -> {args:?}: {}",
                        file.display(),
                        output.stderr.trim()
                    ));
                }
            }
        }
    }
    assert!(checked > 50, "found only {checked} taught commands");
    assert!(
        broken.is_empty(),
        "taught commands that do not parse:\n{}",
        broken.join("\n")
    );
}

#[test]
fn the_drift_guard_rejects_a_command_that_does_not_parse() {
    let env = BTreeMap::new();
    let text = "Launch with `spark convo run-request --conversation {handle} --flow <name>`.";
    let commands = taught_commands(text);
    assert_eq!(
        commands,
        ["spark convo run-request --conversation {handle} --flow <name>"]
    );
    let output = request_plan_with_args_env_and_stdin(sample_arguments(&commands[0]), &env, "")
        .expect_err("convo is not a domain");
    assert_eq!(output.exit_code, EXIT_USAGE_ERROR);
}

fn markdown_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .expect("read assets dir")
        .map(|entry| entry.expect("dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .collect()
}

/// Backticked spans and fenced lines that start with `spark `.
fn taught_commands(text: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut fenced = false;
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        } else if fenced {
            commands.push(line.trim().to_string());
        } else {
            commands.extend(line.split('`').skip(1).step_by(2).map(str::to_string));
        }
    }
    commands.retain(|command| command.starts_with("spark "));
    commands
}

/// The command's words with sample values for its placeholders: `<...>`
/// and `{...}` become `x`, `a|b` becomes `a`, and `spark <domain>` names a
/// real domain. Stops at a shell operator.
fn sample_arguments(command: &str) -> Vec<String> {
    let command = command.replace("spark <domain>", "spark run");
    let mut sample = String::new();
    let mut closing = None;
    for ch in command.chars() {
        match (closing, ch) {
            (None, '<') => closing = Some('>'),
            (None, '{') => closing = Some('}'),
            (None, ch) => sample.push(ch),
            (Some(close), ch) if ch == close => {
                closing = None;
                sample.push('x');
            }
            (Some(_), _) => {}
        }
    }
    shell_words(&sample)
        .into_iter()
        .take_while(|word| !matches!(word.as_str(), "|" | "&&" | ";"))
        .map(|word| word.split('|').next().unwrap_or_default().to_string())
        .collect()
}

/// Splits on whitespace, keeping single- and double-quoted text together.
fn shell_words(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    let mut word: Option<String> = None;
    let mut quote = None;
    for ch in text.chars() {
        match (quote, ch) {
            (Some(open), ch) if ch == open => quote = None,
            (Some(_), ch) => word.get_or_insert_with(String::new).push(ch),
            (None, '"' | '\'') => {
                quote = Some(ch);
                word.get_or_insert_with(String::new);
            }
            (None, ch) if ch.is_whitespace() => words.extend(word.take()),
            (None, ch) => word.get_or_insert_with(String::new).push(ch),
        }
    }
    words.extend(word);
    words
}
