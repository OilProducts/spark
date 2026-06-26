from __future__ import annotations

from pathlib import Path
from typing import Any, Mapping

from fastapi import HTTPException

from attractor.api.flow_sources import normalize_flow_name
from attractor.dsl import DotParseError, parse_dot, validate_graph
from attractor.dsl.formatter import format_readable_dot
from attractor.validation_preview import preview_dot_source
from tests.compat import harness
from tests.compat.conftest import ITEM_DECISIONS, ITEM_ID_M0_I04, ITEM_REQUIREMENTS


VALID_TYPED_DEFAULTS_DOT = """
digraph Workflow {
  graph [goal="Ship $goal", default_max_retries=2, default_fidelity="summary:high"];
  node [shape=box, max_retries=1];
  edge [weight=1, condition="outcome=success"];

  start [shape=Mdiamond];
  subgraph cluster_review {
    graph [label="Review cluster"];
    review [prompt="Review typed defaults", timeout=5s, auto_status=true];
  }
  task [prompt="Task", score=1.5, enabled=false];
  done [shape=Msquare];

  start -> task -> review [label="go", weight=2];
  review -> done;
}
"""

SPARK_EXTENSION_ATTRS_DOT = r"""
digraph SparkExtensions {
  graph [
    goal="Preserve Spark attrs",
    spark.title="Compatibility flow",
    spark.description="Recorded for Rust parity",
    spark.launch_inputs="[{\"key\":\"context.topic\",\"type\":\"string\",\"required\":true}]",
    spark.result_node="summarize",
    spark.result_summary_title="Summary",
    spark.result_summary_body="Done",
    ui_default_goal="Ship it"
  ];
  start [shape=Mdiamond];
  summarize [
    shape=box,
    prompt="Summarize",
    spark.reads_context="[\"context.topic\"]",
    spark.writes_context="[\"context.summary\"]"
  ];
  done [shape=Msquare];
  start -> summarize -> done;
}
"""

FORMAT_BRANCHING_DOT = r"""
digraph Workflow {
  done [shape=Msquare];
  review [shape=diamond, prompt="Review \"quoted\" result"];
  build [shape=box, prompt="Build\nNow"];
  start [shape=Mdiamond];
  start -> build;
  build -> review [label="review", weight=5];
  review -> build [label="again", condition="preferred_label=again"];
  review -> done [label="ship", condition="preferred_label=ship"];
}
"""

VALIDATION_DIAGNOSTICS_DOT = """
digraph Broken {
  start [shape=Mdiamond];
  work [shape=box];
  missing_prompt [shape=box];
  unreachable [shape=box, prompt="Hidden"];
  done [shape=Msquare];

  work -> start;
  start -> work [fidelity="warp"];
  work -> missing [retry_target="nowhere"];
  missing_prompt -> done;
  done -> work;
}
"""

LAUNCH_CONTEXT_CONTRACTS_DOT = r"""
digraph ContractIssues {
  graph [spark.launch_inputs="{\"not\":\"a list\"}"];
  start [shape=Mdiamond];
  write [shape=box, prompt="Write", spark.reads_context="[\"bad key\"]", spark.writes_context="[\"_attractor.runtime.execution_mode\"]"];
  done [shape=Msquare];
  start -> write -> done;
}
"""

UNSUPPORTED_SOURCES = {
    "multiple_graphs": "digraph A { a } digraph B { b }",
    "strict": "strict digraph A { a -> b }",
    "undirected": "graph A { a -- b }",
    "html_label": "digraph A { a [label=<b>bad</b>] }",
    "port": "digraph A { a:port -> b }",
    "malformed_attrs": "digraph A { a [shape=box,,] }",
    "invalid_bare_node": "digraph A { 123bad [shape=box] }",
}


