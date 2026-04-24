from __future__ import annotations

import agent


def test_default_truncation_tables_cover_the_required_limits_and_modes() -> None:
    assert agent.DEFAULT_TOOL_OUTPUT_LIMITS == {
        "apply_patch": 10_000,
        "edit_file": 10_000,
        "glob": 20_000,
        "grep": 20_000,
        "read_file": 50_000,
        "shell": 30_000,
        "spawn_agent": 20_000,
        "write_file": 1_000,
    }
    assert agent.DEFAULT_TRUNCATION_MODES == {
        "apply_patch": "tail",
        "edit_file": "tail",
        "glob": "tail",
        "grep": "tail",
        "read_file": "head_tail",
        "shell": "head_tail",
        "spawn_agent": "head_tail",
        "write_file": "tail",
    }
    assert agent.DEFAULT_TOOL_LINE_LIMITS == {
        "apply_patch": None,
        "edit_file": None,
        "glob": 500,
        "grep": 200,
        "read_file": None,
        "shell": 256,
        "spawn_agent": None,
        "write_file": None,
    }


def test_truncate_output_head_tail_keeps_the_beginning_and_end() -> None:
    output = "0123456789ABCDEFGHIJ"

    assert agent.truncate_output(output, 10, "head_tail") == (
        "01234\n\n[WARNING: Tool output was truncated. 10 characters were removed from the middle. "
        "The full output is available in the event stream. "
        "If you need to see specific parts, re-run the tool with more targeted parameters.]\n\n"
        "FGHIJ"
    )


def test_truncate_output_tail_keeps_the_tail_and_reports_the_removed_prefix() -> None:
    output = "0123456789ABCDEFGHIJ"

    assert agent.truncate_output(output, 10, "tail") == (
        "[WARNING: Tool output was truncated. First 10 characters were removed. "
        "The full output is available in the event stream.]\n\n"
        "ABCDEFGHIJ"
    )


def test_truncate_lines_keeps_head_and_tail_lines() -> None:
    output = "line1\nline2\nline3\nline4\nline5"

    assert agent.truncate_lines(output, 3) == "line1\n[... 2 lines omitted ...]\nline4\nline5"


def test_truncate_tool_output_applies_char_and_line_overrides() -> None:
    output = "X" * 20 + "\nline2\nline3"
    config = agent.SessionConfig(
        tool_output_limits={"shell": 10},
    )
    config.tool_line_limits = {"shell": 2}

    assert agent.truncate_tool_output(output, "shell", config) == (
        "XXXXX\n[... 3 lines omitted ...]\nline3"
    )
