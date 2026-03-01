from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path


def test_graph_attr_validation_and_normalization_wiring_item_6_1_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    graph_attr_validation_text = (
        repo_root / "frontend" / "src" / "lib" / "graphAttrValidation.ts"
    ).read_text(encoding="utf-8")
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")
    graph_settings_text = (
        repo_root / "frontend" / "src" / "components" / "GraphSettings.tsx"
    ).read_text(encoding="utf-8")

    assert "GRAPH_FIDELITY_OPTIONS" in graph_attr_validation_text
    assert "normalizeGraphAttrValue" in graph_attr_validation_text
    assert "validateGraphAttrValue" in graph_attr_validation_text
    assert "default_max_retry" in graph_attr_validation_text
    assert "default_fidelity" in graph_attr_validation_text

    assert "normalizeGraphAttrValue(key, value)" in store_text
    assert "validateGraphAttrValue(key, normalizedValue)" in store_text
    assert "graphAttrErrors" in store_text

    assert 'type="number"' in graph_settings_text
    assert "graph-fidelity-options" in graph_settings_text
    assert "graphAttrErrors.default_max_retry" in graph_settings_text
    assert "graphAttrErrors.default_fidelity" in graph_settings_text


def _run_store_graph_attr_probe() -> dict[str, object]:
    repo_root = Path(__file__).resolve().parents[2]
    frontend_dir = repo_root / "frontend"

    with tempfile.TemporaryDirectory(prefix=".tmp-store-graph-attrs-", dir=frontend_dir) as temp_dir:
        out_dir = Path(temp_dir) / "compiled"
        out_dir.mkdir(parents=True, exist_ok=True)

        subprocess.run(
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
                str(frontend_dir / "src" / "store.ts"),
            ],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        )

        probe_script = """
import { pathToFileURL } from 'node:url'

const storage = new Map()
const localStorage = {
  getItem: (key) => (storage.has(key) ? storage.get(key) : null),
  setItem: (key, value) => { storage.set(key, String(value)) },
  removeItem: (key) => { storage.delete(key) },
}
globalThis.window = { localStorage }

const mod = await import(pathToFileURL(process.env.STORE_JS_PATH).href)
const state = mod.useStore.getState()

state.setGraphAttrs({
  goal: '  Ship release  ',
  default_max_retry: ' 007 ',
  default_fidelity: ' SUMMARY:HIGH ',
})
const afterSet = mod.useStore.getState()

afterSet.updateGraphAttr('default_max_retry', 'oops')
afterSet.updateGraphAttr('default_fidelity', 'unknown-mode')
afterSet.updateGraphAttr('retry_target', '  fix_stage  ')
const afterUpdate = mod.useStore.getState()

console.log(JSON.stringify({
  afterSetGraphAttrs: afterSet.graphAttrs,
  afterSetErrors: afterSet.graphAttrErrors,
  afterUpdateGraphAttrs: afterUpdate.graphAttrs,
  afterUpdateErrors: afterUpdate.graphAttrErrors,
}))
""".strip()

        env = os.environ.copy()
        env.update({"STORE_JS_PATH": str(out_dir / "store.js")})
        result = subprocess.run(
            ["node", "--input-type=module", "-e", probe_script],
            cwd=frontend_dir,
            check=True,
            capture_output=True,
            text=True,
            env=env,
        )
        return json.loads(result.stdout)


def test_store_normalizes_and_validates_graph_attrs_item_6_1_02() -> None:
    probe = _run_store_graph_attr_probe()

    after_set_graph_attrs = probe["afterSetGraphAttrs"]
    assert after_set_graph_attrs["goal"] == "Ship release"
    assert after_set_graph_attrs["default_max_retry"] == "7"
    assert after_set_graph_attrs["default_fidelity"] == "summary:high"
    assert probe["afterSetErrors"] == {}

    after_update_graph_attrs = probe["afterUpdateGraphAttrs"]
    assert after_update_graph_attrs["retry_target"] == "fix_stage"
    assert after_update_graph_attrs["default_max_retry"] == "oops"
    assert after_update_graph_attrs["default_fidelity"] == "unknown-mode"

    after_update_errors = probe["afterUpdateErrors"]
    assert "non-negative integer" in after_update_errors["default_max_retry"]
    assert "must be one of" in after_update_errors["default_fidelity"]


def test_checklist_marks_item_6_1_02_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [6.1-02]" in checklist_text
