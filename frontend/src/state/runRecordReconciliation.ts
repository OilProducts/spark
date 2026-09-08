import type { PipelineStatusResponse } from '@/lib/attractorClient'
import type { RunRecord } from '@/features/runs/model/shared'

export function toRunRecord(status: PipelineStatusResponse): RunRecord {
    return {
        run_id: status.run_id,
        flow_name: status.flow_name || '',
        status: status.status,
        outcome: status.outcome ?? null,
        outcome_reason_code: status.outcome_reason_code ?? null,
        outcome_reason_message: status.outcome_reason_message ?? null,
        working_directory: status.working_directory || '',
        project_path: status.project_path,
        git_branch: status.git_branch ?? null,
        git_commit: status.git_commit ?? null,
        spec_id: status.spec_id ?? null,
        plan_id: status.plan_id ?? null,
        model: status.model || '',
        started_at: status.started_at || '',
        ended_at: status.ended_at ?? null,
        last_error: status.last_error || '',
        token_usage: typeof status.token_usage === 'number' || status.token_usage === null
            ? status.token_usage
            : undefined,
        token_usage_breakdown: status.token_usage_breakdown ?? undefined,
        estimated_model_cost: status.estimated_model_cost ?? undefined,
        current_node: status.current_node ?? null,
        continued_from_run_id: status.continued_from_run_id ?? null,
        continued_from_node: status.continued_from_node ?? null,
        continued_from_flow_mode: status.continued_from_flow_mode ?? null,
        continued_from_flow_name: status.continued_from_flow_name ?? null,
        parent_run_id: status.parent_run_id ?? null,
        parent_node_id: status.parent_node_id ?? null,
        root_run_id: status.root_run_id ?? null,
        child_invocation_index: status.child_invocation_index ?? null,
        execution_mode: status.execution_mode,
        execution_profile_id: status.execution_profile_id,
        execution_container_image: status.execution_container_image,
        execution_profile_capabilities: status.execution_profile_capabilities,
        execution_lock: status.execution_lock ?? undefined,
        cleanup_error: status.cleanup_error,
    }
}
