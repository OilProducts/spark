import {
    parsePipelineStartResponse,
    parsePipelineStatusResponse,
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
})
