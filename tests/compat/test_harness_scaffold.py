from __future__ import annotations

import json
from pathlib import Path
import subprocess
import sys

from tests.compat import harness
from tests.compat.conftest import CompatSandbox


ITEM_ID = "M0-I01-HARNESS-SCAFFOLD"
REQUIREMENTS = ("RR-VAL-001", "RR-VAL-002")
DECISIONS = ("CD-RR-001", "CD-RR-013", "CD-RR-015")


def test_capture_cli_refuses_to_run_outside_rewrite_worktree(
    tmp_path: Path,
    rewrite_worktree_path: Path,
) -> None:
    script = rewrite_worktree_path / "scripts" / "compat_capture.py"
    marker = tmp_path / "command-ran.txt"
    output = tmp_path / "capture.json"

    result = subprocess.run(
        [
            sys.executable,
            str(script),
            "--fixture-id",
            "harness/refuse-outside-worktree",
            "--item-id",
            ITEM_ID,
            "--requirement",
            REQUIREMENTS[0],
            "--decision",
            DECISIONS[0],
            "--output",
            str(output),
            "--",
            sys.executable,
            "-c",
            f"from pathlib import Path; Path({str(marker)!r}).write_text('ran')",
        ],
        cwd=tmp_path,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 2
    assert "refusing to capture outside the configured rewrite worktree" in result.stderr
    assert not marker.exists()
    assert not output.exists()


def test_capture_cli_records_self_check_manifest(
    compat_sandbox: CompatSandbox,
    compat_validation_root: Path,
    rewrite_worktree_path: Path,
) -> None:
    script = rewrite_worktree_path / "scripts" / "compat_capture.py"
    output = compat_sandbox.capture_output("harness/self-check")

    result = subprocess.run(
        [
            sys.executable,
            str(script),
            "--fixture-id",
            "harness/self-check",
            "--item-id",
            ITEM_ID,
            "--requirement",
            REQUIREMENTS[0],
            "--requirement",
            REQUIREMENTS[1],
            "--decision",
            DECISIONS[0],
            "--decision",
            DECISIONS[1],
            "--decision",
            DECISIONS[2],
            "--validation-root",
            str(compat_validation_root),
            "--output",
            str(output),
            "--",
            sys.executable,
            "-c",
            (
                "import os, sys; "
                "print('stdout:' + os.environ['SPARK_HOME']); "
                "print('stderr:self-check', file=sys.stderr)"
            ),
        ],
        cwd=rewrite_worktree_path,
        env=compat_sandbox.env,
        text=True,
        capture_output=True,
        check=False,
    )

    assert result.returncode == 0
    manifest = harness.load_manifest(output)
    coverage = harness.validate_manifest_coverage(
        manifest,
        requirement_ids=REQUIREMENTS,
        decision_ids=DECISIONS,
    )
    assert coverage["requirements"] == sorted(REQUIREMENTS)
    assert coverage["decisions"] == sorted(DECISIONS)
    assert manifest["command"]["argv"][:2] == [sys.executable, "-c"]
    assert manifest["command"]["cwd"] == str(rewrite_worktree_path)
    assert manifest["process"]["returncode"] == 0
    assert "stdout:" in manifest["process"]["stdout"]
    assert "stderr:self-check" in manifest["process"]["stderr"]
    assert manifest["provenance"]["worktree_path"] == str(rewrite_worktree_path)

    selected_env = manifest["environment"]["selected"]
    for key in ("SPARK_HOME", "SPARK_FLOWS_DIR", "ATTRACTOR_CODEX_RUNTIME_ROOT", "CODEX_HOME"):
        assert key in selected_env

    spark_home = Path(manifest["paths"]["spark_home"])
    assert spark_home.is_relative_to(compat_validation_root)
    assert Path(manifest["paths"]["spark_flows_dir"]).is_relative_to(compat_validation_root)
    assert Path(manifest["paths"]["attractor_codex_runtime_root"]).is_relative_to(
        compat_validation_root
    )
    assert Path(manifest["paths"]["codex_home"]).is_relative_to(compat_validation_root)


def test_fixture_and_validation_layout_separates_reviewed_and_generated_roots(
    compat_fixture_root: Path,
    compat_validation_root: Path,
    compat_sandbox: CompatSandbox,
) -> None:
    generated_capture = compat_validation_root / "generated" / "harness" / "self-check.json"
    cache_root = compat_validation_root / "cache"
    package_root = compat_validation_root / "package-artifacts"
    frontend_dist = compat_validation_root / "frontend-dist"

    assert compat_fixture_root != compat_validation_root
    assert generated_capture.is_relative_to(compat_validation_root)
    assert not generated_capture.is_relative_to(compat_fixture_root)
    for path in (
        compat_sandbox.spark_home,
        compat_sandbox.flows_dir,
        compat_sandbox.runtime_root,
        compat_sandbox.codex_home,
    ):
        assert path.is_relative_to(compat_sandbox.root)
        assert not path.is_relative_to(compat_fixture_root)
    for path in (cache_root, package_root, frontend_dist):
        assert path.is_relative_to(compat_validation_root)
        assert not path.is_relative_to(compat_fixture_root)


def test_observable_helper_apis_cover_process_http_sse_filesystem_and_state(
    tmp_path: Path,
) -> None:
    process = harness.observe_process(
        argv=("spark", "flow", "validate"),
        cwd=tmp_path,
        returncode=0,
        stdout="validated\n",
        stderr="",
        environment={"SPARK_HOME": str(tmp_path / "spark-home")},
    )
    harness.assert_process_observation(
        process,
        returncode=0,
        stdout_contains="validated",
        env_keys=("SPARK_HOME",),
    )

    http = harness.http_response_observation(
        method="get",
        path="/workspace/api/projects",
        status_code=200,
        headers={"content-type": "application/json"},
        body={"projects": []},
    )
    harness.assert_http_response(
        http,
        status_code=200,
        header=("content-type", "application/json"),
        body_key="projects",
    )

    envelope = harness.sse_envelope(
        event="run.updated",
        event_id="cursor-1",
        data={"run_id": "run-1", "status": "running"},
    )
    harness.assert_sse_envelope(envelope, event="run.updated", data_keys=("run_id", "status"))

    state_root = tmp_path / "state"
    state_root.mkdir()
    before = harness.filesystem_snapshot(state_root)
    json_path = state_root / "run.json"
    jsonl_path = state_root / "journal.jsonl"
    toml_path = state_root / "settings.toml"
    json_path.write_text(json.dumps({"run_id": "run-1", "status": "ok"}), encoding="utf-8")
    jsonl_path.write_text('{"event":"started"}\n{"event":"finished"}\n', encoding="utf-8")
    toml_path.write_text('[profile]\nname = "default"\n', encoding="utf-8")
    after_create = harness.filesystem_snapshot(state_root)
    harness.assert_filesystem_effect(before, after_create, created=("run.json", "journal.jsonl", "settings.toml"))

    json_path.write_text(json.dumps({"run_id": "run-1", "status": "done"}), encoding="utf-8")
    after_change = harness.filesystem_snapshot(state_root)
    harness.assert_filesystem_effect(after_create, after_change, changed=("run.json",))

    assert harness.durable_state_snapshot(json_path)["data"]["status"] == "done"
    assert harness.durable_state_snapshot(jsonl_path)["records"][1]["event"] == "finished"
    assert harness.durable_state_snapshot(toml_path)["data"]["profile"]["name"] == "default"

    manifest_path = tmp_path / "manifest.json"
    harness.write_manifest(
        manifest_path,
        {
            "fixture_id": "harness/helper-self-check",
            "requirements": list(REQUIREMENTS),
            "decisions": list(DECISIONS),
        },
    )
    loaded = harness.load_manifest(manifest_path)
    harness.validate_manifest_coverage(
        loaded,
        requirement_ids=REQUIREMENTS,
        decision_ids=DECISIONS,
    )
