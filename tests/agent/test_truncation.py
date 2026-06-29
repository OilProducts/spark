from __future__ import annotations

import agent


def _assert_full_output_notice(output: str) -> None:
    assert "[WARNING: Tool output was truncated." in output
    assert "The full output is available in the event stream." in output


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
    truncated = agent.truncate_output(output, 10, "head_tail")

    assert truncated == (
        "01234\n\n[WARNING: Tool output was truncated. 10 characters were removed from the middle. "
        "The full output is available in the event stream. "
        "If you need to see specific parts, re-run the tool with more targeted parameters.]\n\n"
        "FGHIJ"
    )
    _assert_full_output_notice(truncated)


def test_truncate_output_tail_keeps_the_tail_and_reports_the_removed_prefix() -> None:
    output = "0123456789ABCDEFGHIJ"
    truncated = agent.truncate_output(output, 10, "tail")

    assert truncated == (
        "[WARNING: Tool output was truncated. First 10 characters were removed. "
        "The full output is available in the event stream.]\n\n"
        "ABCDEFGHIJ"
    )
    _assert_full_output_notice(truncated)


def test_truncate_output_preserves_utf8_character_boundaries() -> None:
    output = "".join(chr(codepoint) for codepoint in range(0x03B1, 0x03B1 + 10))

    truncated = agent.truncate_output(output, 5, "head_tail")

    assert truncated.encode("utf-8").decode("utf-8") == truncated
    assert truncated.startswith(output[:2])
    assert truncated.endswith(output[-3:])
    assert "5 characters were removed from the middle" in truncated
    _assert_full_output_notice(truncated)


def test_truncate_lines_keeps_head_and_tail_lines() -> None:
    output = "line1\nline2\nline3\nline4\nline5"

    assert agent.truncate_lines(output, 3) == (
        "line1\n"
        "[WARNING: Tool output was truncated. 2 lines omitted. "
        "The full output is available in the event stream.]\n"
        "line4\nline5"
    )


def test_truncate_tool_output_applies_character_limit_before_line_limit() -> None:
    output = "\n".join(f"line-{line:02d}-" + ("X" * 30) for line in range(12))
    config = agent.SessionConfig(
        tool_output_limits={"shell": 80},
        line_limits={"shell": 2},
    )

    truncated = agent.truncate_tool_output(output, "shell", config)

    assert output[:30] in truncated
    assert output[-30:] in truncated
    assert "characters were removed from the middle" in truncated
    assert "lines omitted" in truncated
    _assert_full_output_notice(truncated)


def test_truncate_tool_output_uses_line_limit_overrides_after_character_overrides() -> None:
    output = "\n".join(f"line-{line}" for line in range(8))
    config = agent.SessionConfig(
        tool_output_limits={"shell": 1_000},
    )
    config.tool_line_limits = {"shell": 3}

    truncated = agent.truncate_tool_output(output, "shell", config)

    assert "5 lines omitted" in truncated
    _assert_full_output_notice(truncated)
    assert truncated.startswith("line-0")
    assert truncated.endswith("line-7")


def test_default_shell_line_limit_applies_after_character_truncation() -> None:
    output = "\n".join(f"line-{line}" for line in range(300))

    truncated = agent.truncate_tool_output(output, "shell", agent.SessionConfig())

    assert "44 lines omitted" in truncated
    assert truncated.startswith("line-0")
    assert truncated.endswith("line-299")
    assert "characters were removed" not in truncated
    _assert_full_output_notice(truncated)


def test_default_tail_mode_for_grep_uses_default_character_limit() -> None:
    output = "A" * 20_100

    truncated = agent.truncate_tool_output(output, "grep", agent.SessionConfig())

    assert truncated.startswith(
        "[WARNING: Tool output was truncated. First 100 characters were removed."
    )
    assert truncated.endswith("A" * 20_000)
    _assert_full_output_notice(truncated)


def test_long_line_output_remains_bounded_by_character_truncation_before_line_counting() -> None:
    output = ("A" * 5_000) + "\n" + ("B" * 5_000)
    config = agent.SessionConfig(
        tool_output_limits={"shell": 40},
        line_limits={"shell": 1},
    )

    truncated = agent.truncate_tool_output(output, "shell", config)

    assert len(truncated) < 800
    assert "A" * 100 not in truncated
    assert "B" * 100 not in truncated
    assert "lines omitted" in truncated
    _assert_full_output_notice(truncated)
