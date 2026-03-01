from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path


def _probe_node_visibility_runtime() -> dict[str, object]:
    repo_root = Path(__file__).resolve().parents[2]
    frontend_dir = repo_root / "frontend"

    with tempfile.TemporaryDirectory(prefix=".tmp-node-visibility-probe-", dir=frontend_dir) as temp_dir:
        temp_path = Path(temp_dir)
        out_dir = temp_path / "out"
        out_dir.mkdir(parents=True, exist_ok=True)

        probe_tsconfig = temp_path / "tsconfig.json"
        probe_tsconfig.write_text(
            """{
  "extends": "../tsconfig.app.json",
  "compilerOptions": {
    "noEmit": false,
    "noEmitOnError": false,
    "allowImportingTsExtensions": false,
    "outDir": "./out"
  },
  "include": ["../src/lib/nodeVisibility.ts"]
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

        node_visibility_js = out_dir / "lib" / "nodeVisibility.js"
        if not node_visibility_js.exists():
            node_visibility_js = out_dir / "nodeVisibility.js"
        if not node_visibility_js.exists():
            raise AssertionError(
                "Failed to compile nodeVisibility.ts for shape/type override probe.\n"
                f"stdout:\n{compile_result.stdout}\n"
                f"stderr:\n{compile_result.stderr}"
            )

        probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.NODE_VISIBILITY_JS_PATH).href)
const defaultHuman = mod.getHandlerType('hexagon', '')
const defaultStart = mod.getHandlerType('Mdiamond', '')
const result = {
  defaults: {
    human: defaultHuman,
    start: defaultStart,
    unknownShape: mod.getHandlerType('unknown-shape', ''),
  },
  overrides: {
    humanToTool: mod.getHandlerType('hexagon', 'tool'),
    startToCodergen: mod.getHandlerType('Mdiamond', 'codergen'),
  },
  visibility: {
    human: mod.getNodeFieldVisibility(defaultHuman),
    start: mod.getNodeFieldVisibility(defaultStart),
  }
}
console.log(JSON.stringify(result))
""".strip()

        env = os.environ.copy()
        env["NODE_VISIBILITY_JS_PATH"] = str(node_visibility_js)
        probe_result = subprocess.run(
            ["node", "--input-type=module", "-e", probe_script],
            cwd=frontend_dir,
            capture_output=True,
            text=True,
            check=True,
            env=env,
        )
        return json.loads(probe_result.stdout)


def test_shape_defaults_and_type_override_runtime_behavior_item_6_2_03() -> None:
    result = _probe_node_visibility_runtime()

    assert result["defaults"]["human"] == "wait.human"
    assert result["defaults"]["start"] == "start"
    assert result["defaults"]["unknownShape"] == "codergen"
    assert result["overrides"]["humanToTool"] == "tool"
    assert result["overrides"]["startToCodergen"] == "codergen"

    assert result["visibility"]["human"]["showTypeOverride"] is True
    assert result["visibility"]["start"]["showTypeOverride"] is True


def test_node_inspectors_expose_type_override_control_item_6_2_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    sidebar_text = (repo_root / "frontend" / "src" / "components" / "Sidebar.tsx").read_text(encoding="utf-8")
    task_node_text = (repo_root / "frontend" / "src" / "components" / "TaskNode.tsx").read_text(encoding="utf-8")
    node_visibility_text = (repo_root / "frontend" / "src" / "lib" / "nodeVisibility.ts").read_text(encoding="utf-8")

    assert "showTypeOverride" in node_visibility_text
    assert "showTypeOverride" in sidebar_text
    assert "showTypeOverride" in task_node_text
    assert "Handler Type" in sidebar_text
    assert "Handler Type" in task_node_text


def test_checklist_marks_item_6_2_03_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.2-03]" in checklist_text
