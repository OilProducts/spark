from __future__ import annotations

from pathlib import Path
import subprocess

from tests.compat import harness
from tests.compat.conftest import CompatSandbox


ITEM_ID = "M0-I02-CLI-FILESYSTEM-FIXTURES"
REQUIREMENTS = ("RR-VAL-001", "RR-VAL-002")
DECISIONS = ("CD-RR-001", "CD-RR-013", "CD-RR-015")


def test_local_cli_process_fixtures_match_python_oracle(
    compat_sandbox: CompatSandbox,
    compat_fixture_root: Path,
    rewrite_worktree_path: Path,
) -> None:
    messy_write_path = compat_sandbox.project_dir / "messy-write.dot"
    messy_write_path.write_text(
        compat_sandbox.messy_flow_path.read_text(encoding="utf-8"),
        encoding="utf-8",
    )

    scenarios = [
        (
            "cli/help-top-level",
            ["uv", "run", "spark", "--help"],
            {},
        ),
        (
            "cli/flow-validate-file-success",
            ["uv", "run", "spark", "flow", "validate", "--file", str(compat_sandbox.valid_flow_path)],
            {},
        ),
        (
            "cli/flow-validate-file-validation-error-text",
            [
                "uv",
                "run",
                "spark",
                "flow",
                "validate",
                "--file",
                str(compat_sandbox.validation_error_flow_path),
                "--text",
            ],
            {},
        ),
        (
            "cli/flow-validate-file-missing",
            [
                "uv",
                "run",
                "spark",
                "flow",
                "validate",
                "--file",
                str(compat_sandbox.project_dir / "missing.dot"),
            ],
            {},
        ),
        (
            "cli/flow-format-stdout",
            ["uv", "run", "spark", "flow", "format", "--file", str(compat_sandbox.messy_flow_path)],
            {},
        ),
        (
            "cli/flow-format-write",
            ["uv", "run", "spark", "flow", "format", "--file", str(messy_write_path), "--write"],
            {
                "snapshot_roots": {"project": "__PROJECT_DIR__"},
                "state_files": {"formatted_flow": messy_write_path},
            },
        ),
        (
            "cli/flow-format-invalid",
            ["uv", "run", "spark", "flow", "format", "--file", str(compat_sandbox.invalid_flow_path)],
            {},
        ),
        (
            "cli/argparse-usage-error",
            [
                "uv",
                "run",
                "spark",
                "run",
                "launch",
                "--flow",
                "test-dispatch.dot",
                "--summary",
                "Launch directly",
                "--project",
                str(compat_sandbox.project_dir),
                "--image",
                "direct-selection",
            ],
            {},
        ),
        (
            "cli/agent-source-checkout-guard",
            ["uv", "run", "spark", "flow", "list"],
            {
                "no_default_path_env": True,
                "unset_command_env": (
                    "SPARK_API_BASE_URL",
                    "SPARK_HOME",
                    "SPARK_FLOWS_DIR",
                    "ATTRACTOR_CODEX_RUNTIME_ROOT",
                    "CODEX_HOME",
                ),
            },
        ),
        (
            "cli/server-help",
            ["uv", "run", "spark-server", "--help"],
            {},
        ),
        (
            "cli/server-worker-help",
            ["uv", "run", "spark-server", "worker", "run-node", "--help"],
            {},
        ),
        (
            "cli/server-init-success",
            [
                "uv",
                "run",
                "spark-server",
                "init",
                "--data-dir",
                str(compat_sandbox.spark_home),
                "--flows-dir",
                str(compat_sandbox.flows_dir),
            ],
            {
                "snapshot_roots": {
                    "spark_home": "__SPARK_HOME__",
                    "flows": "__FLOWS_DIR__",
                },
                "state_files": {
                    "flow_catalog": "__SPARK_HOME__/config/flow-catalog.toml",
                },
            },
        ),
        (
            "cli/server-init-source-checkout-guard",
            ["uv", "run", "spark-server", "init"],
            {
                "no_default_path_env": True,
                "unset_command_env": (
                    "SPARK_HOME",
                    "SPARK_FLOWS_DIR",
                    "ATTRACTOR_CODEX_RUNTIME_ROOT",
                    "CODEX_HOME",
                ),
            },
        ),
    ]

    for fixture_id, command, options in scenarios:
        if fixture_id == "cli/flow-format-write":
            messy_write_path.write_text(
                compat_sandbox.messy_flow_path.read_text(encoding="utf-8"),
                encoding="utf-8",
            )
        output = compat_sandbox.capture_output(fixture_id)
        result = subprocess.run(
            compat_sandbox.capture_command(
                rewrite_worktree_path=rewrite_worktree_path,
                fixture_id=fixture_id,
                item_id=ITEM_ID,
                requirements=REQUIREMENTS,
                decisions=DECISIONS,
                command=command,
                output=output,
                normalize_paths={"PROJECT_DIR": compat_sandbox.project_dir},
                **options,
            ),
            cwd=rewrite_worktree_path,
            env=compat_sandbox.env,
            text=True,
            capture_output=True,
            check=False,
        )
        assert result.returncode == 0, result.stderr

        actual = harness.load_manifest(output)
        expected = harness.load_manifest(compat_fixture_root / f"{fixture_id}.json")
        harness.assert_manifest_matches_golden(actual, expected)
