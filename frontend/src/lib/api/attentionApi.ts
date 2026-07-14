import { asOptionalString, expectObjectRecord, expectString } from './shared'
import { fetchWorkspaceJsonValidated } from './apiClient'

export type AttentionKind = 'run_gate' | 'flow_run_request' | 'proposed_plan'

export interface AttentionItem {
    kind: AttentionKind
    id: string
    title: string
    project_path: string
    run_id?: string
    conversation_id?: string
    conversation_handle?: string
    updated_at: string
}

const ATTENTION_KINDS: ReadonlySet<string> = new Set(['run_gate', 'flow_run_request', 'proposed_plan'])

function parseAttentionItems(payload: unknown, endpoint: string): AttentionItem[] {
    const record = expectObjectRecord(payload, endpoint)
    const items = Array.isArray(record.items) ? record.items : []
    return items.flatMap((entry) => {
        const item = expectObjectRecord(entry, endpoint)
        const kind = expectString(item.kind, 'kind', endpoint)
        if (!ATTENTION_KINDS.has(kind)) {
            return []
        }
        return [{
            kind: kind as AttentionKind,
            id: expectString(item.id, 'id', endpoint),
            title: asOptionalString(item.title) ?? '',
            project_path: asOptionalString(item.project_path) ?? '',
            run_id: asOptionalString(item.run_id),
            conversation_id: asOptionalString(item.conversation_id),
            conversation_handle: asOptionalString(item.conversation_handle),
            updated_at: asOptionalString(item.updated_at) ?? '',
        }]
    })
}

export async function fetchPendingAttention(): Promise<AttentionItem[]> {
    return fetchWorkspaceJsonValidated(
        '/attention',
        undefined,
        '/workspace/api/attention',
        parseAttentionItems,
    )
}
