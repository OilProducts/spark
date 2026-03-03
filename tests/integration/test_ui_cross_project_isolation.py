import json
import os
import subprocess
import tempfile
from pathlib import Path


def test_store_clears_transient_runtime_context_on_project_switch_item_4_2_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    required_snippets = [
        "const isProjectSwitch = projectPath !== state.activeProjectPath",
        "runtimeStatus: isProjectSwitch ? 'idle' : state.runtimeStatus,",
        "nodeStatuses: isProjectSwitch ? {} : state.nodeStatuses,",
        "humanGate: isProjectSwitch ? null : state.humanGate,",
        "logs: isProjectSwitch ? [] : state.logs,",
        "selectedNodeId: isProjectSwitch ? null : state.selectedNodeId,",
        "selectedEdgeId: isProjectSwitch ? null : state.selectedEdgeId,",
    ]

    for snippet in required_snippets:
        assert snippet in store_text, f"missing project-switch leakage guard snippet: {snippet}"


def test_run_stream_scopes_status_hydration_to_active_project_item_4_2_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    run_stream_text = (repo_root / "frontend" / "src" / "components" / "RunStream.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const activeProjectPath = useStore((state) => state.activeProjectPath)",
        "const runBelongsToProjectScope = (runWorkingDirectory: string, projectPath: string | null) => {",
        "if (part === '..') {",
        "segments.pop()",
        "const statusRunInScope = runBelongsToProjectScope(lastWorkingDirectory, activeProjectPath)",
        "if (!selectedRunId && runId && statusRunInScope) {",
        "if (!selectedRunId && (!runId || !statusRunInScope)) {",
        "setRuntimeStatus('idle')",
    ]

    for snippet in required_snippets:
        assert snippet in run_stream_text, f"missing run-stream scope-guard snippet: {snippet}"


def test_runs_panel_scope_filter_canonicalizes_parent_path_segments_item_4_2_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const normalizeScopePath = (value: string) => {",
        "if (part === '..') {",
        "segments.pop()",
        "return `${prefix}${normalizedBody}`",
    ]

    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing runs-panel canonical scope normalization snippet: {snippet}"


def _run_store_registration_probe(actions: list[dict[str, object]]) -> dict[str, object]:
    repo_root = Path(__file__).resolve().parents[2]
    frontend_dir = repo_root / "frontend"

    with tempfile.TemporaryDirectory(prefix=".tmp-store-probe-", dir=frontend_dir) as temp_dir:
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

const actions = JSON.parse(process.env.ACTIONS)
const storage = new Map()
const localStorage = {
  getItem: (key) => (storage.has(key) ? storage.get(key) : null),
  setItem: (key, value) => { storage.set(key, String(value)) },
  removeItem: (key) => { storage.delete(key) },
}
globalThis.window = { localStorage }

const mod = await import(pathToFileURL(process.env.STORE_JS_PATH).href)
const actionResults = []
for (const action of actions) {
  const state = mod.useStore.getState()
  if (action.type === 'registerProject') {
    actionResults.push({
      type: action.type,
      value: action.value,
      result: state.registerProject(action.value),
    })
  }
}

const finalState = mod.useStore.getState()
console.log(JSON.stringify({
  actionResults,
  activeProjectPath: finalState.activeProjectPath,
  projectRegistryKeys: Object.keys(finalState.projectRegistry),
}))
""".strip()

        env = os.environ.copy()
        env.update(
            {
                "STORE_JS_PATH": str(out_dir / "store.js"),
                "ACTIONS": json.dumps(actions),
            }
        )

        result = subprocess.run(
            ["node", "--input-type=module", "-e", probe_script],
            cwd=frontend_dir,
            check=True,
            capture_output=True,
            text=True,
            env=env,
        )
        return json.loads(result.stdout)


def test_store_canonicalizes_project_paths_to_prevent_scope_alias_leakage_item_4_2_05() -> None:
    probe = _run_store_registration_probe(
        actions=[
            {"type": "registerProject", "value": "/tmp/project-alpha/.."},
            {"type": "registerProject", "value": "/tmp"},
        ]
    )

    first_result = probe["actionResults"][0]["result"]
    second_result = probe["actionResults"][1]["result"]
    assert first_result["ok"] is True
    assert first_result["normalizedPath"] == "/tmp"
    assert second_result["ok"] is False
    assert second_result["error"] == "Project already registered: /tmp"
    assert probe["activeProjectPath"] == "/tmp"
    assert probe["projectRegistryKeys"] == ["/tmp"]


def test_run_stream_rejects_selected_run_metadata_outside_active_project_scope_item_4_2_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    run_stream_text = (repo_root / "frontend" / "src" / "components" / "RunStream.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const selectedRunWorkingDirectory = typeof data?.working_directory === 'string' ? data.working_directory : ''",
        "const selectedRunInScope = runBelongsToProjectScope(selectedRunWorkingDirectory, activeProjectPath)",
        "if (!selectedRunInScope) {",
        "setSelectedRunId(null)",
        "setRuntimeStatus('idle')",
        "}, [selectedRunId, activeProjectPath, addLog, setNodeStatus, clearHumanGate, resetNodeStatuses, setHumanGate, setRuntimeStatus, setSelectedRunId])",
    ]

    for snippet in required_snippets:
        assert snippet in run_stream_text, f"missing run-stream selected-run scope-guard snippet: {snippet}"
