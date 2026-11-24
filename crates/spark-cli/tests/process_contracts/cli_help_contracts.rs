//! Domains list their commands, and Spark prints help on stdout with exit 0.

use std::process::Command;

/// Each command path and the subcommands its help lists.
const TREE: &[(&str, &[&str])] = &[
    (
        "",
        &[
            "flow",
            "run",
            "send",
            "mission",
            "role",
            "playbook",
            "trigger",
            "settings",
            "environment",
            "delegation",
        ],
    ),
    ("flow", &["list", "get", "validate"]),
    (
        "run",
        &[
            "start",
            "list",
            "get",
            "cancel",
            "retry",
            "continue",
            "events",
            "questions",
            "answer",
        ],
    ),
    (
        "mission",
        &["list", "get", "create", "update", "start", "wait", "close"],
    ),
    ("role", &["list", "start", "stop"]),
    ("playbook", &["list", "get"]),
    ("trigger", &["list", "get", "create", "update", "delete"]),
    ("settings", &["get", "validate", "set"]),
    ("environment", &["show", "pair-code", "tokens", "link"]),
    ("environment tokens", &["list", "revoke"]),
    ("environment link", &["add", "list", "projects", "remove"]),
    (
        "delegation",
        &["start", "list", "get", "wait", "cancel", "answer"],
    ),
];

fn help(args: &[&str]) -> String {
    let output = Command::new(env!("CARGO_BIN_EXE_spark"))
        .args(args)
        .env_clear()
        .output()
        .expect("run spark");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(0), "spark {args:?}: {stderr}");
    assert_eq!(stderr, "", "spark {args:?}");
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    let path = args.iter().filter(|arg| !arg.starts_with('-'));
    let usage = format!(
        "Usage: spark {}",
        path.copied().collect::<Vec<_>>().join(" ")
    );
    assert!(stdout.contains(&usage), "spark {args:?}:\n{stdout}");
    stdout
}

/// The first word of each line under `Commands:`.
fn listed(help: &str) -> Vec<&str> {
    help.lines()
        .skip_while(|line| *line != "Commands:")
        .skip(1)
        .take_while(|line| !line.trim().is_empty())
        .filter_map(|line| line.split_whitespace().next())
        .collect()
}

#[test]
fn domains_list_their_commands_and_help_exits_zero() {
    for (path, subcommands) in TREE {
        let path = path.split_whitespace().collect::<Vec<_>>();
        let help = help(&path);
        assert_eq!(listed(&help), *subcommands, "spark {path:?}:\n{help}");
    }

    // ponytail: clap shares leaf help handling; cover required arguments and
    // nested commands once. Add cases if a command gets custom help handling.
    for args in [
        vec!["--help"],
        vec!["-h"],
        vec!["run", "start", "--help"],
        vec!["environment", "tokens", "revoke", "-h"],
    ] {
        let help = help(&args);
        assert!(help.contains("Options:"), "spark {args:?}:\n{help}");
    }
}
