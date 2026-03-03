from __future__ import annotations

import json
import os
import subprocess
import tempfile
from pathlib import Path


def test_runs_panel_renders_run_summary_fields_item_9_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_test_ids = [
        "data-testid=\"run-summary-panel\"",
        "data-testid=\"run-summary-status\"",
        "data-testid=\"run-summary-result\"",
        "data-testid=\"run-summary-flow-name\"",
        "data-testid=\"run-summary-started-at\"",
        "data-testid=\"run-summary-ended-at\"",
        "data-testid=\"run-summary-duration\"",
        "data-testid=\"run-summary-model\"",
        "data-testid=\"run-summary-working-directory\"",
        "data-testid=\"run-summary-project-path\"",
        "data-testid=\"run-summary-git-branch\"",
        "data-testid=\"run-summary-git-commit\"",
        "data-testid=\"run-summary-last-error\"",
        "data-testid=\"run-summary-token-usage\"",
    ]
    required_labels = [
        "Status:",
        "Result:",
        "Flow:",
        "Started:",
        "Ended:",
        "Duration:",
        "Model:",
        "Working Dir:",
        "Project Path:",
        "Git Branch:",
        "Git Commit:",
        "Last Error:",
        "Tokens:",
    ]

    for snippet in required_test_ids:
        assert snippet in runs_panel_text, f"missing run summary panel snippet: {snippet}"
    for label in required_labels:
        assert label in runs_panel_text, f"missing run summary label: {label}"


def test_runs_panel_project_path_prefers_project_metadata_item_9_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    assert "selectedRunSummary.project_path || activeProjectPath || '—'" in runs_panel_text


def test_ui_smoke_includes_populated_run_summary_visual_qa_item_9_1_01() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run summary panel renders populated metadata for item 9.1-01" in ui_smoke_text
    assert "08b-runs-panel-populated-summary.png" in ui_smoke_text


def _run_run_metadata_freshness_probe() -> dict[str, object]:
    repo_root = Path(__file__).resolve().parents[2]
    frontend_dir = repo_root / "frontend"

    with tempfile.TemporaryDirectory(prefix=".tmp-run-metadata-freshness-probe-", dir=frontend_dir) as temp_dir:
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
                str(frontend_dir / "src" / "lib" / "runMetadataFreshness.ts"),
            ],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        )

        probe_script = """
import { pathToFileURL } from 'node:url'

const mod = await import(pathToFileURL(process.env.RUN_METADATA_FRESHNESS_JS_PATH).href)
const staleAfterMs = 30000

const never = mod.computeRunMetadataFreshness({ isLoading: false, lastFetchedAtMs: null, nowMs: 1000, staleAfterMs })
const refreshing = mod.computeRunMetadataFreshness({ isLoading: true, lastFetchedAtMs: 0, nowMs: 1000, staleAfterMs })
const fresh = mod.computeRunMetadataFreshness({ isLoading: false, lastFetchedAtMs: 4000, nowMs: 12000, staleAfterMs })
const stale = mod.computeRunMetadataFreshness({ isLoading: false, lastFetchedAtMs: 4000, nowMs: 40001, staleAfterMs })

const labels = {
  never: mod.formatRunMetadataLastUpdated({ lastFetchedAtMs: null, nowMs: 1000 }),
  fresh: mod.formatRunMetadataLastUpdated({ lastFetchedAtMs: 9000, nowMs: 14000 }),
  stale: mod.formatRunMetadataLastUpdated({ lastFetchedAtMs: 1000, nowMs: 70000 }),
}

console.log(JSON.stringify({ never, refreshing, fresh, stale, labels }))
""".strip()

        env = os.environ.copy()
        env.update(
            {
                "RUN_METADATA_FRESHNESS_JS_PATH": str(out_dir / "runMetadataFreshness.js"),
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


def test_run_metadata_freshness_states_item_9_1_02() -> None:
    probe = _run_run_metadata_freshness_probe()

    assert probe["never"] == "never"
    assert probe["refreshing"] == "refreshing"
    assert probe["fresh"] == "fresh"
    assert probe["stale"] == "stale"
    assert probe["labels"]["never"] == "Never refreshed"
    assert probe["labels"]["fresh"] == "Updated 5s ago"
    assert probe["labels"]["stale"] == "Updated 69s ago"


def test_runs_panel_exposes_refresh_and_stale_state_indicators_item_9_1_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    runs_panel_text = (repo_root / "frontend" / "src" / "components" / "RunsPanel.tsx").read_text(encoding="utf-8")

    required_snippets = [
        "computeRunMetadataFreshness",
        "formatRunMetadataLastUpdated",
        "setLastFetchedAtMs(Date.now())",
        "const metadataFreshness = computeRunMetadataFreshness({",
        "data-testid=\"run-metadata-freshness-indicator\"",
        "data-testid=\"run-metadata-last-updated\"",
        "Run metadata may be stale. Refresh to load the latest run status.",
    ]
    for snippet in required_snippets:
        assert snippet in runs_panel_text, f"missing run metadata refresh/stale indicator snippet: {snippet}"


def test_ui_smoke_includes_run_metadata_refresh_visual_qa_item_9_1_02() -> None:
    repo_root = Path(__file__).resolve().parents[2]
    ui_smoke_text = (repo_root / "frontend" / "e2e" / "ui-smoke.spec.ts").read_text(encoding="utf-8")

    assert "run summary metadata refresh and stale-state indicator for item 9.1-02" in ui_smoke_text
    assert "08c-runs-panel-refresh-stale-indicator.png" in ui_smoke_text