def test_dsl_fixtures_match_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    manifests = [
        _parse_manifest(
            fixture_id="dsl/parse-valid-typed-defaults",
            source=VALID_TYPED_DEFAULTS_DOT,
        ),
        _parse_manifest(
            fixture_id="dsl/parse-valid-spark-extension-attrs",
            source=SPARK_EXTENSION_ATTRS_DOT,
        ),
        _parse_rejection_manifest(),
        _format_manifest(),
        _validation_manifest(
            fixture_id="dsl/validate-structure-diagnostics",
            source=VALIDATION_DIAGNOSTICS_DOT,
        ),
        _validation_manifest(
            fixture_id="dsl/validate-launch-context-contracts",
            source=LAUNCH_CONTEXT_CONTRACTS_DOT,
        ),
        _flow_name_manifest(),
        _preview_manifest(),
    ]

    for manifest in manifests:
        _assert_dsl_fixture(
            manifest,
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def _parse_manifest(*, fixture_id: str, source: str) -> dict[str, Any]:
    graph = parse_dot(source)
    return _manifest(
        fixture_id=fixture_id,
        operation="parse",
        input_payload={"dot": source},
        observation={
            "result": "accepted",
            "graph": harness.dot_graph_payload(graph),
        },
    )


def _parse_rejection_manifest() -> dict[str, Any]:
    cases: list[dict[str, Any]] = []
    for case_id, source in sorted(UNSUPPORTED_SOURCES.items()):
        try:
            parse_dot(source)
        except DotParseError as exc:
            cases.append(
                {
                    "case_id": case_id,
                    "result": "rejected",
                    "error_type": exc.__class__.__name__,
                    "message": str(exc),
                    "line": exc.line,
                }
            )
        else:
            cases.append({"case_id": case_id, "result": "accepted"})
    return _manifest(
        fixture_id="dsl/parse-reject-unsupported-constructs",
        operation="parse_rejections",
        input_payload={"cases": UNSUPPORTED_SOURCES},
        observation={"cases": cases},
    )


def _format_manifest() -> dict[str, Any]:
    graph = parse_dot(FORMAT_BRANCHING_DOT)
    first = format_readable_dot(graph)
    second = format_readable_dot(parse_dot(first))
    return _manifest(
        fixture_id="dsl/format-canonical-branching",
        operation="format_readable",
        input_payload={"dot": FORMAT_BRANCHING_DOT},
        observation={
            "canonical_dot": first,
            "stable_after_reparse": second == first,
            "reparsed_graph": harness.dot_graph_payload(parse_dot(second)),
        },
    )


def _validation_manifest(*, fixture_id: str, source: str) -> dict[str, Any]:
    graph = parse_dot(source)
    diagnostics = validate_graph(graph)
    return _manifest(
        fixture_id=fixture_id,
        operation="validate",
        input_payload={"dot": source},
        observation={
            "diagnostics": harness.diagnostics_payload(diagnostics),
            "error_rules": [
                diagnostic["rule"]
                for diagnostic in harness.diagnostics_payload(diagnostics)
                if diagnostic["severity"] == "error"
            ],
        },
    )


def _flow_name_manifest() -> dict[str, Any]:
    accepted_inputs = [
        "examples/simple-linear",
        "nested/flow.dot",
        r"windows\path\flow",
    ]
    rejected_inputs = [
        "",
        "/absolute/path.dot",
        "../escape.dot",
        "nested/../escape.dot",
        "./same.dot",
        "folder/",
    ]
    accepted = [
        {"input": value, "normalized": normalize_flow_name(value)}
        for value in accepted_inputs
    ]
    rejected: list[dict[str, Any]] = []
    for value in rejected_inputs:
        try:
            normalize_flow_name(value)
        except HTTPException as exc:
            rejected.append(
                {
                    "input": value,
                    "status_code": exc.status_code,
                    "detail": exc.detail,
                }
            )
        else:
            rejected.append({"input": value, "accepted": True})
    return _manifest(
        fixture_id="dsl/flow-name-path-safety",
        operation="flow_name_normalization",
        input_payload={"accepted": accepted_inputs, "rejected": rejected_inputs},
        observation={"accepted": accepted, "rejected": rejected},
    )


def _preview_manifest() -> dict[str, Any]:
    graph, payload = preview_dot_source(SPARK_EXTENSION_ATTRS_DOT)
    bad_graph, bad_payload = preview_dot_source("digraph Broken { start -> }\n")
    return _manifest(
        fixture_id="dsl/preview-status-and-errors",
        operation="preview",
        input_payload={
            "accepted_dot": SPARK_EXTENSION_ATTRS_DOT,
            "parse_error_dot": "digraph Broken { start -> }\n",
        },
        observation={
            "accepted": {
                "graph": harness.dot_graph_payload(graph) if graph is not None else None,
                "payload": payload,
            },
            "parse_error": {
                "graph": harness.dot_graph_payload(bad_graph) if bad_graph is not None else None,
                "payload": bad_payload,
            },
        },
    )


def _manifest(
    *,
    fixture_id: str,
    operation: str,
    input_payload: Mapping[str, Any],
    observation: Mapping[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": "compat-dsl-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID_M0_I04,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "python-dsl-public-interfaces",
            "interfaces": [
                "attractor.dsl.parse_dot",
                "attractor.dsl.formatter.format_readable_dot",
                "attractor.dsl.validate_graph",
                "attractor.validation_preview.preview_dot_source",
                "attractor.api.flow_sources.normalize_flow_name",
            ],
        },
        "operation": operation,
        "input": dict(input_payload),
        "observation": dict(observation),
    }


def _assert_dsl_fixture(
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
    harness.assert_dsl_manifest_matches_golden(manifest, expected)
