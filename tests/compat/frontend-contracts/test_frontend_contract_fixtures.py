from __future__ import annotations

import json
import os
from pathlib import Path
import subprocess
import textwrap
import uuid
from typing import Any, Mapping

from tests.compat import harness
from tests.compat.conftest import ITEM_DECISIONS, ITEM_ID_M0_I05, ITEM_REQUIREMENTS
from tests.contracts.frontend._support.behavior_bridge import (
    FRONTEND_CONTRACT_TEST_FILE,
    _run_frontend_contract_behavior_tests,
)


def test_frontend_behavior_contract_status_fixture_matches_python_oracle(
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    statuses = _run_frontend_contract_behavior_tests()
    status_counts = {
        status: sum(1 for value in statuses.values() if value == status)
        for status in sorted(set(statuses.values()))
    }
    manifest = _frontend_manifest(
        fixture_id="frontend/frontend-behavior-contract-status",
        scenario="frontend_behavior_contract_status",
        input_payload={
            "test_file": FRONTEND_CONTRACT_TEST_FILE,
            "command": [
                "npm",
                "--prefix",
                "frontend",
                "run",
                "test:unit",
                "--",
                "--run",
                FRONTEND_CONTRACT_TEST_FILE,
                "--reporter=json",
            ],
        },
        observation={
            "contract_ids": sorted(statuses),
            "status_by_contract_id": dict(sorted(statuses.items())),
            "status_counts": status_counts,
        },
    )
    _assert_frontend_fixture(
        manifest,
        compat_fixture_root / "frontend/frontend-behavior-contract-status.json",
        compat_update_goldens,
    )


def test_frontend_payload_contract_fixtures_match_python_oracle(
    tmp_path: Path,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    observations = _run_frontend_contract_probe(tmp_path)
    scenarios = {
        "frontend/canonical-flow-model-inputs": "canonical_flow_model_inputs",
        "frontend/editor-preview-save-payloads": "editor_preview_save_payloads",
        "frontend/launch-review-run-human-gate-payloads": "launch_review_run_human_gate_payloads",
        "frontend/api-wrapper-error-shapes": "api_wrapper_error_shapes",
        "frontend/app-shell-live-event-expectations": "app_shell_live_event_expectations",
    }
    for fixture_id, scenario in scenarios.items():
        manifest = _frontend_manifest(
            fixture_id=fixture_id,
            scenario=scenario,
            input_payload=observations[scenario]["input"],
            observation=observations[scenario]["observation"],
        )
        _assert_frontend_fixture(
            manifest,
            compat_fixture_root / f"{fixture_id}.json",
            compat_update_goldens,
        )


def test_frontend_trigger_crud_contract_fixture_matches_python_oracle(
    tmp_path: Path,
    compat_fixture_root: Path,
    compat_update_goldens: bool,
) -> None:
    observations = _run_frontend_contract_probe(tmp_path)
    manifest = _frontend_manifest(
        fixture_id="frontend/trigger-crud-payloads",
        scenario="trigger_crud_payloads",
        input_payload=observations["trigger_crud_payloads"]["input"],
        observation=observations["trigger_crud_payloads"]["observation"],
    )
    _assert_frontend_fixture(
        manifest,
        compat_fixture_root / "frontend/trigger-crud-payloads.json",
        compat_update_goldens,
    )


def _run_frontend_contract_probe(tmp_path: Path) -> dict[str, Any]:
    repo_root = Path(__file__).resolve().parents[3]
    frontend_dir = repo_root / "frontend"
    probe_dir = frontend_dir / "src" / "__tests__" / ".tmp-compat-probes"
    probe_dir.mkdir(parents=True, exist_ok=True)
    output_path = tmp_path / "frontend-contract-probe.json"
    probe_path = probe_dir / f"probe-{uuid.uuid4().hex}.test.ts"
    probe_path.write_text(_FRONTEND_PROBE_SOURCE, encoding="utf-8")

    env = os.environ.copy()
    env["SPARK_COMPAT_PROBE_OUTPUT"] = str(output_path)
    try:
        result = subprocess.run(
            [
                "npm",
                "exec",
                "--",
                "vitest",
                "run",
                "--config",
                "vitest.config.ts",
                str(probe_path.relative_to(frontend_dir)),
            ],
            cwd=frontend_dir,
            env=env,
            text=True,
            capture_output=True,
            check=False,
            timeout=60,
        )
    finally:
        probe_path.unlink(missing_ok=True)

    if result.returncode != 0:
        raise AssertionError(
            "Frontend compatibility probe failed.\n"
            f"stdout:\n{result.stdout}\n"
            f"stderr:\n{result.stderr}"
        )
    if not output_path.exists():
        raise AssertionError("Frontend compatibility probe did not write its JSON output.")
    loaded = json.loads(output_path.read_text(encoding="utf-8"))
    assert isinstance(loaded, dict)
    return loaded


def _frontend_manifest(
    *,
    fixture_id: str,
    scenario: str,
    input_payload: Mapping[str, Any],
    observation: Mapping[str, Any],
) -> dict[str, Any]:
    return {
        "schema_version": "compat-frontend-v1",
        "fixture_id": fixture_id,
        "item_id": ITEM_ID_M0_I05,
        "requirements": list(ITEM_REQUIREMENTS),
        "decisions": list(ITEM_DECISIONS),
        "provenance": {
            "oracle": "python-backed-typescript-frontend-contract",
            "interfaces": [
                "frontend Vitest contract runner",
                "public TypeScript API parser functions",
                "public TypeScript request builder functions",
                "workspace live-event URL helper",
            ],
        },
        "scenario": scenario,
        "input": dict(input_payload),
        "observation": dict(observation),
    }


def _assert_frontend_fixture(
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
    harness.assert_frontend_manifest_matches_golden(manifest, expected)


_FRONTEND_PROBE_SOURCE = textwrap.dedent(
    r"""
    import { describe, expect, it, vi } from 'vitest'
    import { writeFileSync } from 'node:fs'
    import {
      buildCanonicalFlowModelFromEditorState,
      buildCanonicalFlowModelFromPreviewGraph,
      generateDotFromCanonicalFlowModel,
    } from '@/lib/canonicalFlowModel'
    import {
      fetchPreviewValidated,
      parseArtifactListResponse,
      parsePipelineAnswerResponse,
      parsePipelineCheckpointResponse,
      parsePipelineContextResponse,
      parsePipelineResultResponse,
      parsePipelineStartResponse,
      parsePipelineStatusResponse,
      parsePipelineQuestionsResponse,
      parsePreviewResponse,
      parseRunJournalPageResponse,
      saveFlowValidated,
    } from '@/lib/api/attractorApi'
    import {
      parseConversationSnapshotResponse,
      parseConversationStreamEventResponse,
    } from '@/lib/api/conversationsApi'
    import { workspaceLiveEventsUrl } from '@/lib/api/apiClient'
    import {
      extractHttpError,
      parseHttpErrorDetail,
    } from '@/lib/api/shared'
    import { parseWorkspaceFlowResponse } from '@/lib/api/flowsApi'
    import {
      createTriggerValidated,
      deleteTriggerValidated,
      fetchTriggerListValidated,
      fetchTriggerValidated,
      parseTriggerListResponse,
      parseTriggerResponse,
      updateTriggerValidated,
    } from '@/lib/api/triggersApi'
    import {
      buildTriggerActionPayload,
      buildTriggerSourcePayload,
      triggerSourceSummary,
      triggerTargetSummary,
      triggerToFormState,
    } from '@/features/triggers/model/triggerForm'
    import {
      buildPipelineContinuePayload,
      buildPipelineStartPayload,
    } from '@/lib/pipelineStartPayload'

    const DOT_SOURCE = `digraph CompatFrontend {
      graph [
        goal="Ship frontend contract",
        spark.launch_inputs="[{\\"key\\":\\"context.ticket\\",\\"type\\":\\"string\\",\\"required\\":true}]"
      ];
      start [shape=Mdiamond];
      review [shape=box, prompt="Review contract", spark.writes_context="[\\"context.review\\"]"];
      done [shape=Msquare];
      start -> review [condition="outcome=success"];
      review -> done;
    }`

    function serializeError(error: unknown) {
      if (!(error instanceof Error)) {
        return { name: 'NonError', message: String(error) }
      }
      const payload: Record<string, unknown> = {
        name: error.name,
        message: error.message,
      }
      if ('endpoint' in error) {
        payload.endpoint = (error as { endpoint?: unknown }).endpoint
      }
      if ('status' in error) {
        payload.status = (error as { status?: unknown }).status
      }
      if ('detail' in error) {
        payload.detail = (error as { detail?: unknown }).detail
      }
      return payload
    }

    function buildCanonicalFixture() {
      const previewGraph = {
        graph_attrs: {
          goal: 'Ship frontend contract',
          'spark.launch_inputs': '[{"key":"context.ticket","type":"string","required":true}]',
          'spark.result_node': 'done',
        },
        defaults: {
          node: { timeout: '45s', 'spark.default_node': 'kept' },
          edge: { weight: 2, 'spark.default_edge': 'kept' },
        },
        subgraphs: [
          {
            id: 'cluster_review',
            attrs: { label: 'Review', 'spark.scope': 'review' },
            node_ids: ['review'],
            defaults: { node: { fidelity: 'summary:high' }, edge: { weight: 4 } },
            subgraphs: [],
          },
        ],
        nodes: [
          {
            id: 'review',
            label: 'Review contract',
            shape: 'box',
            prompt: 'Review contract',
            'spark.writes_context': '["context.review"]',
          },
        ],
        edges: [
          {
            from: 'start',
            to: 'review',
            condition: 'outcome=success',
            'spark.edge_flag': 'kept',
          },
        ],
      }
      const fromPreview = buildCanonicalFlowModelFromPreviewGraph('compat/frontend-flow', previewGraph, {
        rawDot: DOT_SOURCE,
      })
      const fromEditor = buildCanonicalFlowModelFromEditorState('compat/frontend-flow', {
        graphAttrs: {
          goal: 'Ship frontend contract',
          default_max_retries: 2,
          'spark.result_node': 'done',
        },
        nodes: [
          {
            id: 'review',
            data: {
              label: 'Review contract',
              shape: 'box',
              type: 'codergen',
              prompt: 'Review contract',
              status: 'idle',
              'spark.writes_context': '["context.review"]',
            },
          },
        ],
        edges: [
          {
            source: 'start',
            target: 'review',
            data: {
              condition: 'outcome=success',
              weight: 2,
              'spark.edge_flag': 'kept',
            },
          },
        ],
        defaults: {
          node: { timeout: '45s' },
          edge: { weight: 1 },
        },
        subgraphs: [
          {
            id: 'cluster_review',
            attrs: { label: 'Review', 'spark.scope': 'review' },
            nodeIds: ['review'],
            defaults: { node: { fidelity: 'summary:high' }, edge: { weight: 4 } },
            subgraphs: [],
          },
        ],
      })
      return {
        input: { flow_name: 'compat/frontend-flow', dot: DOT_SOURCE },
        observation: {
          fromPreview,
          fromEditor,
          generatedDot: generateDotFromCanonicalFlowModel('compat/frontend-flow', fromEditor),
        },
      }
    }

    async function buildPreviewSaveFixture() {
      const capturedRequests: Array<Record<string, unknown>> = []
      const previewPayload = {
        status: 'ok',
        graph: {
          graph_attrs: { goal: 'Ship frontend contract' },
          defaults: { node: {}, edge: {} },
          nodes: [{ id: 'review', label: 'Review contract', shape: 'box', prompt: 'Review contract' }],
          edges: [{ from: 'start', to: 'review', condition: 'outcome=success' }],
          subgraphs: [],
        },
        diagnostics: [
          { rule_id: 'compat.warning', severity: 'warning', message: 'Recorded warning', line: 3 },
        ],
      }
      const originalFetch = globalThis.fetch
      globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
        const bodyText = typeof init?.body === 'string' ? init.body : ''
        capturedRequests.push({
          url: String(url),
          method: init?.method ?? 'GET',
          headers: Object.fromEntries(new Headers(init?.headers).entries()),
          body: bodyText ? JSON.parse(bodyText) : null,
        })
        const responseBody = String(url).includes('/preview')
          ? previewPayload
          : { status: 'saved', name: 'compat/frontend-flow.dot' }
        return new Response(JSON.stringify(responseBody), {
          status: 200,
          headers: { 'Content-Type': 'application/json' },
        })
      })
      try {
        const parsedDirect = parsePreviewResponse(previewPayload, '/attractor/preview')
        const fetchedPreview = await fetchPreviewValidated(
          DOT_SOURCE,
          undefined,
          { flowName: 'compat/frontend-flow.dot', expandChildren: true },
        )
        const saveResult = await saveFlowValidated('compat/frontend-flow.dot', DOT_SOURCE, true)
        return {
          input: { dot: DOT_SOURCE, flow_name: 'compat/frontend-flow.dot' },
          observation: {
            parsedDirect,
            fetchedPreview,
            saveResult,
            capturedRequests,
          },
        }
      } finally {
        globalThis.fetch = originalFetch
      }
    }

    function buildLaunchRunHumanGateFixture() {
      const startPayload = buildPipelineStartPayload({
        projectPath: '/workspace/project',
        flowSource: 'compat/frontend-flow.dot',
        workingDirectory: 'services/api',
        model: 'gpt-5',
        llmProvider: 'openai',
        llmProfile: 'openai/gpt-5',
        reasoningEffort: 'medium',
        launchContext: { 'context.ticket': 'SPARK-123' },
        executionProfileId: 'native-fast',
        projectDefaultExecutionProfileId: 'default-profile',
      }, DOT_SOURCE)
      const continuePayload = buildPipelineContinuePayload({
        projectPath: '/workspace/project',
        workingDirectory: 'services/api',
        model: 'gpt-5',
        llmProvider: 'openai',
        llmProfile: 'openai/gpt-5',
        reasoningEffort: 'medium',
      }, {
        startNodeId: 'review',
        flowSourceMode: 'flow_name',
        flowName: 'compat/frontend-flow.dot',
      })
      const parsedStart = parsePipelineStartResponse({
        status: 'started',
        pipeline_id: 'run-frontend-compat',
        run_id: 'run-frontend-compat',
        working_directory: '/workspace/project/services/api',
        model: 'gpt-5',
        llm_provider: 'openai',
        llm_profile: 'openai/gpt-5',
        reasoning_effort: 'medium',
        execution_profile_id: 'native-fast',
        execution_lock: {
          scope: 'project',
          key: '/workspace/project',
          conflict_policy: 'queue',
          identity: 'run-frontend-compat',
          state: 'acquired',
          queue_position: null,
        },
      })
      const parsedStatus = parsePipelineStatusResponse({
        pipeline_id: 'run-frontend-compat',
        run_id: 'run-frontend-compat',
        flow_name: 'compat/frontend-flow.dot',
        status: 'waiting',
        outcome: null,
        working_directory: '/workspace/project/services/api',
        project_path: '/workspace/project',
        model: 'gpt-5',
        started_at: '2026-01-01T00:00:00Z',
        ended_at: null,
        current_node: 'review',
        completed_nodes: ['start'],
        progress: { current_node: 'review', completed_nodes: ['start'], completed_count: 1 },
      })
      const parsedJournal = parseRunJournalPageResponse({
        pipeline_id: 'run-frontend-compat',
        entries: [
          {
            id: 'event-1',
            sequence: 1,
            emitted_at: '2026-01-01T00:00:00Z',
            kind: 'node',
            raw_type: 'node_started',
            severity: 'info',
            summary: 'Review started',
            node_id: 'review',
            payload: { node_id: 'review' },
          },
        ],
        oldest_sequence: 1,
        newest_sequence: 1,
        has_older: false,
      })
      const parsedCheckpoint = parsePipelineCheckpointResponse({
        pipeline_id: 'run-frontend-compat',
        checkpoint: { active_node_id: 'review', context: { 'context.ticket': 'SPARK-123' } },
      })
      const parsedContext = parsePipelineContextResponse({
        pipeline_id: 'run-frontend-compat',
        context: { 'context.ticket': 'SPARK-123', 'context.review': 'pending' },
      })
      const parsedQuestions = parsePipelineQuestionsResponse({
        questions: [
          {
            id: 'approval',
            header: 'Approve',
            question: 'Ship this change?',
            options: [{ label: 'Approve', description: 'Continue the run.' }],
            allow_other: false,
          },
        ],
      })
      const parsedAnswer = parsePipelineAnswerResponse({
        status: 'answered',
        pipeline_id: 'run-frontend-compat',
        question_id: 'approval',
      })
      const parsedResult = parsePipelineResultResponse({
        run_id: 'run-frontend-compat',
        status: 'waiting',
        state: 'ready',
        source_node_id: 'review',
        source_artifact_path: 'artifacts/review.md',
        display_mode: 'summary',
        body_markdown: 'Compatibility result',
        summary_enabled: true,
        summary_prompt: 'Summarize',
        summary_error: null,
      })
      const parsedArtifacts = parseArtifactListResponse({
        pipeline_id: 'run-frontend-compat',
        artifacts: [
          { path: 'artifacts/review.md', size_bytes: 42, media_type: 'text/markdown', viewable: true },
        ],
      })
      const parsedConversation = parseConversationSnapshotResponse({
        schema_version: 1,
        revision: 3,
        conversation_id: 'conversation-frontend-compat',
        conversation_handle: 'calm-river',
        project_path: '/workspace/project',
        chat_mode: 'chat',
        provider: 'codex',
        model: 'gpt-5',
        title: 'Frontend compat',
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:01Z',
        turns: [
          {
            id: 'turn-1',
            role: 'assistant',
            content: 'Ready',
            timestamp: '2026-01-01T00:00:00Z',
            status: 'complete',
            kind: 'message',
          },
        ],
        segments: [
          {
            id: 'segment-1',
            turn_id: 'turn-1',
            order: 1,
            kind: 'request_user_input',
            role: 'assistant',
            status: 'pending',
            timestamp: '2026-01-01T00:00:00Z',
            updated_at: '2026-01-01T00:00:00Z',
            content: 'Approve?',
            request_user_input: {
              request_id: 'approval',
              status: 'pending',
              questions: [
                {
                  id: 'decision',
                  header: 'Approve',
                  question: 'Ship this change?',
                  question_type: 'MULTIPLE_CHOICE',
                  options: [{ label: 'Approve', description: 'Continue the run.' }],
                  allow_other: false,
                  is_secret: false,
                },
              ],
              answers: {},
            },
          },
        ],
        event_log: [{ message: 'Review requested', timestamp: '2026-01-01T00:00:00Z', kind: 'review' }],
        flow_run_requests: [
          {
            id: 'flow-run-request-frontend',
            created_at: '2026-01-01T00:00:00Z',
            updated_at: '2026-01-01T00:00:00Z',
            flow_name: 'compat/frontend-flow.dot',
            summary: 'Run frontend fixture',
            project_path: '/workspace/project',
            conversation_id: 'conversation-frontend-compat',
            source_turn_id: 'turn-1',
            status: 'pending',
            launch_context: { 'context.ticket': 'SPARK-123' },
          },
        ],
        flow_launches: [],
        proposed_plans: [
          {
            id: 'plan-frontend',
            created_at: '2026-01-01T00:00:00Z',
            updated_at: '2026-01-01T00:00:00Z',
            title: 'Plan',
            content: 'Do the work',
            project_path: '/workspace/project',
            conversation_id: 'conversation-frontend-compat',
            source_turn_id: 'turn-1',
            status: 'pending_review',
          },
        ],
      })
      return {
        input: { dot: DOT_SOURCE, run_id: 'run-frontend-compat' },
        observation: {
          startPayload,
          continuePayload,
          parsedStart,
          parsedStatus,
          parsedJournal,
          parsedCheckpoint,
          parsedContext,
          parsedQuestions,
          parsedAnswer,
          parsedResult,
          parsedArtifacts,
          parsedConversation,
        },
      }
    }

    async function buildApiErrorFixture() {
      const httpJson = await extractHttpError(
        new Response(JSON.stringify({ detail: { error: 'Webhook secret mismatch' } }), {
          status: 403,
          headers: { 'Content-Type': 'application/json' },
        }),
        '/workspace/api/webhooks',
      )
      const httpText = await extractHttpError(
        new Response('plain failure', { status: 500, headers: { 'Content-Type': 'text/plain' } }),
        '/attractor/preview',
      )
      let schemaFlow: Record<string, unknown>
      try {
        parseWorkspaceFlowResponse({ name: 5 }, '/workspace/api/flows/{name}')
        schemaFlow = { name: 'none' }
      } catch (error) {
        schemaFlow = serializeError(error)
      }
      let schemaPreview: Record<string, unknown>
      try {
        parsePreviewResponse({ status: 'ok', graph: { nodes: {}, edges: [] } }, '/attractor/preview')
        schemaPreview = { name: 'none' }
      } catch (error) {
        schemaPreview = serializeError(error)
      }
      return {
        input: {
          http_statuses: [403, 500],
          schema_endpoints: ['/workspace/api/flows/{name}', '/attractor/preview'],
        },
        observation: {
          httpJson: serializeError(httpJson),
          httpText: serializeError(httpText),
          schemaFlow,
          schemaPreview,
          detailExtraction: [
            parseHttpErrorDetail({ error: 'top-level error' }),
            parseHttpErrorDetail({ detail: 'detail string' }),
            parseHttpErrorDetail({ detail: { error: 'nested detail' } }),
            parseHttpErrorDetail({ detail: { note: 'ignored' } }),
          ],
        },
      }
    }

    function buildLiveEventFixture() {
      const params = new URLSearchParams()
      params.set('conversation_id', 'conversation-frontend-compat')
      params.set('run_id', 'run-frontend-compat')
      params.set('trigger_id', 'trigger-frontend-compat')
      params.set('cursor', '42')
      const turnEvent = parseConversationStreamEventResponse({
        type: 'turn_upsert',
        revision: 4,
        conversation_id: 'conversation-frontend-compat',
        project_path: '/workspace/project',
        title: 'Frontend compat',
        updated_at: '2026-01-01T00:00:02Z',
        turn: {
          id: 'turn-2',
          role: 'assistant',
          content: 'Updated',
          timestamp: '2026-01-01T00:00:02Z',
          status: 'complete',
          kind: 'message',
        },
      })
      const segmentEvent = parseConversationStreamEventResponse({
        type: 'segment_upsert',
        revision: 5,
        conversation_id: 'conversation-frontend-compat',
        project_path: '/workspace/project',
        title: 'Frontend compat',
        updated_at: '2026-01-01T00:00:03Z',
        segment: {
          id: 'segment-2',
          turn_id: 'turn-2',
          order: 2,
          kind: 'flow_run_request',
          role: 'assistant',
          status: 'complete',
          timestamp: '2026-01-01T00:00:03Z',
          updated_at: '2026-01-01T00:00:03Z',
          content: 'Run requested',
        },
        flow_run_requests: [
          {
            id: 'flow-run-request-live',
            created_at: '2026-01-01T00:00:03Z',
            updated_at: '2026-01-01T00:00:03Z',
            flow_name: 'compat/frontend-flow.dot',
            summary: 'Run frontend fixture',
            project_path: '/workspace/project',
            conversation_id: 'conversation-frontend-compat',
            source_turn_id: 'turn-2',
            status: 'approved',
            run_id: 'run-frontend-compat',
          },
        ],
      })
      const invalidMissingResource = parseConversationStreamEventResponse({
        type: 'turn_upsert',
        revision: 6,
        conversation_id: 'conversation-frontend-compat',
      })
      return {
        input: {
          query_params: Object.fromEntries(params.entries()),
          live_payload_types: ['turn_upsert', 'segment_upsert', 'missing-resource'],
        },
        observation: {
          url: workspaceLiveEventsUrl(params),
          turnEvent,
          segmentEvent,
          invalidMissingResource,
        },
      }
    }

    async function buildTriggerCrudFixture() {
      const activeProjectPath = '/workspace/project'
      const scheduleTrigger = {
        id: 'trigger-schedule',
        name: 'Schedule trigger',
        enabled: true,
        protected: false,
        source_type: 'schedule',
        created_at: '2026-01-01T00:00:00Z',
        updated_at: '2026-01-01T00:00:00Z',
        action: {
          flow_name: 'compat/frontend-flow.dot',
          project_path: activeProjectPath,
          static_context: { ticket: 'SPARK-123' },
        },
        source: { kind: 'weekly', weekdays: ['mon', 'fri'], hour: 9, minute: 30 },
        state: {
          last_fired_at: null,
          last_result: null,
          last_error: null,
          next_run_at: '2026-01-02T14:30:00Z',
          recent_history: [],
        },
      }
      const pollTrigger = {
        ...scheduleTrigger,
        id: 'trigger-poll',
        name: 'Poll trigger',
        source_type: 'poll',
        action: {
          flow_name: 'compat/poll-flow.dot',
          project_path: null,
          static_context: {},
        },
        source: {
          url: 'https://example.com/items.json',
          interval_seconds: 300,
          headers: { Authorization: 'Bearer token' },
          items_path: 'items',
          item_id_path: 'id',
        },
      }
      const webhookTrigger = {
        ...scheduleTrigger,
        id: 'trigger-webhook',
        name: 'Webhook trigger',
        source_type: 'webhook',
        action: {
          flow_name: 'compat/webhook-flow.dot',
          project_path: null,
          static_context: { source: 'frontend' },
        },
        source: { webhook_key: 'webhook-key-123' },
        state: { ...scheduleTrigger.state, next_run_at: null },
      }
      const protectedFlowEventTrigger = {
        ...scheduleTrigger,
        id: 'trigger-protected',
        name: 'Protected flow-event trigger',
        enabled: false,
        protected: true,
        source_type: 'flow_event',
        action: {
          flow_name: 'compat/protected-flow.dot',
          project_path: activeProjectPath,
          static_context: { protected: true },
        },
        source: {
          flow_name: 'compat/observed-flow.dot',
          statuses: ['completed', 'failed'],
        },
        state: { ...scheduleTrigger.state, next_run_at: null },
      }
      const createResponse = {
        ...webhookTrigger,
        id: 'trigger-created-webhook',
        name: 'Created webhook trigger',
        webhook_secret: 'created-secret',
      }
      const updateResponse = {
        ...webhookTrigger,
        name: 'Webhook trigger renamed',
        enabled: false,
        webhook_secret: 'rotated-secret',
      }
      const listPayload = [protectedFlowEventTrigger, scheduleTrigger, pollTrigger, webhookTrigger]
      const createPayload = {
        name: 'Created webhook trigger',
        enabled: true,
        source_type: 'webhook' as const,
        action: {
          flow_name: 'compat/webhook-flow.dot',
          project_path: null,
          static_context: { source: 'frontend' },
        },
        source: {},
      }
      const updatePayload = {
        name: 'Webhook trigger renamed',
        enabled: false,
        regenerate_webhook_secret: true,
      }
      const capturedRequests: Array<Record<string, unknown>> = []
      const originalFetch = globalThis.fetch
      globalThis.fetch = vi.fn(async (url: RequestInfo | URL, init?: RequestInit) => {
        const method = init?.method ?? 'GET'
        const bodyText = typeof init?.body === 'string' ? init.body : ''
        capturedRequests.push({
          url: String(url),
          method,
          headers: Object.fromEntries(new Headers(init?.headers).entries()),
          body: bodyText ? JSON.parse(bodyText) : null,
        })
        let responseBody: unknown = { detail: 'Unexpected trigger fixture request' }
        let status = 200
        if (String(url).endsWith('/workspace/api/triggers') && method === 'GET') {
          responseBody = listPayload
        } else if (String(url).endsWith('/workspace/api/triggers') && method === 'POST') {
          responseBody = createResponse
        } else if (String(url).endsWith('/workspace/api/triggers/trigger-webhook') && method === 'GET') {
          responseBody = webhookTrigger
        } else if (String(url).endsWith('/workspace/api/triggers/trigger-webhook') && method === 'PATCH') {
          responseBody = updateResponse
        } else if (String(url).endsWith('/workspace/api/triggers/trigger-custom') && method === 'DELETE') {
          responseBody = { status: 'deleted', id: 'trigger-custom' }
        } else {
          status = 404
        }
        return new Response(JSON.stringify(responseBody), {
          status,
          headers: { 'Content-Type': 'application/json' },
        })
      })
      try {
        const parsedList = parseTriggerListResponse(listPayload)
        const parsedWebhook = parseTriggerResponse(webhookTrigger)
        const parsedCreated = parseTriggerResponse(createResponse)
        const parsedUpdated = parseTriggerResponse(updateResponse)
        const fetchedList = await fetchTriggerListValidated()
        const fetchedWebhook = await fetchTriggerValidated('trigger-webhook')
        const createdWebhook = await createTriggerValidated(createPayload)
        const updatedWebhook = await updateTriggerValidated('trigger-webhook', updatePayload)
        const deleteResult = await deleteTriggerValidated('trigger-custom')
        const protectedForm = triggerToFormState(protectedFlowEventTrigger, activeProjectPath)
        const scheduleForm = triggerToFormState(scheduleTrigger, activeProjectPath)
        const pollForm = triggerToFormState(pollTrigger, activeProjectPath)
        const flowEventForm = triggerToFormState(protectedFlowEventTrigger, activeProjectPath)
        let unsupportedSourceError: Record<string, unknown>
        try {
          parseTriggerResponse({ ...scheduleTrigger, source_type: 'unknown' })
          unsupportedSourceError = { name: 'none' }
        } catch (error) {
          unsupportedSourceError = serializeError(error)
        }
        const hasOwn = (value: Record<string, unknown>, key: string) => Object.prototype.hasOwnProperty.call(value, key)
        expect(parsedList.map((trigger) => trigger.source_type)).toEqual(['flow_event', 'schedule', 'poll', 'webhook'])
        expect(createdWebhook.webhook_secret).toBe('created-secret')
        expect(fetchedWebhook.webhook_secret ?? null).toBeNull()
        expect(updatedWebhook.webhook_secret).toBe('rotated-secret')
        expect(updatedWebhook.source.webhook_key).toBe(createdWebhook.source.webhook_key)
        expect(hasOwn(fetchedWebhook.source, 'secret_hash')).toBe(false)
        return {
          input: {
            trigger_ids: listPayload.map((trigger) => trigger.id),
            create_payload: createPayload,
            update_payload: updatePayload,
          },
          observation: {
            parsedSourceTypes: parsedList.map((trigger) => trigger.source_type),
            summaries: {
              schedule: triggerSourceSummary(parsedList[1]),
              poll: triggerSourceSummary(parsedList[2]),
              webhook: triggerSourceSummary(parsedWebhook),
              flow_event: triggerSourceSummary(parsedList[0]),
              target: triggerTargetSummary(parsedList[0], activeProjectPath),
            },
            formPayloads: {
              protectedForm,
              scheduleAction: buildTriggerActionPayload(scheduleForm),
              scheduleSource: buildTriggerSourcePayload(scheduleForm),
              pollSource: buildTriggerSourcePayload(pollForm),
              flowEventSource: buildTriggerSourcePayload(flowEventForm),
            },
            parserRedaction: {
              describeHasWebhookSecret: parsedWebhook.webhook_secret ?? null,
              describeHasSecretHash: hasOwn(parsedWebhook.source, 'secret_hash'),
              createWebhookSecret: parsedCreated.webhook_secret,
              updateWebhookSecret: parsedUpdated.webhook_secret,
              webhookKeyPreservedOnRegeneration: parsedUpdated.source.webhook_key === parsedCreated.source.webhook_key,
            },
            fetchResults: {
              listCount: fetchedList.length,
              fetchedWebhookId: fetchedWebhook.id,
              createdWebhookId: createdWebhook.id,
              updatedEnabled: updatedWebhook.enabled,
              deleteResult,
            },
            capturedRequests,
            unsupportedSourceError,
          },
        }
      } finally {
        globalThis.fetch = originalFetch
      }
    }

    describe('M0-I05 frontend compatibility probe', () => {
      it('writes compact frontend contract observations', async () => {
        const payload = {
          canonical_flow_model_inputs: buildCanonicalFixture(),
          editor_preview_save_payloads: await buildPreviewSaveFixture(),
          launch_review_run_human_gate_payloads: buildLaunchRunHumanGateFixture(),
          api_wrapper_error_shapes: await buildApiErrorFixture(),
          app_shell_live_event_expectations: buildLiveEventFixture(),
          trigger_crud_payloads: await buildTriggerCrudFixture(),
        }
        writeFileSync(process.env.SPARK_COMPAT_PROBE_OUTPUT!, JSON.stringify(payload, null, 2))
        expect(Object.keys(payload)).toHaveLength(6)
      })
    })
    """
).strip() + "\n"
