import { useEffect, useState } from 'react'

import { useStore } from '@/store'
import { requestNavigation } from '@/state/workspaceSlice'
import { fetchPendingAttention, type AttentionItem } from '@/lib/api/attentionApi'
import { runTitle } from '@/features/runs/model/runOverviewModel'
import type { RunRecord } from '@/features/runs/model/shared'

const ATTENTION_POLL_MS = 30_000

export const ATTENTION_KIND_LABELS: Record<AttentionItem['kind'], string> = {
    run_gate: 'Run waiting for input',
    flow_run_request: 'Flow run request',
    proposed_plan: 'Plan pending review',
    mission: 'Mission needs you',
}

/**
 * What an item is called: a run waiting at a gate is titled by its flow's name, so it
 * takes the title the Runs list gives that run.
 */
export function attentionTitle(item: AttentionItem, runs: Pick<RunRecord, 'run_id' | 'title' | 'flow_name' | 'launch_context' | 'first_launch_input'>[]): string {
    const run = item.kind === 'run_gate' ? runs.find((candidate) => candidate.run_id === item.run_id) : undefined
    if (run && item.title === run.flow_name) {
        return runTitle(run)
    }
    return item.title || item.run_id || item.id
}

/** Pending attention, polled; null until the first fetch resolves. */
export function useAttentionItems() {
    const [items, setItems] = useState<AttentionItem[] | null>(null)
    useEffect(() => {
        let disposed = false
        const refresh = () => {
            fetchPendingAttention()
                .then((next) => {
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
/** Opens a chat; a `draft` replaces its composer text, unsent. */
export function openChat(projectPath: string, conversationId: string, draft?: string) {
    const open = () => {
        useStore.setState({ projectPagePath: null })
        useStore.getState().updateProjectSessionState(projectPath, { conversationId })
        if (draft !== undefined) useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: draft })
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
