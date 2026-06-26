from __future__ import annotations

import json
import os
from pathlib import Path
from typing import Any, Mapping

import pytest

from tests.compat import harness
from tests.compat.conftest import CompatServer


ITEM_ID = "M0-I03-HTTP-SSE-FIXTURES"
REQUIREMENTS = ("RR-VAL-001", "RR-VAL-002")
DECISIONS = ("CD-RR-001", "CD-RR-013", "CD-RR-015")
AGENT_FLOW = "software-development/implement-change-request.dot"
SIMPLE_FLOW = "examples/simple-linear.dot"
VALID_DOT = """digraph Workflow {
  start [shape=Mdiamond];
  task [shape=box, prompt="Do work"];
  done [shape=Msquare];
  start -> task;
  task -> done;
}
"""
INVALID_DOT = "digraph Workflow { start -> }\n"


def test_http_route_fixtures_match_python_oracle(
    compat_server: CompatServer,
    compat_fixture_root: Path,
    pytestconfig: pytest.Config,
) -> None:
    scenarios: list[tuple[str, str, str, dict[str, Any]]] = [
        ("http/product-root-index", "GET", "/", {}),
        ("http/product-asset-success", "GET", "/assets/spark-app-icon.png", {}),
        ("http/product-favicon", "GET", "/favicon.ico", {}),
        ("http/product-root-status-404", "GET", "/status", {}),
        ("http/product-root-docs-404", "GET", "/docs", {}),
        ("http/product-api-no-html-404", "GET", "/workspace/api/missing", {}),
        ("http/product-asset-missing", "GET", "/assets/missing.png", {}),
        ("http/workspace-projects-list", "GET", "/workspace/api/projects", {}),
        (
            "http/workspace-project-register-success",
            "POST",
            "/workspace/api/projects/register",
            {"json_body": {"project_path": str(compat_server.project_dir)}},
        ),
        (
            "http/workspace-project-register-invalid",
            "POST",
            "/workspace/api/projects/register",
            {"json_body": {"project_path": ""}},
        ),
        ("http/workspace-settings-success", "GET", "/workspace/api/settings", {}),
        (
            "http/workspace-flows-agent-list",
            "GET",
            "/workspace/api/flows",
            {"params": {"surface": "agent"}},
        ),
        (
            "http/workspace-flow-raw-success",
            "GET",
            f"/workspace/api/flows/{SIMPLE_FLOW}/raw",
            {},
        ),
        (
            "http/workspace-flow-validate-success",
            "GET",
            f"/workspace/api/flows/{SIMPLE_FLOW}/validate",
            {},
        ),
        (
            "http/workspace-flow-describe-success",
            "GET",
            f"/workspace/api/flows/{SIMPLE_FLOW}",
            {},
        ),
        (
            "http/workspace-flow-missing",
            "GET",
            "/workspace/api/flows/missing.dot",
            {},
        ),
        (
            "http/workspace-flow-invalid-surface",
            "GET",
            "/workspace/api/flows",
            {"params": {"surface": "invalid"}},
        ),
        ("http/attractor-status", "GET", "/attractor/status", {}),
        ("http/attractor-runs-empty", "GET", "/attractor/runs", {}),
        (
            "http/attractor-preview-success",
            "POST",
            "/attractor/preview",
            {"json_body": {"flow_content": VALID_DOT}},
        ),
        (
            "http/attractor-preview-parse-error",
            "POST",
            "/attractor/preview",
            {"json_body": {"flow_content": INVALID_DOT}},
        ),
        ("http/attractor-flow-list", "GET", "/attractor/api/flows", {}),
        (
            "http/attractor-flow-get-success",
            "GET",
            f"/attractor/api/flows/{SIMPLE_FLOW}",
            {},
        ),
        (
            "http/attractor-flow-get-missing",
            "GET",
            "/attractor/api/flows/missing.dot",
            {},
        ),
        (
            "http/attractor-flow-save-parse-error",
            "POST",
            "/attractor/api/flows",
            {"json_body": {"name": "bad.dot", "content": INVALID_DOT}},
        ),
        (
            "http/attractor-pipeline-missing",
            "GET",
            "/attractor/pipelines/missing",
            {},
        ),
        (
            "http/deprecated-attractor-runs-events",
            "GET",
            "/attractor/runs/events",
            {},
        ),
        (
            "http/deprecated-workspace-conversation-events",
            "GET",
            f"/workspace/api/conversations/{compat_server.conversation_id}/events",
            {"params": {"project_path": str(compat_server.project_dir)}},
        ),
    ]

    for fixture_id, method, path, options in scenarios:
        manifest = compat_server.request_manifest(
            fixture_id=fixture_id,
            item_id=ITEM_ID,
            method=method,
            path=path,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            **options,
        )
        _assert_http_fixture(
            manifest,
            compat_fixture_root / f"{fixture_id}.json",
            pytestconfig,
        )


