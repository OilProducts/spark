import { useEffect, useState } from 'react'

import { useStore } from '@/store'
import { requestNavigation } from '@/state/workspaceSlice'
import { fetchPendingAttention, type AttentionItem } from '@/lib/api/attentionApi'
import { forgetResolvedAttention } from '@/features/overview/model/overviewModel'

const ATTENTION_POLL_MS = 30_000

export const ATTENTION_KIND_LABELS: Record<AttentionItem['kind'], string> = {
    run_gate: 'Run waiting for input',
    flow_run_request: 'Flow run request',
    proposed_plan: 'Plan pending review',
    mission: 'Mission needs you',
}

/** Pending attention, polled; null until the first fetch resolves. */
export function useAttentionItems() {
    const [items, setItems] = useState<AttentionItem[] | null>(null)
    useEffect(() => {
        let disposed = false
        const refresh = () => {
            fetchPendingAttention()
                .then((next) => {
                    // Every poll, the Overview open or not, so resolved attention that returns shows the dot.
                    forgetResolvedAttention(next)
                    if (!disposed) {
                        setItems(next)
                    }
                })
                .catch(() => {
                    // Transient poll failures keep the last known items.
                })
        }
        refresh()
        const interval = window.setInterval(refresh, ATTENTION_POLL_MS)
        const onFocus = () => refresh()
        window.addEventListener('focus', onFocus)
        return () => {
            disposed = true
            window.clearInterval(interval)
            window.removeEventListener('focus', onFocus)
        }
    }, [])
    return items
}

/** Opens a chat in the Chats view; leaving a project page asks first, so a cancelled leave keeps the shown chat. */
export function openChat(projectPath: string, conversationId: string) {
    const open = () => {
        useStore.setState({ projectPagePath: null })
        useStore.getState().updateProjectSessionState(projectPath, { conversationId })
        useStore.getState().setActiveProjectPath(projectPath)
        useStore.getState().setViewMode('home')
    }
    if (useStore.getState().projectPagePath) requestNavigation(open)
    else open()
}

export function openRun(runId: string) {
    useStore.getState().setRunsSelectedRunId(runId)
    useStore.getState().setViewMode('runs')
}

export function openMission(id: string, projectPath: string) {
    useStore.getState().setSelectedMission({ id, projectPath })
    useStore.getState().setViewMode('missions')
}

/** Opens an attention item where it can be answered. */
export function openAttentionItem(item: AttentionItem) {
    if (item.kind === 'run_gate' && item.run_id) {
        openRun(item.run_id)
        return
    }
    if (item.kind === 'mission') {
        if (item.project_path) openMission(item.id, item.project_path)
        else useStore.getState().setViewMode('missions')
        return
    }
    if (!item.conversation_id || !item.project_path) {
        useStore.getState().setViewMode('home')
        return
    }
    openChat(item.project_path, item.conversation_id)
}
