from __future__ import annotations

import json
from pathlib import Path
import subprocess

from tests.compat import harness
from tests.compat.conftest import CompatServer
from spark_common.project_identity import build_project_id


ITEM_ID = "M0-I02-CLI-FILESYSTEM-FIXTURES"
REQUIREMENTS = ("RR-VAL-001", "RR-VAL-002")
DECISIONS = ("CD-RR-001", "CD-RR-013", "CD-RR-015")
AGENT_FLOW = "software-development/implement-change-request.dot"


def test_server_backed_cli_fixtures_match_python_oracle(
    compat_server: CompatServer,
    compat_fixture_root: Path,
    rewrite_worktree_path: Path,
) -> None:
    trigger_id: str | None = None
    scenarios: list[tuple[str, list[str], dict[str, object]]] = [
        (
            "cli/server-backed/flow-list-json",
            ["uv", "run", "spark", "flow", "list", "--base-url", compat_server.base_url],
            {},
        ),
        (
            "cli/server-backed/flow-list-text",
            ["uv", "run", "spark", "flow", "list", "--text", "--base-url", compat_server.base_url],
            {},
        ),
        (
            "cli/server-backed/flow-describe-text",
            [
                "uv",
                "run",
                "spark",
                "flow",
                "describe",
                "--flow",
                AGENT_FLOW,
                "--text",
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
        (
            "cli/server-backed/flow-get-json",
            ["uv", "run", "spark", "flow", "get", "--flow", AGENT_FLOW, "--base-url", compat_server.base_url],
            {},
        ),
        (
            "cli/server-backed/flow-validate-text",
            [
                "uv",
                "run",
                "spark",
                "flow",
                "validate",
                "--flow",
                AGENT_FLOW,
                "--text",
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
        (
            "cli/server-backed/flow-describe-missing",
            [
                "uv",
                "run",
                "spark",
                "flow",
                "describe",
                "--flow",
                "missing.dot",
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
        (
            "cli/server-backed/trigger-list-empty",
            ["uv", "run", "spark", "trigger", "list", "--base-url", compat_server.base_url],
            {},
        ),
        (
            "cli/server-backed/trigger-create",
            [
                "uv",
                "run",
                "spark",
                "trigger",
                "create",
                "--json",
                str(compat_server.trigger_payload_path),
                "--base-url",
                compat_server.base_url,
            ],
            {
                "snapshot_roots": {"spark_home": "__SPARK_HOME__"},
                "state_files": {"trigger_dir": "__SPARK_HOME__/config/triggers"},
            },
        ),
    ]

    for fixture_id, command, options in scenarios:
        actual = _run_capture(
            compat_server=compat_server,
            compat_fixture_root=compat_fixture_root,
            rewrite_worktree_path=rewrite_worktree_path,
            fixture_id=fixture_id,
            command=command,
            **options,
        )
        if fixture_id == "cli/server-backed/trigger-create":
            stdout_payload = json.loads(actual["process"]["stdout"])
            trigger_id = str(stdout_payload["id"])

    assert trigger_id is not None
    project_id = build_project_id(str(compat_server.project_dir))
    follow_up_scenarios = [
        (
            "cli/server-backed/trigger-describe-text",
            [
                "uv",
                "run",
                "spark",
                "trigger",
                "describe",
                "--id",
                trigger_id,
                "--text",
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
        (
            "cli/server-backed/trigger-delete",
            [
                "uv",
                "run",
                "spark",
                "trigger",
                "delete",
                "--id",
                trigger_id,
                "--base-url",
                compat_server.base_url,
            ],
            {
                "snapshot_roots": {"spark_home": "__SPARK_HOME__"},
                "state_files": {"trigger_definition": f"__SPARK_HOME__/config/triggers/{trigger_id}.toml"},
            },
        ),
        (
            "cli/server-backed/convo-run-request-success",
            [
                "uv",
                "run",
                "spark",
                "convo",
                "run-request",
                "--conversation",
                compat_server.conversation_handle,
                "--flow",
                AGENT_FLOW,
                "--summary",
                "Compatibility fixture request",
                "--goal",
                "Keep this request pending.",
                "--base-url",
                compat_server.base_url,
            ],
            {
                "snapshot_roots": {"spark_home": "__SPARK_HOME__"},
                "state_files": {
                    "conversation_state": (
                        f"__SPARK_HOME__/workspace/projects/{project_id}/conversations/"
                        f"{compat_server.conversation_id}/state.json"
                    ),
                    "flow_run_requests": (
                        f"__SPARK_HOME__/workspace/projects/{project_id}/flow-run-requests/"
                        f"{compat_server.conversation_id}.json"
                    ),
                },
            },
        ),
        (
            "cli/server-backed/convo-run-request-unknown-handle",
            [
                "uv",
                "run",
                "spark",
                "convo",
                "run-request",
                "--conversation",
                "unknown-handle",
                "--flow",
                AGENT_FLOW,
                "--summary",
                "Compatibility fixture request",
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
        (
            "cli/server-backed/run-launch-missing-flow",
            [
                "uv",
                "run",
                "spark",
                "run",
                "launch",
                "--flow",
                "missing.dot",
                "--summary",
                "Missing flow",
                "--project",
                str(compat_server.project_dir),
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
        (
            "cli/server-backed/run-retry-missing",
            [
                "uv",
                "run",
                "spark",
                "run",
                "retry",
                "--run",
                "missing-run",
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
        (
            "cli/server-backed/run-continue-missing",
            [
                "uv",
                "run",
                "spark",
                "run",
                "continue",
                "--run",
                "missing-run",
                "--start-node",
                "start",
                "--flow-source-mode",
                "snapshot",
                "--base-url",
                compat_server.base_url,
            ],
            {},
        ),
    ]
    for fixture_id, command, options in follow_up_scenarios:
        _run_capture(
            compat_server=compat_server,
            compat_fixture_root=compat_fixture_root,
            rewrite_worktree_path=rewrite_worktree_path,
            fixture_id=fixture_id,
            command=command,
            **options,
        )


def _run_capture(
    *,
    compat_server: CompatServer,
    compat_fixture_root: Path,
    rewrite_worktree_path: Path,
    fixture_id: str,
    command: list[str],
    snapshot_roots: dict[str, str | Path] | None = None,
    state_files: dict[str, str | Path] | None = None,
) -> dict[str, object]:
    output = compat_server.capture_output(fixture_id)
    result = subprocess.run(
        compat_server.capture_command(
            rewrite_worktree_path=rewrite_worktree_path,
            fixture_id=fixture_id,
            item_id=ITEM_ID,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            command=command,
            output=output,
            snapshot_roots=snapshot_roots,
            state_files=state_files,
        ),
        cwd=rewrite_worktree_path,
        env=compat_server.env,
        text=True,
        capture_output=True,
        check=False,
    )
    assert result.returncode == 0, result.stderr
    actual = harness.load_manifest(output)
    expected = harness.load_manifest(compat_fixture_root / f"{fixture_id}.json")
    harness.assert_manifest_matches_golden(actual, expected)
    return actual
