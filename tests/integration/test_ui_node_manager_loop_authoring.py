from __future__ import annotations

import asyncio
import os
import subprocess
import tempfile
from pathlib import Path

import attractor.api.server as server


def _generate_dot_with_manager_loop_attrs() -> str:
    repo_root = Path(__file__).resolve().parents[2]
    frontend_dir = repo_root / "frontend"

    with tempfile.TemporaryDirectory(prefix=".tmp-dotutils-manager-loop-", dir=frontend_dir) as temp_dir:
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
  "include": ["../src/lib/dotUtils.ts"]
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
                "Failed to compile dotUtils.ts for manager-loop probe.\n"
                f"stdout:\n{compile_result.stdout}\n"
                f"stderr:\n{compile_result.stderr}"
            )

        probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.DOT_UTILS_JS_PATH).href)
const nodes = [
  {
    id: 'manager',
    data: {
      label: 'Manager',
      shape: 'house',
      type: 'stack.manager_loop',
      'manager.poll_interval': '25ms',
      'manager.max_cycles': '3',
      'manager.stop_condition': 'child.status == "success"',
      'manager.actions': 'observe,steer'
    }
  }
]
const dot = mod.generateDot('manager_loop_probe', nodes, [], {})
console.log(dot)
""".strip()

        env = os.environ.copy()
        env["DOT_UTILS_JS_PATH"] = str(dot_utils_js)
        probe_result = subprocess.run(
            ["node", "--input-type=module", "-e", probe_script],
            cwd=frontend_dir,
            capture_output=True,
            text=True,
            check=True,
            env=env,
        )
        return probe_result.stdout


def test_manager_loop_authoring_controls_present_item_6_2_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    sidebar_text = (repo_root / "frontend" / "src" / "components" / "Sidebar.tsx").read_text(encoding="utf-8")
    task_node_text = (repo_root / "frontend" / "src" / "components" / "TaskNode.tsx").read_text(encoding="utf-8")

    assert '<option value="house">Manager Loop</option>' in sidebar_text
    assert '<option value="house">Manager Loop</option>' in task_node_text

    assert "Manager Poll Interval" in sidebar_text
    assert "Manager Max Cycles" in sidebar_text
    assert "Manager Stop Condition" in sidebar_text
    assert "Manager Actions" in sidebar_text


def test_manager_loop_shape_and_type_are_selectable_item_6_7_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    sidebar_text = (repo_root / "frontend" / "src" / "components" / "Sidebar.tsx").read_text(encoding="utf-8")
    task_node_text = (repo_root / "frontend" / "src" / "components" / "TaskNode.tsx").read_text(encoding="utf-8")

    assert '<option value="house">Manager Loop</option>' in sidebar_text
    assert '<option value="house">Manager Loop</option>' in task_node_text
    assert '<option value="stack.manager_loop">stack.manager_loop</option>' in sidebar_text
    assert '<option value="stack.manager_loop">stack.manager_loop</option>' in task_node_text



def test_manager_loop_attrs_round_trip_through_preview_item_6_2_01() -> None:
    flow = _generate_dot_with_manager_loop_attrs()
    payload = asyncio.run(server.preview_pipeline(server.PreviewRequest(flow_content=flow)))
    nodes = payload["graph"]["nodes"]
    manager_node = next((node for node in nodes if node["id"] == "manager"), None)

    assert manager_node is not None
    assert manager_node["shape"] == "house"
    assert manager_node["type"] == "stack.manager_loop"
    assert manager_node["manager.poll_interval"] == "25ms"
    assert manager_node["manager.max_cycles"] == 3
    assert manager_node["manager.stop_condition"] == 'child.status == "success"'
    assert manager_node["manager.actions"] == "observe,steer"


def test_checklist_marks_item_6_2_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.2-01]" in checklist_text


def test_checklist_marks_item_6_7_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.7-01]" in checklist_text
