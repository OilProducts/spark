from __future__ import annotations

from pathlib import Path
from typing import Any, Mapping

from attractor.dsl import parse_dot
from attractor.engine.context import Context
from attractor.graph_prep import graph_attr_context_seed
from attractor.transforms import AttributeDefaultsTransform, GoalVariableTransform, ModelStylesheetTransform
from attractor.transforms.runtime_preamble import RuntimePreambleTransform
from tests.compat import harness
from tests.compat.conftest import ITEM_DECISIONS, ITEM_ID_M0_I04, ITEM_REQUIREMENTS


DEFAULTS_DOT = """
digraph Defaults {
  graph [goal="Preserve defaults", default_max_retries=4];
  start [shape=Mdiamond];
  task;
  done [shape=Msquare];
  start -> task -> done;
}
"""

GOAL_PREAMBLE_DOT = """
digraph GoalPreamble {
  graph [goal="Ship transform parity"];
  start [shape=Mdiamond];
  task [shape=box, prompt="Plan for $goal"];
  done [shape=Msquare];
  start -> task -> done;
}
"""

STYLESHEET_DOT = """
digraph Styles {
  graph [model_stylesheet="* { llm_model: base; llm_provider: generic; } box { reasoning_effort: medium; } .fast { llm_model: flash; } #review { llm_model: best; llm_provider: openai; reasoning_effort: high; }"];
  start [shape=Mdiamond];
  plan [shape=box, class="fast"];
  review [shape=box, class="fast", llm_model="explicit"];
  done [shape=Msquare];
  start -> plan -> review -> done;
}
"""

GRAPH_ATTR_CONTEXT_DOT = """
digraph ContextMirror {
  graph [goal="Mirror attrs", default_max_retries=3, release="2026.06"];
  start [shape=Mdiamond];
  done [shape=Msquare];
  start -> done;
}
"""


def test_transform_fixtures_match_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
    rewrite_worktree_path: Path,
) -> None:
    manifests = [
        _attribute_defaults_manifest(rewrite_worktree_path),
        _goal_runtime_preamble_manifest(),
        _stylesheet_manifest(),
        _graph_attr_context_manifest(),
    ]

    for manifest in manifests:
        _assert_transform_fixture(
            manifest,
            compat_fixture_root / f"{manifest['fixture_id']}.json",
            compat_update_goldens,
        )


def _attribute_defaults_manifest(rewrite_worktree_path: Path) -> dict[str, Any]:
    graph = parse_dot(DEFAULTS_DOT)
    AttributeDefaultsTransform().apply(graph)
    graph_attrs = harness.normalize_path_tokens(
        harness.dot_graph_payload(graph)["graph_attrs"],
        {"__WORKTREE__": rewrite_worktree_path},
    )
    return _manifest(
        fixture_id="dsl/transform-attribute-defaults",
        operation="attribute_defaults",
        input_payload={"dot": DEFAULTS_DOT},
        observation={
            "graph_attrs": graph_attrs,
            "task_attrs": harness.dot_graph_payload(graph)["nodes"]["task"]["attrs"],
            "edge_attrs": harness.dot_graph_payload(graph)["edges"][0]["attrs"],
        },
    )


def _goal_runtime_preamble_manifest() -> dict[str, Any]:
    graph = parse_dot(GOAL_PREAMBLE_DOT)
    GoalVariableTransform().apply(graph)
    context = Context(
        values={
            "graph.goal": "Ship transform parity",
            "internal.run_id": "run-transform",
            "context.release": "v1",
            "_attractor.node_outcomes": {"start": "success"},
        }
    )
    preamble = RuntimePreambleTransform().apply("summary:high", context, ["start"])
    return _manifest(
        fixture_id="dsl/transform-goal-runtime-preamble",
        operation="goal_variable_and_runtime_preamble",
        input_payload={"dot": GOAL_PREAMBLE_DOT},
        observation={
            "graph": harness.dot_graph_payload(graph),
            "preamble": preamble,
        },
    )


def _stylesheet_manifest() -> dict[str, Any]:
    graph = parse_dot(STYLESHEET_DOT)
    ModelStylesheetTransform().apply(graph)
    payload = harness.dot_graph_payload(graph)
    return _manifest(
        fixture_id="dsl/transform-stylesheet-precedence",
        operation="model_stylesheet",
        input_payload={"dot": STYLESHEET_DOT},
        observation={
            "plan_attrs": payload["nodes"]["plan"]["attrs"],
            "review_attrs": payload["nodes"]["review"]["attrs"],
        },
    )


def _graph_attr_context_manifest() -> dict[str, Any]:
    graph = parse_dot(GRAPH_ATTR_CONTEXT_DOT)
    seed = graph_attr_context_seed(graph)
    launch_context = {"context.release": "launch", "graph.goal": "launch should not win"}
    merged = {**seed, **launch_context}
    return _manifest(
        fixture_id="dsl/transform-graph-attrs-context-mirror",
        operation="graph_attr_context_seed",
        input_payload={"dot": GRAPH_ATTR_CONTEXT_DOT, "launch_context": launch_context},
        observation={
            "graph_attrs": harness.dot_graph_payload(graph)["graph_attrs"],
            "context_seed": seed,
            "merged_context": merged,
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
            "oracle": "python-transform-public-interfaces",
            "interfaces": [
                "attractor.transforms.AttributeDefaultsTransform",
                "attractor.transforms.GoalVariableTransform",
                "attractor.transforms.ModelStylesheetTransform",
                "attractor.transforms.RuntimePreambleTransform",
                "attractor.graph_prep.graph_attr_context_seed",
            ],
        },
        "operation": operation,
        "input": dict(input_payload),
        "observation": dict(observation),
    }


def _assert_transform_fixture(
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
