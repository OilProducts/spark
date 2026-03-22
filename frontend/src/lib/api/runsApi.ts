import type { FlowLaunchResponse } from './conversationsApi'
import { fetchWorkspaceJsonValidated } from './apiClient'
import { parseConversationSnapshotResponse } from './conversationsApi'

export interface WorkspaceRunLaunchResponse {
    ok: true
    launch?: FlowLaunchResponse | null
    conversation?: ReturnType<typeof parseConversationSnapshotResponse> | null
    run_id?: string | null
}

export async function launchWorkspaceRunValidated(payload: {
    flow_name: string
    summary: string
    conversation_handle?: string | null
    project_path?: string | null
    goal?: string | null
    launch_context?: Record<string, unknown> | null
    model?: string | null
}): Promise<WorkspaceRunLaunchResponse> {
    return fetchWorkspaceJsonValidated(
        '/runs/launch',
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(payload),
        },
        '/workspace/api/runs/launch',
        (value, endpoint) => {
            const record = value && typeof value === 'object' && !Array.isArray(value)
                ? value as Record<string, unknown>
                : null
            if (!record || record.ok !== true) {
                throw new Error(`${endpoint}: Expected run launch response.`)
            }
            const launch = record.launch && typeof record.launch === 'object'
                ? record.launch as FlowLaunchResponse
                : null
            const conversation = record.conversation
                ? parseConversationSnapshotResponse(record.conversation, endpoint)
                : null
            return {
                ok: true,
                launch,
                conversation,
                run_id: typeof record.run_id === 'string' ? record.run_id : null,
            }
        },
    )
}
