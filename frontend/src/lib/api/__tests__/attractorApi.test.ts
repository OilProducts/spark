import { afterEach, vi } from 'vitest'

import {
    fetchRunActivityValidated,
    parsePipelineStartResponse,
    parsePipelineStatusResponse,
    parseRunRecordPayload,
    parseRunsListResponse,
} from '@/lib/api/attractorApi'

describe('attractorApi parsing', () => {
    it('preserves provider and reasoning metadata on start, status, and run list payloads', () => {
        const run = {
            run_id: 'run-provider',
            flow_name: 'provider.dot',
            status: 'running',
            outcome: null,
            outcome_reason_code: null,
            outcome_reason_message: null,
            working_directory: '/tmp/provider',
            project_path: '/tmp/provider',
            git_branch: null,
            git_commit: null,
            spec_id: null,
            plan_id: null,
            model: 'gpt-5.4',
            provider: 'openai',
            llm_provider: 'openai',
            reasoning_effort: 'high',
            started_at: '2026-04-24T12:00:00Z',
            ended_at: null,
            last_error: '',
            token_usage: null,
            token_usage_breakdown: null,
            estimated_model_cost: null,
            continued_from_run_id: null,
            continued_from_node: null,
            continued_from_flow_mode: null,
            continued_from_flow_name: null,
        }

        expect(parsePipelineStartResponse({
            status: 'started',
            pipeline_id: 'run-provider',
            run_id: 'run-provider',
            working_directory: '/tmp/provider',
            model: 'gpt-5.4',
            provider: 'openai',
            llm_provider: 'openai',
            reasoning_effort: 'high',
        })).toMatchObject({
            provider: 'openai',
            llm_provider: 'openai',
            reasoning_effort: 'high',
        })

        expect(parsePipelineStatusResponse({
            ...run,
            pipeline_id: 'run-provider',
            completed_nodes: [],
            progress: { current_node: 'start', completed_nodes: [], completed_count: 0 },
        })).toMatchObject({
            provider: 'openai',
            llm_provider: 'openai',
            reasoning_effort: 'high',
            current_node: 'start',
            progress: {
                current_node: 'start',
                completed_nodes: [],
                completed_count: 0,
            },
        })

        expect(parseRunsListResponse({ runs: [run] }).runs[0]).toMatchObject({
            provider: 'openai',
            llm_provider: 'openai',
            reasoning_effort: 'high',
        })
    })

    it('normalizes a single run payload the same way as a runs list entry', () => {
        const run = {
            run_id: 'run-single-parser',
            flow_name: 'single.dot',
            status: 'completed',
            outcome: 'success',
            outcome_reason_code: null,
            outcome_reason_message: null,
            working_directory: '/tmp/single/workdir',
            project_path: '/tmp/single',
            git_branch: null,
            git_commit: null,
            spec_id: null,
            plan_id: null,
            model: 'gpt-5.4',
            provider: 'openai',
            llm_provider: 'openai',
            reasoning_effort: 'medium',
            started_at: '2026-04-24T12:00:00Z',
            ended_at: '2026-04-24T12:05:00Z',
            last_error: '',
            token_usage: 36,
            token_usage_breakdown: {
                input_tokens: 23,
                cached_input_tokens: 3,
                output_tokens: 13,
                total_tokens: 36,
                by_model: {
                    'gpt-5.4': {
                        input_tokens: 23,
                        cached_input_tokens: 3,
                        output_tokens: 13,
                        total_tokens: 36,
                    },
                },
            },
            estimated_model_cost: {
                currency: 'USD',
                amount: 0.000166,
                status: 'estimated',
                unpriced_models: [],
            },
            continued_from_run_id: null,
            continued_from_node: null,
            continued_from_flow_mode: null,
            continued_from_flow_name: null,
        }

        expect(parseRunRecordPayload(run)).toEqual(parseRunsListResponse({ runs: [run] }).runs[0])
        expect(parseRunRecordPayload({ run_id: 'missing-status' })).toBeNull()
    })
})

describe('fetchRunActivityValidated', () => {
    afterEach(() => vi.unstubAllGlobals())

    const detail = (executions: Array<{ node_id: string; stage_index: number; status: Record<string, unknown> | null }>) => ({
        pipeline_id: 'run-a', run_id: 'run-a', flow_name: 'f.yaml', status: 'running', outcome: null,
        outcome_reason_code: null, outcome_reason_message: null, working_directory: '/tmp/a', project_path: '/tmp/a',
        git_branch: null, git_commit: null, spec_id: null, plan_id: null, model: 'm', started_at: '2026-09-29T12:00:00Z',
        ended_at: null, last_error: '', token_usage: null, completed_nodes: [],
        executions: executions.map((execution) => ({ run_id: 'run-a', attempt: 0, ...execution })),
        child_runs: [],
    })
    const transcript = (text: string) => ({ records: [
        { type: 'turn_upsert', turn: { id: 'prompt', role: 'user', kind: 'message', content: `Prompt for ${text}` } },
        { type: 'segment_upsert', source_event_sequence: 1, segment: { id: 'event-1', turn_id: 'response', order: 0, kind: 'assistant_message', role: 'assistant', status: 'complete', timestamp: '2026-09-29T12:00:01Z', updated_at: '2026-09-29T12:00:01Z', content: text } },
    ] })

    it('refetches only executions that are new or whose status changed', async () => {
        const requested: string[] = []
        let executions = [{ node_id: 'plan', stage_index: 0, status: null as Record<string, unknown> | null }]
        vi.stubGlobal('fetch', vi.fn(async (input: RequestInfo | URL) => {
            const url = String(input)
            requested.push(url.replace(/^.*\/pipelines\//, ''))
            const body = url.includes('/transcript') ? transcript(url.includes('/build/') ? 'build' : 'plan') : detail(executions)
            return new Response(JSON.stringify(body), { status: 200, headers: { 'content-type': 'application/json' } })
        }))

        const first = await fetchRunActivityValidated('run-a')
        expect(first.prompts?.map((prompt) => prompt.content)).toEqual(['Prompt for plan'])

        // plan finished (its status changed) and build started: both are fetched.
        executions = [
            { node_id: 'plan', stage_index: 0, status: { outcome: 'success' } },
            { node_id: 'build', stage_index: 1, status: null },
        ]
        requested.length = 0
        const second = await fetchRunActivityValidated('run-a', first)
        expect(requested).toEqual(['run-a', 'run-a/executions/plan/0-0/transcript', 'run-a/executions/build/1-0/transcript'])

        // Nothing changed: only the detail is fetched, and loaded segments and prompts are kept.
        requested.length = 0
        const third = await fetchRunActivityValidated('run-a', second)
        expect(requested).toEqual(['run-a'])
        expect(third.segments.map((segment) => segment.content)).toEqual(['plan', 'build'])
        expect(third.prompts?.map((prompt) => prompt.content)).toEqual(['Prompt for plan', 'Prompt for build'])
    })
})
