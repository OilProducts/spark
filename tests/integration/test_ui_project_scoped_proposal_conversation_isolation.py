import json
import os
import subprocess
import tempfile
from pathlib import Path


def _run_proposal_scope_probe() -> dict[str, object]:
    repo_root = Path(__file__).resolve().parents[2]
    frontend_dir = repo_root / "frontend"

    with tempfile.TemporaryDirectory(prefix=".tmp-proposal-scope-probe-", dir=frontend_dir) as temp_dir:
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
                str(frontend_dir / "src" / "lib" / "projectSpecProposals.ts"),
            ],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        )

        proposal_candidates = list(out_dir.rglob("projectSpecProposals.js"))
        if not proposal_candidates:
            raise AssertionError("failed to locate compiled proposal helper module")
        proposal_js_path = proposal_candidates[0]

        probe_script = """
import { pathToFileURL } from 'node:url'

const mod = await import(pathToFileURL(process.env.PROPOSAL_HELPER_JS_PATH).href)
const alphaPath = '/tmp/project-alpha'
const betaPath = '/tmp/project-beta'
const alphaProposal = {
  id: 'proposal-alpha',
  createdAt: '2026-03-01T00:00:00Z',
  summary: 'alpha summary',
  changes: [{ path: 'spec/goals.md#scope', before: 'before-a', after: 'after-a' }],
}
const betaProposal = {
  id: 'proposal-beta',
  createdAt: '2026-03-01T00:01:00Z',
  summary: 'beta summary',
  changes: [{ path: 'spec/goals.md#scope', before: 'before-b', after: 'after-b' }],
}

let proposals = {}
proposals = mod.upsertProjectSpecEditProposal(proposals, alphaPath, alphaProposal)
proposals = mod.upsertProjectSpecEditProposal(proposals, betaPath, betaProposal)

const alphaBeforeClear = mod.getProjectSpecEditProposal(proposals, alphaPath)
const betaBeforeClear = mod.getProjectSpecEditProposal(proposals, betaPath)
proposals = mod.clearProjectSpecEditProposal(proposals, alphaPath)
const alphaAfterClear = mod.getProjectSpecEditProposal(proposals, alphaPath)
const betaAfterClear = mod.getProjectSpecEditProposal(proposals, betaPath)

console.log(JSON.stringify({
  alphaBeforeClear,
  betaBeforeClear,
  alphaAfterClear,
  betaAfterClear,
  finalMap: proposals,
}))
""".strip()

        env = os.environ.copy()
        env["PROPOSAL_HELPER_JS_PATH"] = str(proposal_js_path)
        result = subprocess.run(
            ["node", "--input-type=module", "-e", probe_script],
            cwd=frontend_dir,
            check=True,
            capture_output=True,
            text=True,
            env=env,
        )

        return json.loads(result.stdout)


