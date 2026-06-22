import {
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
