from __future__ import annotations

from pathlib import Path
import time
from typing import Any, Mapping

import httpx

from tests.compat import harness
from tests.compat.conftest import CompatServer, ITEM_DECISIONS, ITEM_ID_M0_I04, ITEM_REQUIREMENTS


RUN_DOT = """
digraph ApiRuntimeFixture {
  graph [goal="Capture API runtime durability", release="compat"];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
"""


def test_runtime_api_journal_fixtures_match_python_oracle(
    compat_server: CompatServer,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    run_id = "run-m0-i04-api"
    with httpx.Client(base_url=compat_server.base_url, timeout=10.0) as client:
        launch = _request_observation(
            client,
            method="POST",
            path="/attractor/pipelines",
            json_body={
                "run_id": run_id,
                "flow_content": RUN_DOT,
                "working_directory": str(compat_server.project_dir),
                "model": "compat-model",
                "launch_context": {"context.topic": "compat"},
            },
        )
        terminal = _wait_for_run_completion(client, run_id)
        journal = _request_observation(client, method="GET", path=f"/attractor/pipelines/{run_id}/journal")
        checkpoint = _request_observation(client, method="GET", path=f"/attractor/pipelines/{run_id}/checkpoint")
        context = _request_observation(client, method="GET", path=f"/attractor/pipelines/{run_id}/context")
        result = _request_observation(client, method="GET", path=f"/attractor/pipelines/{run_id}/result")
        artifacts = _request_observation(client, method="GET", path=f"/attractor/pipelines/{run_id}/artifacts")
        continue_guard = _request_observation(
            client,
            method="POST",
            path=f"/attractor/pipelines/{run_id}/continue",
            json_body={"flow_source_mode": "snapshot", "start_node": ""},
        )
        retry_guard = _request_observation(client, method="POST", path="/attractor/pipelines/missing/retry")
        cancel_guard = _request_observation(client, method="POST", path="/attractor/pipelines/missing/cancel")

    run_root = _find_run_root(compat_server, run_id)
    durable_snapshot = harness.normalize_path_tokens(
        harness.runtime_directory_snapshot(
            run_root,
            parse_json=[
                "run.json",
                "state.json",
                "checkpoint.json",
                "manifest.json",
            ],
            parse_jsonl=["events.jsonl"],
        ),
        {
            "__RUN_ROOT__": run_root,
            "__SPARK_HOME__": compat_server.spark_home,
            "__PROJECT_DIR__": compat_server.project_dir,
        },
    )

    manifests = [
        _manifest(
            fixture_id="runtime/api-run-lifecycle-journal",
            scenario="api_run_lifecycle_journal",
            input_payload={"dot": RUN_DOT, "run_id": run_id},
            api={
                "launch": launch,
                "terminal": terminal,
                "journal": journal,
            },
            durable_state={"run_root": durable_snapshot},
        ),
        _manifest(
            fixture_id="runtime/api-run-checkpoint-context",
            scenario="api_run_checkpoint_context",
            input_payload={"dot": RUN_DOT, "run_id": run_id},
            api={"checkpoint": checkpoint, "context": context},
            durable_state={"run_root": durable_snapshot},
        ),
        _manifest(
            fixture_id="runtime/api-run-result-artifacts",
            scenario="api_run_result_artifacts",
            input_payload={"dot": RUN_DOT, "run_id": run_id},
            api={"result": result, "artifacts": artifacts},
            durable_state={"run_root": durable_snapshot},
        ),
        _manifest(
            fixture_id="runtime/api-run-retry-continue-cancel-guards",
            scenario="api_run_retry_continue_cancel_guards",
            input_payload={"run_id": run_id},
            api={
                "continue_guard": continue_guard,
                "retry_missing": retry_guard,
                "cancel_missing": cancel_guard,
            },
        ),
    ]

    for manifest in manifests:
        _assert_runtime_fixture(
            harness.normalize_path_tokens(
                manifest,
                {
                    "__API_BASE_URL__": compat_server.base_url,
                    "__SPARK_HOME__": compat_server.spark_home,
                    "__PROJECT_DIR__": compat_server.project_dir,
                },
            ),
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def _request_observation(
    client: httpx.Client,
    *,
    method: str,
    path: str,
    json_body: Any = None,
) -> dict[str, Any]:
    kwargs: dict[str, Any] = {}
    request_body = None
    if json_body is not None:
        kwargs["json"] = json_body
        request_body = {"kind": "json", "json": json_body}
    response = client.request(method, path, **kwargs)
    return {
        "request": {
            "method": method.upper(),
            "path": path,
            "body": request_body,
        },
        "response": {
            "status_code": response.status_code,
            "headers": harness.selected_http_headers(response.headers),
            "body": harness.http_body_observation(
                content=response.content,
                headers=response.headers,
            ),
        },
    }


def _wait_for_run_completion(client: httpx.Client, run_id: str) -> dict[str, Any]:
    deadline = time.monotonic() + 20
    last_response: dict[str, Any] | None = None
    while time.monotonic() < deadline:
        last_response = _request_observation(client, method="GET", path=f"/attractor/pipelines/{run_id}")
        body = last_response["response"]["body"]
        if body.get("kind") == "json":
            payload = body.get("json")
            if isinstance(payload, dict) and payload.get("status") in {
                "completed",
                "failed",
                "validation_error",
                "canceled",
            }:
                return last_response
        time.sleep(0.2)
    raise AssertionError(f"run {run_id} did not reach a terminal state: {last_response}")


def _find_run_root(compat_server: CompatServer, run_id: str) -> Path:
    candidates = [
        path
        for path in compat_server.spark_home.rglob(run_id)
        if path.is_dir() and path.name == run_id
    ]
    assert candidates, f"run root not found for {run_id}"
    return sorted(candidates)[0]


def _manifest(
    *,
    fixture_id: str,
    scenario: str,
    input_payload: Mapping[str, Any],
    api: Mapping[str, Any],
    durable_state: Mapping[str, Any] | None = None,
) -> dict[str, Any]:
    return {
        "schema_version": "compat-runtime-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID_M0_I04,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "python-attractor-api-runtime",
            "interfaces": [
                "POST /attractor/pipelines",
                "GET /attractor/pipelines/{id}",
                "GET /attractor/pipelines/{id}/journal",
                "GET /attractor/pipelines/{id}/checkpoint",
                "GET /attractor/pipelines/{id}/context",
                "GET /attractor/pipelines/{id}/result",
                "GET /attractor/pipelines/{id}/artifacts",
                "durable run root files",
            ],
        },
        "scenario": scenario,
        "input": dict(input_payload),
        "api": dict(api),
        "durable_state": dict(durable_state or {}),
    }


def _assert_runtime_fixture(
    manifest: Mapping[str, Any],
    fixture_path: Path,
    update_goldens: bool,
) -> None:
    harness.validate_manifest_coverage(
        manifest,
        requirement_ids=ITEM_REQUIREMENTS,
        decision_ids=ITEM_DECISIONS,
    )
    if update_goldens:
        harness.write_manifest(fixture_path, manifest)
    expected = harness.load_manifest(fixture_path)
    harness.assert_runtime_manifest_matches_golden(manifest, expected)
