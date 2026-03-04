from __future__ import annotations

import json

from tests.contracts.frontend._support.dot_probe import run_canonical_flow_model_probe
from tests.contracts.frontend._support.preview_api import preview_pipeline


def _probe_canonical_flow_model_mapping() -> dict[str, object]:
    probe_script = """
import { pathToFileURL } from 'node:url'
const mod = await import(pathToFileURL(process.env.CANONICAL_FLOW_MODEL_JS_PATH).href)

const previewGraph = {
  graph_attrs: {
    goal: 'Ship release',
    'ext.graph_flag': 'true'
  },
  defaults: {
    node: {
      timeout: '5m',
      'ext.node_default': 'keep'
    },
    edge: {
      weight: 3,
      'ext.edge_default': 'keep'
    }
  },
  subgraphs: [
    {
      id: 'cluster_review',
      attrs: {
        label: 'Review',
        'ext.scope_flag': 'loop'
      },
      node_ids: ['author'],
      defaults: {
        node: {
          thread_id: 'review-thread'
        },
        edge: {
          weight: 7
        }
      },
      subgraphs: [
        {
          id: 'cluster_inner',
          attrs: {
            label: 'Inner'
          },
          node_ids: ['author'],
          defaults: {
            node: {
              timeout: '45s'
            },
            edge: {}
          },
          subgraphs: []
        }
      ]
    }
  ],
  nodes: [
    {
      id: 'author',
      label: 'Author',
      shape: 'box',
      prompt: 'Draft implementation plan',
      'ext.node_flag': 'retain-me'
    }
  ],
  edges: [
    {
      from: 'author',
      to: 'approve',
      label: 'submit',
      condition: 'ready=true',
      'ext.edge_flag': 'retain-me'
    }
  ]
}

const fromPreview = mod.buildCanonicalFlowModelFromPreviewGraph('flow_model_probe', previewGraph, {
  rawDot: 'digraph flow_model_probe { author -> approve }'
})

const fromEditor = mod.buildCanonicalFlowModelFromEditorState('flow_model_probe', {
  graphAttrs: {
    goal: 'Ship release',
    default_max_retry: 3,
    'ext.graph_scope': 'custom'
  },
  nodes: [
    {
      id: 'author',
      data: {
        label: 'Author',
        shape: 'box',
        type: 'codergen',
        prompt: 'Draft implementation plan',
        status: 'idle',
        'ext.node_scope': 'custom'
      }
    }
  ],
  edges: [
    {
      source: 'author',
      target: 'approve',
      data: {
        label: 'submit',
        condition: 'ready=true',
        weight: 2,
        'ext.edge_scope': 'custom'
      }
    }
  ],
  defaults: {
    node: {
      prompt: 'default prompt',
      'ext.node_default_scope': 'custom'
    },
    edge: {
      weight: 1,
      'ext.edge_default_scope': 'custom'
    }
  },
  subgraphs: [
    {
      id: 'cluster_review',
      attrs: {
        label: 'Review',
        'ui_extension.scope': 'review'
      },
      nodeIds: ['author'],
      defaults: {
        node: {
          timeout: '45s',
          'ext.node_default_scope': 'review'
        },
        edge: {
          weight: 8,
          'ext.edge_default_scope': 'review'
        }
      },
      subgraphs: [
        {
          id: 'cluster_inner',
          attrs: {
            label: 'Inner',
            'ui_extension.scope': 'inner'
          },
          nodeIds: ['author'],
          defaults: {
            node: {
              timeout: '15s'
            },
            edge: {}
          },
          subgraphs: []
        }
      ]
    }
  ]
})

const dot = mod.generateDotFromCanonicalFlowModel('flow_model_probe', fromEditor)
console.log(JSON.stringify({ fromPreview, fromEditor, dot }))
""".strip()

    output = run_canonical_flow_model_probe(
        probe_script,
        temp_prefix=".tmp-canonical-flow-model-",
        error_context="canonical flow model probe",
    )
    return json.loads(output)