def _run_project_switch_isolation_probe() -> dict[str, object]:
    repo_root = Path(__file__).resolve().parents[2]
    frontend_dir = repo_root / "frontend"

    with tempfile.TemporaryDirectory(prefix=".tmp-project-scope-switch-probe-", dir=frontend_dir) as temp_dir:
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
                str(frontend_dir / "src" / "lib" / "projectSpecProposals.ts"),
            ],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        )

        store_candidates = list(out_dir.rglob("store.js"))
        proposal_candidates = list(out_dir.rglob("projectSpecProposals.js"))
        if not store_candidates or not proposal_candidates:
            raise AssertionError("failed to locate compiled probe modules for store/proposal helpers")
        store_js_path = store_candidates[0]
        proposal_js_path = proposal_candidates[0]

        probe_script = """
import { pathToFileURL } from 'node:url'

const routeKey = process.env.ROUTE_KEY
const storage = new Map()
const localStorage = {
  getItem: (key) => (storage.has(key) ? storage.get(key) : null),
  setItem: (key, value) => { storage.set(key, String(value)) },
  removeItem: (key) => { storage.delete(key) },
}
globalThis.window = { localStorage }
window.localStorage.setItem(routeKey, JSON.stringify({
  viewMode: 'projects',
  activeProjectPath: '/tmp/project-alpha',
  activeFlow: null,
  selectedRunId: null,
}))

const storeMod = await import(pathToFileURL(process.env.STORE_JS_PATH).href)
const proposalMod = await import(pathToFileURL(process.env.PROPOSAL_HELPER_JS_PATH).href)
const state = storeMod.useStore.getState()

state.setConversationId('conversation-alpha')
state.appendConversationHistoryEntry({
  role: 'user',
  content: 'alpha history item',
  timestamp: '2026-03-01T00:00:00Z',
})

state.setActiveProjectPath('/tmp/project-beta')
state.setConversationId('conversation-beta')
state.appendConversationHistoryEntry({
  role: 'user',
  content: 'beta history item',
  timestamp: '2026-03-01T00:01:00Z',
})

const alphaPath = '/tmp/project-alpha'
const betaPath = '/tmp/project-beta'
let proposals = {}
proposals = proposalMod.upsertProjectSpecEditProposal(proposals, alphaPath, {
  id: 'proposal-alpha',
  createdAt: '2026-03-01T00:02:00Z',
  summary: 'alpha proposal',
  changes: [{ path: 'spec/goals.md#scope', before: 'a1', after: 'a2' }],
})
proposals = proposalMod.upsertProjectSpecEditProposal(proposals, betaPath, {
  id: 'proposal-beta',
  createdAt: '2026-03-01T00:03:00Z',
  summary: 'beta proposal',
  changes: [{ path: 'spec/goals.md#scope', before: 'b1', after: 'b2' }],
})

const snapshotFor = (projectPath) => {
  state.setActiveProjectPath(projectPath)
  const currentState = storeMod.useStore.getState()
  const workspace = currentState.projectScopedWorkspaces[projectPath] ?? null
  const proposal = proposalMod.getProjectSpecEditProposal(proposals, projectPath)
  return {
    activeProjectPath: currentState.activeProjectPath,
    conversationId: workspace?.conversationId ?? null,
    conversationHistory: workspace?.conversationHistory ?? [],
    proposalId: proposal?.id ?? null,
  }
}

console.log(JSON.stringify({
  alphaSnapshot: snapshotFor(alphaPath),
  betaSnapshot: snapshotFor(betaPath),
}))
""".strip()

        env = os.environ.copy()
        env.update(
            {
                "ROUTE_KEY": "sparkspawn.ui_route_state",
                "STORE_JS_PATH": str(store_js_path),
                "PROPOSAL_HELPER_JS_PATH": str(proposal_js_path),
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


def test_projects_panel_scopes_conversation_context_and_proposals_to_active_project_item_5_5_06() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null",
        "const activeConversationHistory = activeProjectScope?.conversationHistory || []",
        "const [projectSpecEditProposals, setProjectSpecEditProposals] = useState<ProjectSpecEditProposalMap>({})",
        "const activeProjectProposalPreview = getProjectSpecEditProposal(projectSpecEditProposals, activeProjectPath)",
        "setProjectSpecEditProposals((current) => upsertProjectSpecEditProposal(current, activeProjectPath, proposal))",
        "setProjectSpecEditProposals((current) => clearProjectSpecEditProposal(current, activeProjectPath))",
    ]

    for snippet in required_snippets:
        assert snippet in projects_panel_text, f"missing project-isolated conversation/proposal snippet: {snippet}"


def test_store_persists_conversation_state_in_project_scoped_workspaces_item_5_5_06() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    store_text = (repo_root / "frontend" / "src" / "store.ts").read_text(encoding="utf-8")

    required_snippets = [
        "const PROJECT_CONVERSATION_STATE_STORAGE_KEY = \"sparkspawn.project_conversation_state\"",
        "const saveProjectConversationState = (projectScopedWorkspaces: Record<string, ProjectScopedWorkspace>) => {",
        "Object.entries(projectScopedWorkspaces).forEach(([projectPath, workspace]) => {",
        "persisted[projectPath] = {",
        "conversationId: workspace.conversationId,",
        "conversationHistory: workspace.conversationHistory,",
        "const scoped = resolveProjectScopedWorkspace(nextProjectScopedWorkspaces[state.activeProjectPath], state.activeProjectPath)",
    ]

    for snippet in required_snippets:
        assert snippet in store_text, f"missing project-scoped conversation persistence snippet: {snippet}"


def test_project_spec_edit_proposals_remain_isolated_by_project_item_5_5_06() -> None:
    probe = _run_proposal_scope_probe()

    assert probe["alphaBeforeClear"]["id"] == "proposal-alpha"
    assert probe["betaBeforeClear"]["id"] == "proposal-beta"
    assert probe["alphaAfterClear"] is None
    assert probe["betaAfterClear"]["id"] == "proposal-beta"
    assert "/tmp/project-alpha" not in probe["finalMap"]
    assert probe["finalMap"]["/tmp/project-beta"]["id"] == "proposal-beta"


def test_project_switch_keeps_conversation_and_proposal_artifacts_isolated_item_5_5_06() -> None:
    probe = _run_project_switch_isolation_probe()

    assert probe["alphaSnapshot"]["activeProjectPath"] == "/tmp/project-alpha"
    assert probe["alphaSnapshot"]["conversationId"] == "conversation-alpha"
    assert probe["alphaSnapshot"]["conversationHistory"] == [
        {
            "role": "user",
            "content": "alpha history item",
            "timestamp": "2026-03-01T00:00:00Z",
        }
    ]
    assert probe["alphaSnapshot"]["proposalId"] == "proposal-alpha"

    assert probe["betaSnapshot"]["activeProjectPath"] == "/tmp/project-beta"
    assert probe["betaSnapshot"]["conversationId"] == "conversation-beta"
    assert probe["betaSnapshot"]["conversationHistory"] == [
        {
            "role": "user",
            "content": "beta history item",
            "timestamp": "2026-03-01T00:01:00Z",
        }
    ]
    assert probe["betaSnapshot"]["proposalId"] == "proposal-beta"


def test_checklist_marks_item_5_5_06_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [5.5-06]" in checklist_text
