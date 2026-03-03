from __future__ import annotations

from pathlib import Path


def test_run_initiation_captures_project_flow_source_workdir_and_backend_model_item_8_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    payload_builder_text = (repo_root / "frontend" / "src" / "lib" / "pipelineStartPayload.ts").read_text(
        encoding="utf-8"
    )

    required_snippets = [
        "const runInitiationForm = {",
        "projectPath: activeProjectPath || '',",
        "flowSource: activeFlow || '',",
        "workingDirectory: workingDir,",
        "backend: 'codex',",
        "model: model.trim() || null,",
        "data-testid=\"run-initiation-form\"",
        "data-testid=\"run-initiation-project\"",
        "data-testid=\"run-initiation-flow-source\"",
        "data-testid=\"run-initiation-working-directory\"",
        "data-testid=\"run-initiation-backend-model\"",
        "const flowRes = await fetch(`/api/flows/${encodeURIComponent(runInitiationForm.flowSource)}`)",
    ]

    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing run initiation capture snippet: {snippet}"

    payload_snippets = [
        "flow_content: flowContent,",
        "working_directory: form.workingDirectory,",
        "backend: form.backend,",
        "model: form.model,",
        "flow_name: form.flowSource || null,",
    ]
    for snippet in payload_snippets:
        assert snippet in payload_builder_text, f"missing run initiation payload snippet: {snippet}"


def test_run_initiation_payload_parity_with_pipelines_contract_item_8_1_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    server_text = (repo_root / "attractor" / "api" / "server.py").read_text(encoding="utf-8")
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    payload_builder_text = (repo_root / "frontend" / "src" / "lib" / "pipelineStartPayload.ts").read_text(
        encoding="utf-8"
    )
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    backend_contract_snippets = [
        "class PipelineStartRequest(BaseModel):",
        'flow_content: str = Field(validation_alias=AliasChoices("flow_content", "dot_source"))',
        'working_directory: str = "./workspace"',
        'backend: str = "codex"',
        "model: Optional[str] = None",
        "flow_name: Optional[str] = None",
    ]
    for snippet in backend_contract_snippets:
        assert snippet in server_text, f"missing /pipelines request contract snippet: {snippet}"

    payload_builder_snippets = [
        "export interface RunInitiationFormState {",
        "projectPath: string",
        "flowSource: string",
        "workingDirectory: string",
        "backend: string",
        "model: string | null",
        "export interface PipelineStartPayload {",
        "flow_content: string",
        "working_directory: string",
        "backend: string",
        "model: string | null",
        "flow_name: string | null",
        "export function buildPipelineStartPayload(",
        "flow_content: flowContent,",
        "working_directory: form.workingDirectory,",
        "backend: form.backend,",
        "model: form.model,",
        "flow_name: form.flowSource || null,",
    ]
    for snippet in payload_builder_snippets:
        assert snippet in payload_builder_text, f"missing payload parity snippet: {snippet}"

    navbar_snippets = [
        'import { buildPipelineStartPayload } from "@/lib/pipelineStartPayload"',
        "const startPayload = buildPipelineStartPayload(runInitiationForm, flow.content)",
        "body: JSON.stringify(startPayload)",
    ]
    for snippet in navbar_snippets:
        assert snippet in navbar_text, f"missing navbar payload builder usage snippet: {snippet}"

    assert "- [x] [8.1-02]" in checklist_text


def test_run_start_rejection_surfaces_failure_handling_ui_item_8_1_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    required_snippets = [
        "const [runStartError, setRunStartError] = useState<string | null>(null)",
        "let reason = `Run request failed (${runRes.status})`",
        "reason = detail.error.trim()",
        "reason = detail.detail.trim()",
        "throw new Error(`Run not started: ${reason}`)",
        "setRunStartError(error instanceof Error ? error.message : 'Failed to start pipeline run.')",
        "data-testid=\"run-start-error-banner\"",
        "Failed to start run:",
    ]
    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing run start rejection failure UI snippet: {snippet}"

    assert "- [x] [8.1-03]" in checklist_text


def test_run_initiation_working_directory_defaults_to_active_project_unless_overridden_item_8_1_04() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    required_snippets = [
        "const resolvedWorkingDirectory = runInitiationForm.workingDirectory.trim() || runInitiationForm.projectPath",
        "runInitiationForm.workingDirectory = resolvedWorkingDirectory",
    ]
    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing working directory defaulting snippet: {snippet}"

    assert "- [x] [8.1-04]" in checklist_text


def test_run_start_surfaces_git_policy_warning_path_item_8_1_05() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "const [runStartGitPolicyWarning, setRunStartGitPolicyWarning] = useState<string | null>(null)",
        "const metadataRes = await fetch(`/api/projects/metadata?directory=${encodeURIComponent(runInitiationForm.projectPath)}`)",
        "const branch = typeof metadata?.branch === 'string' ? metadata.branch.trim() : ''",
        "const warning = 'Project Git policy check failed: active project is not a Git repository.'",
        "setRunStartGitPolicyWarning(warning)",
        "const allowNonGitRun = window.confirm(`${warning} Continue with run start anyway?`)",
        "if (!allowNonGitRun) {",
        'data-testid="run-start-git-policy-warning-banner"',
    ]
    for snippet in required_snippets:
        assert snippet in navbar_text, f"missing git policy warning run-start snippet: {snippet}"


def test_run_initiation_payload_carries_project_spec_plan_linkage_for_traceability_item_9_6_03() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    server_text = (repo_root / "attractor" / "api" / "server.py").read_text(encoding="utf-8")
    navbar_text = (repo_root / "frontend" / "src" / "components" / "Navbar.tsx").read_text(encoding="utf-8")
    projects_panel_text = (repo_root / "frontend" / "src" / "components" / "ProjectsPanel.tsx").read_text(encoding="utf-8")
    payload_builder_text = (repo_root / "frontend" / "src" / "lib" / "pipelineStartPayload.ts").read_text(
        encoding="utf-8"
    )

    server_contract_snippets = [
        "spec_id: Optional[str] = None",
        "plan_id: Optional[str] = None",
    ]
    for snippet in server_contract_snippets:
        assert snippet in server_text, f"missing traceability request contract snippet: {snippet}"

    payload_builder_snippets = [
        "specArtifactId: string | null",
        "planArtifactId: string | null",
        "spec_id: form.specArtifactId,",
        "plan_id: form.planArtifactId,",
    ]
    for snippet in payload_builder_snippets:
        assert snippet in payload_builder_text, f"missing payload traceability snippet: {snippet}"

    launcher_snippets = [
        "specArtifactId: activeProjectScope?.specId || null,",
        "planArtifactId: activeProjectScope?.planId || null,",
    ]
    for snippet in launcher_snippets:
        assert snippet in navbar_text, f"missing navbar traceability launch snippet: {snippet}"
        assert snippet in projects_panel_text, f"missing projects traceability launch snippet: {snippet}"