def test_canonical_flow_model_captures_preview_and_editor_state_item_11_1_01() -> None:
    probe = _probe_canonical_flow_model_mapping()
    from_preview = probe["fromPreview"]
    from_editor = probe["fromEditor"]

    assert from_preview["graphAttrs"]["goal"] == "Ship release"
    assert from_preview["graphAttrs"]["ext.graph_flag"] == "true"
    assert from_preview["nodes"][0]["attrs"]["ext.node_flag"] == "retain-me"
    assert from_preview["edges"][0]["attrs"]["ext.edge_flag"] == "retain-me"
    assert from_preview["rawDot"] == "digraph flow_model_probe { author -> approve }"
    assert from_preview["defaults"]["node"]["timeout"] == "5m"
    assert from_preview["defaults"]["node"]["ext.node_default"] == "keep"
    assert from_preview["defaults"]["edge"]["weight"] == 3
    assert from_preview["defaults"]["edge"]["ext.edge_default"] == "keep"
    assert from_preview["subgraphs"][0]["id"] == "cluster_review"
    assert from_preview["subgraphs"][0]["attrs"]["ext.scope_flag"] == "loop"
    assert from_preview["subgraphs"][0]["nodeIds"] == ["author"]
    assert from_preview["subgraphs"][0]["defaults"]["node"]["thread_id"] == "review-thread"
    assert from_preview["subgraphs"][0]["subgraphs"][0]["id"] == "cluster_inner"
    assert from_preview["subgraphs"][0]["subgraphs"][0]["defaults"]["node"]["timeout"] == "45s"

    assert from_editor["graphAttrs"]["ext.graph_scope"] == "custom"
    assert from_editor["nodes"][0]["attrs"]["ext.node_scope"] == "custom"
    assert "status" not in from_editor["nodes"][0]["attrs"]
    assert from_editor["edges"][0]["attrs"]["ext.edge_scope"] == "custom"
    assert from_editor["defaults"]["node"]["prompt"] == "default prompt"
    assert from_editor["defaults"]["edge"]["weight"] == 1
    assert from_editor["subgraphs"][0]["id"] == "cluster_review"


def test_canonical_flow_model_serializes_editor_state_without_losing_core_attrs_item_11_1_01() -> None:
    probe = _probe_canonical_flow_model_mapping()
    payload = preview_pipeline(probe["dot"])
    graph = payload["graph"]

    assert graph["graph_attrs"]["goal"] == "Ship release"
    assert graph["graph_attrs"]["default_max_retry"] == 3

    node = graph["nodes"][0]
    assert node["id"] == "author"
    assert node["type"] == "codergen"
    assert node["prompt"] == "Draft implementation plan"

    edge = graph["edges"][0]
    assert edge["from"] == "author"
    assert edge["to"] == "approve"
    assert edge["label"] == "submit"
    assert edge["condition"] == "ready=true"
    assert edge["weight"] == 2


def test_canonical_flow_model_serializes_scopes_and_extension_attrs_item_11_1_02() -> None:
    probe = _probe_canonical_flow_model_mapping()
    payload = preview_pipeline(probe["dot"])
    graph = payload["graph"]
    nodes_by_id = {node["id"]: node for node in graph["nodes"]}
    edge = graph["edges"][0]

    assert graph["graph_attrs"]["ext.graph_scope"] == "custom"

    assert graph["defaults"]["node"]["prompt"] == "default prompt"
    assert graph["defaults"]["node"]["ext.node_default_scope"] == "custom"
    assert graph["defaults"]["edge"]["weight"] == 1
    assert graph["defaults"]["edge"]["ext.edge_default_scope"] == "custom"

    assert nodes_by_id["author"]["ext.node_scope"] == "custom"
    assert edge["ext.edge_scope"] == "custom"

    assert len(graph["subgraphs"]) == 1
    review_scope = graph["subgraphs"][0]
    assert review_scope["id"] == "cluster_review"
    assert review_scope["attrs"]["ui_extension.scope"] == "review"
    assert review_scope["defaults"]["node"]["timeout"] == "45s"
    assert review_scope["defaults"]["node"]["ext.node_default_scope"] == "review"
    assert review_scope["defaults"]["edge"]["weight"] == 8
    assert review_scope["defaults"]["edge"]["ext.edge_default_scope"] == "review"
    assert review_scope["node_ids"] == ["author"]

    assert len(review_scope["subgraphs"]) == 1
    inner_scope = review_scope["subgraphs"][0]
    assert inner_scope["id"] == "cluster_inner"
    assert inner_scope["attrs"]["ui_extension.scope"] == "inner"
    assert inner_scope["defaults"]["node"]["timeout"] == "15s"
