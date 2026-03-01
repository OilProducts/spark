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


def test_checklist_marks_item_8_1_01_complete() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    checklist_text = (repo_root / "ui-implementation-checklist.md").read_text(encoding="utf-8")

    assert "- [x] [8.1-01]" in checklist_text


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
