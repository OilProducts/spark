from __future__ import annotations

import re
import shlex
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[2]
DOC_PATHS = (
    Path("README.md"),
    Path("README-package.md"),
    Path("docs/first-flow-tutorial.md"),
    Path("src/spark/guides/dot-authoring.md"),
    Path("src/spark/guides/spark-operations.md"),
)
SHELL_LANGS = {"", "bash", "sh", "shell", "console"}
ENV_ASSIGNMENT_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=.*$")
FENCE_RE = re.compile(r"(?ms)^```(?P<lang>[A-Za-z0-9_-]*)\s*$\n(?P<body>.*?)(?=^```\s*$)^```\s*$")

SUPPORTED_SPARK_FAMILIES = {
    ("convo", "run-request"),
    ("flow", "describe"),
    ("flow", "format"),
    ("flow", "get"),
    ("flow", "list"),
    ("flow", "validate"),
    ("run", "continue"),
    ("run", "events"),
    ("run", "launch"),
    ("run", "retry"),
    ("trigger", "create"),
    ("trigger", "delete"),
    ("trigger", "describe"),
    ("trigger", "list"),
    ("trigger", "update"),
}
SUPPORTED_SPARK_SERVER_FAMILIES = {
    ("init",),
    ("serve",),
    ("service", "install"),
    ("service", "remove"),
    ("service", "status"),
}
REQUIRED_DOCUMENTED_FAMILIES = {
    ("spark-server", "init"),
    ("spark-server", "serve"),
    ("spark-server", "service", "install"),
    ("spark-server", "service", "remove"),
    ("spark-server", "service", "status"),
    ("spark", "flow", "validate"),
}


def test_documented_spark_command_examples_use_supported_public_surface() -> None:
    examples = list(_spark_command_examples(DOC_PATHS))
    assert examples, "Expected Spark command examples in user-facing docs."

    unsupported: list[str] = []
    for example in examples:
        family = _command_family(example.tokens)
        if example.command == "spark" and family not in SUPPORTED_SPARK_FAMILIES:
            unsupported.append(example.render())
        if example.command == "spark-server" and family not in SUPPORTED_SPARK_SERVER_FAMILIES:
            unsupported.append(example.render())

    assert unsupported == []


def test_core_public_command_families_remain_documented() -> None:
    documented = {
        (example.command, *_command_family(example.tokens))
        for example in _spark_command_examples(DOC_PATHS)
    }

    assert REQUIRED_DOCUMENTED_FAMILIES <= documented


def test_source_checkout_server_init_examples_are_isolated_from_stable_home() -> None:
    checkout_init_examples = [
        example
        for example in _spark_command_examples((Path("README.md"), Path("docs/first-flow-tutorial.md")))
        if example.used_uv_run and example.command == "spark-server" and _command_family(example.tokens) == ("init",)
    ]
    assert checkout_init_examples, "Expected source-checkout spark-server init examples."

    unsafe = [
        example.render()
        for example in checkout_init_examples
        if not any(assignment.startswith("SPARK_HOME=") for assignment in example.env_assignments)
    ]
    assert unsafe == []


def test_first_flow_native_source_checkout_examples_use_cargo_built_binaries() -> None:
    examples = list(_spark_command_examples((Path("docs/first-flow-tutorial.md"),)))

    assert any(
        example.command == "spark-server"
        and example.executable == "target/debug/spark-server"
        and _command_family(example.tokens) == ("init",)
        for example in examples
    )
    assert any(
        example.command == "spark-server"
        and example.executable == "target/debug/spark-server"
        and _command_family(example.tokens) == ("serve",)
        for example in examples
    )
    assert any(
        example.command == "spark"
        and example.executable == "target/debug/spark"
        and _command_family(example.tokens) == ("run", "launch")
        for example in examples
    )


def test_source_checkout_wrappers_are_not_described_as_native_rust_execution() -> None:
    docs = (Path("README.md"), Path("docs/first-flow-tutorial.md"))
    offending_blocks: list[str] = []
    for doc_path in docs:
        text = (REPO_ROOT / doc_path).read_text(encoding="utf-8")
        for block in _markdown_text_blocks(text):
            normalized = block.lower()
            mentions_wrapper = (
                "uv run spark" in normalized
                or "uv run spark-server" in normalized
                or "just dev-run" in normalized
            )
            claims_native = "rust-backed" in normalized or "native rust" in normalized
            if mentions_wrapper and claims_native:
                offending_blocks.append(f"{doc_path}: {block}")

    assert offending_blocks == []


class CommandExample:
    def __init__(
        self,
        *,
        doc_path: Path,
        line: str,
        command: str,
        executable: str,
        tokens: list[str],
        env_assignments: list[str],
        used_uv_run: bool,
    ) -> None:
        self.doc_path = doc_path
        self.line = line
        self.command = command
        self.executable = executable
        self.tokens = tokens
        self.env_assignments = env_assignments
        self.used_uv_run = used_uv_run

    def render(self) -> str:
        return f"{self.doc_path}: {self.line}"


def _spark_command_examples(doc_paths: tuple[Path, ...]) -> list[CommandExample]:
    examples: list[CommandExample] = []
    for doc_path in doc_paths:
        text = (REPO_ROOT / doc_path).read_text(encoding="utf-8")
        for line in _shell_lines_from_markdown(text):
            parsed = _parse_command_line(doc_path, line)
            if parsed is not None:
                examples.append(parsed)
    return examples


def _shell_lines_from_markdown(text: str) -> list[str]:
    lines: list[str] = []
    for match in FENCE_RE.finditer(text):
        lang = match.group("lang").strip().lower()
        if lang not in SHELL_LANGS:
            continue
        lines.extend(_logical_shell_lines(match.group("body")))
    return lines


def _logical_shell_lines(block: str) -> list[str]:
    lines: list[str] = []
    pending = ""
    for raw_line in block.splitlines():
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("$ "):
            line = line[2:].strip()
        if line.endswith("\\"):
            pending += line[:-1].strip() + " "
            continue
        line = pending + line
        pending = ""
        lines.extend(part.strip() for part in line.split(";") if part.strip())
    if pending.strip():
        lines.append(pending.strip())
    return lines


def _parse_command_line(doc_path: Path, line: str) -> CommandExample | None:
    try:
        tokens = shlex.split(line)
    except ValueError:
        return None

    env_assignments: list[str] = []
    while tokens and ENV_ASSIGNMENT_RE.match(tokens[0]):
        env_assignments.append(tokens.pop(0))

    used_uv_run = tokens[:2] == ["uv", "run"]
    if used_uv_run:
        tokens = tokens[2:]

    if not tokens:
        return None

    command = Path(tokens[0]).name
    if command not in {"spark", "spark-server"}:
        return None

    normalized = [command, *tokens[1:]]
    return CommandExample(
        doc_path=doc_path,
        line=line,
        command=command,
        executable=tokens[0],
        tokens=normalized,
        env_assignments=env_assignments,
        used_uv_run=used_uv_run,
    )


def _markdown_text_blocks(text: str) -> list[str]:
    text_without_fences = FENCE_RE.sub("", text)
    return [
        " ".join(line.strip() for line in block.splitlines() if line.strip())
        for block in re.split(r"\n\s*\n", text_without_fences)
        if block.strip()
    ]


def _command_family(tokens: list[str]) -> tuple[str, ...]:
    command = tokens[0]
    args = [token for token in tokens[1:] if not token.startswith("-")]
    if command == "spark":
        return tuple(args[:2])
    if args[:1] == ["service"]:
        return tuple(args[:2])
    return tuple(args[:1])
