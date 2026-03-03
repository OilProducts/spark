from __future__ import annotations

import os
import subprocess
import tempfile
from pathlib import Path


def _repo_paths() -> tuple[Path, Path]:
    repo_root = Path(__file__).resolve().parents[4]
    frontend_dir = repo_root / "frontend"
    return repo_root, frontend_dir


def run_dot_utils_probe(
    probe_script: str,
    *,
    temp_prefix: str,
    error_context: str,
    env_extra: dict[str, str] | None = None,
) -> str:
    repo_root, frontend_dir = _repo_paths()

    with tempfile.TemporaryDirectory(prefix=temp_prefix, dir=frontend_dir) as temp_dir:
        temp_path = Path(temp_dir)
        out_dir = temp_path / "out"
        out_dir.mkdir(parents=True, exist_ok=True)

        probe_tsconfig = temp_path / "tsconfig.json"
        probe_tsconfig.write_text(
            """{
  \"extends\": \"../tsconfig.app.json\",
  \"compilerOptions\": {
    \"noEmit\": false,
    \"noEmitOnError\": false,
    \"allowImportingTsExtensions\": false,
    \"outDir\": \"./out\"
  },
  \"include\": [\"../src/lib/dotUtils.ts\"]
}
""",
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

        dot_utils_js = out_dir / "lib" / "dotUtils.js"
        if not dot_utils_js.exists():
            raise AssertionError(
                f"Failed to compile dotUtils.ts for {error_context}.\n"
                f"stdout:\n{compile_result.stdout}\n"
                f"stderr:\n{compile_result.stderr}"
            )

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


def run_graph_attr_validation_probe(
    probe_script: str,
    *,
    temp_prefix: str,
    error_context: str,
) -> str:
    repo_root, frontend_dir = _repo_paths()

    with tempfile.TemporaryDirectory(prefix=temp_prefix, dir=frontend_dir) as temp_dir:
        out_dir = Path(temp_dir) / "compiled"
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
                str(frontend_dir / "src" / "lib" / "graphAttrValidation.ts"),
            ],
            cwd=repo_root,
            capture_output=True,
            text=True,
            check=False,
        )

        graph_attr_validation_js = out_dir / "lib" / "graphAttrValidation.js"
        if not graph_attr_validation_js.exists():
            raise AssertionError(
                f"Failed to compile graphAttrValidation.ts for {error_context}.\n"
                f"stdout:\n{compile_result.stdout}\n"
                f"stderr:\n{compile_result.stderr}"
            )

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