def test_http_trigger_and_webhook_route_fixtures_match_python_oracle(
    compat_server: CompatServer,
    compat_fixture_root: Path,
    pytestconfig: pytest.Config,
) -> None:
    create_payload = json.loads(compat_server.trigger_payload_path.read_text(encoding="utf-8"))

    _assert_http_fixture(
        compat_server.request_manifest(
            fixture_id="http/workspace-trigger-list-empty",
            item_id=ITEM_ID,
            method="GET",
            path="/workspace/api/triggers",
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
        ),
        compat_fixture_root / "http/workspace-trigger-list-empty.json",
        pytestconfig,
    )
    create_manifest = compat_server.request_manifest(
        fixture_id="http/workspace-trigger-create-webhook",
        item_id=ITEM_ID,
        method="POST",
        path="/workspace/api/triggers",
        requirements=REQUIREMENTS,
        decisions=DECISIONS,
        json_body=create_payload,
    )
    _assert_http_fixture(
        create_manifest,
        compat_fixture_root / "http/workspace-trigger-create-webhook.json",
        pytestconfig,
    )
    trigger = create_manifest["response"]["body"]["json"]
    trigger_id = str(trigger["id"])
    webhook_key = str(trigger["source"]["webhook_key"])
    webhook_secret = str(trigger["webhook_secret"])

    follow_up = [
        (
            "http/workspace-trigger-describe",
            "GET",
            f"/workspace/api/triggers/{trigger_id}",
            {},
        ),
        (
            "http/workspace-trigger-update",
            "PATCH",
            f"/workspace/api/triggers/{trigger_id}",
            {"json_body": {"name": "Compat webhook updated", "regenerate_webhook_secret": True}},
        ),
        (
            "http/workspace-webhook-missing-headers",
            "POST",
            "/workspace/api/webhooks",
            {"json_body": {"payload": "compat"}},
        ),
        (
            "http/workspace-webhook-bad-secret",
            "POST",
            "/workspace/api/webhooks",
            {
                "headers": {
                    "X-Spark-Webhook-Key": webhook_key,
                    "X-Spark-Webhook-Secret": "not-the-secret",
                },
                "json_body": {"payload": "compat"},
            },
        ),
    ]
    update_manifest: Mapping[str, Any] | None = None
    for fixture_id, method, path, options in follow_up:
        manifest = compat_server.request_manifest(
            fixture_id=fixture_id,
            item_id=ITEM_ID,
            method=method,
            path=path,
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            **options,
        )
        _assert_http_fixture(manifest, compat_fixture_root / f"{fixture_id}.json", pytestconfig)
        if fixture_id == "http/workspace-trigger-update":
            update_manifest = manifest

    if update_manifest is not None:
        updated = update_manifest["response"]["body"]["json"]
        webhook_secret = str(updated["webhook_secret"])

    _assert_http_fixture(
        compat_server.request_manifest(
            fixture_id="http/workspace-webhook-success",
            item_id=ITEM_ID,
            method="POST",
            path="/workspace/api/webhooks",
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
            headers={
                "X-Spark-Webhook-Key": webhook_key,
                "X-Spark-Webhook-Secret": webhook_secret,
            },
            json_body={"payload": "compat"},
            timeout=20.0,
        ),
        compat_fixture_root / "http/workspace-webhook-success.json",
        pytestconfig,
    )
    _assert_http_fixture(
        compat_server.request_manifest(
            fixture_id="http/workspace-trigger-delete",
            item_id=ITEM_ID,
            method="DELETE",
            path=f"/workspace/api/triggers/{trigger_id}",
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
        ),
        compat_fixture_root / "http/workspace-trigger-delete.json",
        pytestconfig,
    )
    _assert_http_fixture(
        compat_server.request_manifest(
            fixture_id="http/workspace-trigger-missing",
            item_id=ITEM_ID,
            method="GET",
            path="/workspace/api/triggers/missing",
            requirements=REQUIREMENTS,
            decisions=DECISIONS,
        ),
        compat_fixture_root / "http/workspace-trigger-missing.json",
        pytestconfig,
    )


def _assert_http_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    pytestconfig: pytest.Config,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=REQUIREMENTS,
        decision_ids=DECISIONS,
    )
    if _update_goldens(pytestconfig):
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_http_manifest_matches_golden(manifest, expected)


def _update_goldens(pytestconfig: pytest.Config) -> bool:
    return bool(pytestconfig.getoption("--compat-update-goldens")) or os.environ.get(
        "SPARK_COMPAT_UPDATE_GOLDENS"
    ) == "1"
