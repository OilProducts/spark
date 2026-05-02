from __future__ import annotations

import atexit
import json
import os
import shutil
import subprocess
import tempfile
from functools import lru_cache
from pathlib import Path


def _repo_paths() -> tuple[Path, Path]:
    repo_root = Path(__file__).resolve().parents[4]
    frontend_dir = repo_root / "frontend"
    return repo_root, frontend_dir


def _persistent_temp_dir(prefix: str) -> Path:
    temp_path = Path(tempfile.mkdtemp(prefix=prefix))
    atexit.register(shutil.rmtree, temp_path, ignore_errors=True)
    return temp_path


def _build_project_probe_artifact(
    *,
    source_path: Path,
    artifact_path: Path,
    temp_prefix: str,
    failure_context: str,
) -> Path:
    repo_root, frontend_dir = _repo_paths()
    temp_path = _persistent_temp_dir(prefix=temp_prefix)
    out_dir = temp_path / "out"
    out_dir.mkdir(parents=True, exist_ok=True)

    probe_tsconfig = temp_path / "tsconfig.json"
    probe_tsconfig.write_text(
        json.dumps(
            {
                "extends": (frontend_dir / "tsconfig.app.json").as_posix(),
                "compilerOptions": {
                    "noEmit": False,
                    "noEmitOnError": False,
                    "allowImportingTsExtensions": False,
                    "outDir": "./out",
                },
                "include": [source_path.as_posix()],
            },
            indent=2,
        )
        + "\n",
        encoding="utf-8",
    )

    compile_result = subprocess.run(
        [
            "npm",
            "--prefix",
            str(frontend_dir),
            "exec",
            "--",
            "tsc",
            "--pretty",
            "false",
            "--project",
            str(probe_tsconfig),
        ],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    )

    compiled_js = out_dir / artifact_path
    if not compiled_js.exists():
        raise AssertionError(
            f"Failed to compile {failure_context}.\n"
            f"stdout:\n{compile_result.stdout}\n"
            f"stderr:\n{compile_result.stderr}"
        )
    return compiled_js


def _build_direct_probe_artifact(
    *,
    source_path: Path,
    artifact_path: Path,
    temp_prefix: str,
    failure_context: str,
) -> Path:
    repo_root, frontend_dir = _repo_paths()
    out_dir = _persistent_temp_dir(prefix=temp_prefix) / "compiled"
    out_dir.mkdir(parents=True, exist_ok=True)

    compile_result = subprocess.run(
        [
            "npm",
            "--prefix",
            str(frontend_dir),
            "exec",
            "--",
            "tsc",
            "--pretty",
            "false",
            "--target",
            "ES2022",
            "--module",
            "ESNext",
            "--moduleResolution",
            "bundler",
            "--skipLibCheck",
            "--outDir",
            str(out_dir),
            str(source_path),
        ],
        cwd=repo_root,
        capture_output=True,
        text=True,
        check=False,
    )

    compiled_js = out_dir / artifact_path
    if not compiled_js.exists():
        raise AssertionError(
            f"Failed to compile {failure_context}.\n"
            f"stdout:\n{compile_result.stdout}\n"
            f"stderr:\n{compile_result.stderr}"
        )
    return compiled_js


@lru_cache(maxsize=1)
def _compile_dot_utils_js() -> Path:
    _, frontend_dir = _repo_paths()
    return _build_project_probe_artifact(
        source_path=frontend_dir / "src" / "lib" / "dotUtils.ts",
        artifact_path=Path("lib") / "dotUtils.js",
        temp_prefix=".tmp-dotutils-build-",
        failure_context="dotUtils.ts probe artifact",
    )


@lru_cache(maxsize=1)
def _compile_graph_attr_validation_js() -> Path:
    _, frontend_dir = _repo_paths()
    return _build_direct_probe_artifact(
        source_path=frontend_dir / "src" / "lib" / "graphAttrValidation.ts",
        artifact_path=Path("lib") / "graphAttrValidation.js",
        temp_prefix=".tmp-graph-attr-validation-build-",
        failure_context="graphAttrValidation.ts probe artifact",
    )


@lru_cache(maxsize=1)
def _compile_canonical_flow_model_js() -> Path:
    _, frontend_dir = _repo_paths()
    return _build_project_probe_artifact(
        source_path=frontend_dir / "src" / "lib" / "canonicalFlowModel.ts",
        artifact_path=Path("lib") / "canonicalFlowModel.js",
        temp_prefix=".tmp-canonical-flow-model-build-",
        failure_context="canonicalFlowModel.ts probe artifact",
    )


@lru_cache(maxsize=1)
def _compile_pipeline_start_payload_js() -> Path:
    _, frontend_dir = _repo_paths()
    return _build_direct_probe_artifact(
        source_path=frontend_dir / "src" / "lib" / "pipelineStartPayload.ts",
        artifact_path=Path("pipelineStartPayload.js"),
        temp_prefix=".tmp-pipeline-start-payload-build-",
        failure_context="pipelineStartPayload.ts probe artifact",
    )


def run_dot_utils_probe(
    probe_script: str,
    *,
    temp_prefix: str,
    error_context: str,
    env_extra: dict[str, str] | None = None,
) -> str:
    _, frontend_dir = _repo_paths()
    dot_utils_js = _compile_dot_utils_js()

    env = os.environ.copy()
    env["DOT_UTILS_JS_PATH"] = str(dot_utils_js)
    if env_extra:
        env.update(env_extra)

    probe_result = subprocess.run(
        ["node", "--input-type=module", "-e", probe_script],
        cwd=frontend_dir,
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    return probe_result.stdout


def run_pipeline_start_payload_probe(
    probe_script: str,
    *,
    temp_prefix: str,
    error_context: str,
    env_extra: dict[str, str] | None = None,
) -> str:
    _, frontend_dir = _repo_paths()
    pipeline_start_payload_js = _compile_pipeline_start_payload_js()

    env = os.environ.copy()
    env["PIPELINE_START_PAYLOAD_JS_PATH"] = str(pipeline_start_payload_js)
    if env_extra:
        env.update(env_extra)

    probe_result = subprocess.run(
        ["node", "--input-type=module", "-e", probe_script],
        cwd=frontend_dir,
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    return probe_result.stdout


def run_graph_attr_validation_probe(
    probe_script: str,
    *,
    temp_prefix: str,
    error_context: str,
) -> str:
    _, frontend_dir = _repo_paths()
    graph_attr_validation_js = _compile_graph_attr_validation_js()

    env = os.environ.copy()
    env["GRAPH_ATTR_VALIDATION_JS_PATH"] = str(graph_attr_validation_js)

    result = subprocess.run(
        ["node", "--input-type=module", "-e", probe_script],
        cwd=frontend_dir,
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    return result.stdout


def run_canonical_flow_model_probe(
    probe_script: str,
    *,
    temp_prefix: str,
    error_context: str,
    env_extra: dict[str, str] | None = None,
) -> str:
    _, frontend_dir = _repo_paths()
    canonical_flow_model_js = _compile_canonical_flow_model_js()

    env = os.environ.copy()
    env["CANONICAL_FLOW_MODEL_JS_PATH"] = str(canonical_flow_model_js)
    if env_extra:
        env.update(env_extra)

    probe_result = subprocess.run(
        ["node", "--input-type=module", "-e", probe_script],
        cwd=frontend_dir,
        capture_output=True,
        text=True,
        check=True,
        env=env,
    )
    return probe_result.stdout
