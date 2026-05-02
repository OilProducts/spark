from __future__ import annotations

import json
import subprocess
from pathlib import Path

from tests.contracts.frontend._support import dot_probe


def _clear_probe_caches() -> None:
    dot_probe._compile_dot_utils_js.cache_clear()
    dot_probe._compile_graph_attr_validation_js.cache_clear()
    dot_probe._compile_canonical_flow_model_js.cache_clear()
    dot_probe._compile_pipeline_start_payload_js.cache_clear()


def test_probe_compilation_is_cached_per_target(monkeypatch, tmp_path: Path) -> None:
    repo_root = tmp_path / "repo"
    frontend_dir = repo_root / "frontend"
    frontend_dir.mkdir(parents=True, exist_ok=True)

    temp_dir_counter = {"value": 0}
    compile_targets: list[str] = []
    node_envs: list[dict[str, str]] = []

    def fake_mkdtemp(*, prefix: str) -> str:
        temp_dir_counter["value"] += 1
        temp_dir = tmp_path / f"{prefix.strip('.').replace('/', '-')}{temp_dir_counter['value']}"
        temp_dir.mkdir(parents=True, exist_ok=True)
        return str(temp_dir)

    def fake_run(
        command: list[str],
        *,
        cwd: Path,
        capture_output: bool,
        text: bool,
        check: bool,
        env: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        del cwd, capture_output, text, check

        if command[:4] == ["npm", "--prefix", str(frontend_dir), "exec"]:
            if "--project" in command:
                project_path = Path(command[command.index("--project") + 1])
                tsconfig = json.loads(project_path.read_text(encoding="utf-8"))
                include_path = Path(tsconfig["include"][0]).name
                out_dir = project_path.parent / tsconfig["compilerOptions"]["outDir"]
            else:
                include_path = Path(command[-1]).name
                out_dir = Path(command[command.index("--outDir") + 1])

            target_name = include_path.removesuffix(".ts")
            compile_targets.append(target_name)
            compiled_paths = [out_dir / "lib" / f"{target_name}.js"]
            if target_name == "pipelineStartPayload":
                compiled_paths.append(out_dir / f"{target_name}.js")
            for compiled_js in compiled_paths:
                compiled_js.parent.mkdir(parents=True, exist_ok=True)
                compiled_js.write_text("export {}", encoding="utf-8")
            return subprocess.CompletedProcess(command, 0, stdout="", stderr="")

        if command[:2] == ["node", "--input-type=module"]:
            node_envs.append(env or {})
            return subprocess.CompletedProcess(command, 0, stdout="probe-ok\n", stderr="")

        raise AssertionError(f"Unexpected command: {command}")

    monkeypatch.setattr(dot_probe, "_repo_paths", lambda: (repo_root, frontend_dir))
    monkeypatch.setattr(dot_probe.tempfile, "mkdtemp", fake_mkdtemp)
    monkeypatch.setattr(dot_probe.subprocess, "run", fake_run)

    _clear_probe_caches()
    try:
        assert (
            dot_probe.run_dot_utils_probe(
                "console.log('dot')",
                temp_prefix=".tmp-dotutils-1-",
                error_context="dot utils first run",
                env_extra={"PROBE_LABEL": "first"},
            )
            == "probe-ok\n"
        )
        assert (
            dot_probe.run_dot_utils_probe(
                "console.log('dot again')",
                temp_prefix=".tmp-dotutils-2-",
                error_context="dot utils second run",
            )
            == "probe-ok\n"
        )
        assert (
            dot_probe.run_graph_attr_validation_probe(
                "console.log('graph attr')",
                temp_prefix=".tmp-graph-attr-1-",
                error_context="graph attr first run",
            )
            == "probe-ok\n"
        )
        assert (
            dot_probe.run_graph_attr_validation_probe(
                "console.log('graph attr again')",
                temp_prefix=".tmp-graph-attr-2-",
                error_context="graph attr second run",
            )
            == "probe-ok\n"
        )
        assert (
            dot_probe.run_canonical_flow_model_probe(
                "console.log('canonical')",
                temp_prefix=".tmp-canonical-1-",
                error_context="canonical first run",
                env_extra={"PREVIEW_JSON": "{}"},
            )
            == "probe-ok\n"
        )
        assert (
            dot_probe.run_canonical_flow_model_probe(
                "console.log('canonical again')",
                temp_prefix=".tmp-canonical-2-",
                error_context="canonical second run",
            )
            == "probe-ok\n"
        )
        assert (
            dot_probe.run_pipeline_start_payload_probe(
                "console.log('pipeline start payload')",
                temp_prefix=".tmp-pipeline-start-payload-1-",
                error_context="pipeline start payload first run",
            )
            == "probe-ok\n"
        )
        assert (
            dot_probe.run_pipeline_start_payload_probe(
                "console.log('pipeline start payload again')",
                temp_prefix=".tmp-pipeline-start-payload-2-",
                error_context="pipeline start payload second run",
                env_extra={"PAYLOAD_PROBE": "second"},
            )
            == "probe-ok\n"
        )
    finally:
        _clear_probe_caches()

    assert compile_targets == ["dotUtils", "graphAttrValidation", "canonicalFlowModel", "pipelineStartPayload"]
    assert len(node_envs) == 8
    assert "DOT_UTILS_JS_PATH" in node_envs[0]
    assert node_envs[0]["PROBE_LABEL"] == "first"
    assert "GRAPH_ATTR_VALIDATION_JS_PATH" in node_envs[2]
    assert "CANONICAL_FLOW_MODEL_JS_PATH" in node_envs[4]
    assert node_envs[4]["PREVIEW_JSON"] == "{}"
    assert "PIPELINE_START_PAYLOAD_JS_PATH" in node_envs[6]
    assert node_envs[7]["PAYLOAD_PROBE"] == "second"
