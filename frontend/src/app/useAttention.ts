import { useEffect, useRef, useSyncExternalStore } from 'react'

import { useStore } from '@/store'
import { requestNavigation } from '@/state/workspaceSlice'
import { revealSelection } from '@/components/app/view-layout'
import { ATTENTION_CHANGED_EVENT, fetchPendingAttention, type AttentionItem } from '@/lib/api/attentionApi'
import { triggerKey } from '@/lib/api/triggersApi'
import { flowTitle, formatRunDate, formatRunTime, runFlowTitle, runTitle } from '@/features/runs/model/runOverviewModel'
import type { RunRecord } from '@/features/runs/model/shared'
import { focusRunHeading } from '@/features/runs/components/RunHeaderBar'

/** The open chat's heading, which takes focus when a chat is opened from elsewhere. */
export const CHAT_HEADING_ID = 'chat-title'

const ATTENTION_POLL_MS = 15_000

export const ATTENTION_KIND_LABELS: Record<AttentionItem['kind'], string> = {
    run_gate: 'Run waiting for input',
    chat_question: 'Chat waiting for your answer',
    flow_run_request: 'Flow run request',
    proposed_plan: 'Plan pending review',
    mission: 'Mission needs you',
}

type AttentionRun = Pick<RunRecord, 'run_id' | 'title' | 'flow_name' | 'flow_title' | 'first_launch_input'>

/**
 * What an item is called: a run waiting at a gate takes the title the Runs list gives it,
 * and a run not loaded yet goes by its name or its flow's title, never the flow's file name.
 */
export function attentionTitle(item: AttentionItem, runs: AttentionRun[]): string {
    const run = item.kind === 'run_gate' ? runs.find((candidate) => candidate.run_id === item.run_id) : undefined
    if (run && (item.title === run.flow_name || item.title === run.title)) {
        return runTitle(run)
    }
    if (item.kind === 'run_gate' && item.flow_name && item.title === item.flow_name) {
        return flowTitle(item.flow_name)
    }
    return item.title || item.run_id || item.id
}

/** A run gate's flow, when the item's title doesn't already say it. */
export function attentionFlow(item: AttentionItem, runs: AttentionRun[]): string | null {
    if (item.kind !== 'run_gate' || !item.flow_name) return null
    // A loaded run knows its flow's declared title.
    const run = runs.find((candidate) => candidate.run_id === item.run_id)
    const flow = run ? runFlowTitle(run) : flowTitle(item.flow_name)
    return flow.toLowerCase() === attentionTitle(item, runs).toLowerCase() ? null : flow
}

/**
 * When each item came in: "12:52 AM" today, a date before. Items with one title that share a
 * minute today, such as a trigger's repeated runs, show the second too, so each tells apart.
 */
export function attentionTimes(items: AttentionItem[], runs: AttentionRun[], now: number): (item: AttentionItem) => string {
    const today = new Date(now).toDateString()
    const key = (item: AttentionItem) => `${attentionTitle(item, runs)}\n${formatRunDate(item.updated_at, now)}`
    const counts = new Map<string, number>()
    items.forEach((item) => counts.set(key(item), (counts.get(key(item)) ?? 0) + 1))
    return (item) => (counts.get(key(item)) ?? 0) > 1 && new Date(item.updated_at).toDateString() === today
        ? formatRunTime(item.updated_at, true)
        : formatRunDate(item.updated_at, now)
}

// One shared list for every view that shows it, so the Inbox and the Overview never disagree.
let attentionItems: AttentionItem[] | null = null
// Whether the latest check failed: an empty list is then unknown, not an all-clear.
let attentionFailed = false
const attentionListeners = new Set<() => void>()
let attentionRequest = 0

/** Fetches pending attention now; anything that answers or resolves an item calls this so the Inbox clears at once. */
export function refreshAttention() {
    const request = ++attentionRequest
    fetchPendingAttention()
        .then((next) => {
            if (request !== attentionRequest) return
            attentionItems = next
            attentionFailed = false
            attentionListeners.forEach((listener) => listener())
        })
        .catch(() => {
            // A failed check keeps the last known items and says it failed.
            if (request !== attentionRequest || attentionFailed) return
            attentionFailed = true
            attentionListeners.forEach((listener) => listener())
        })
}

function subscribeAttention(listener: () => void) {
    attentionListeners.add(listener)
    if (attentionListeners.size === 1) {
        refreshAttention()
        const interval = window.setInterval(refreshAttention, ATTENTION_POLL_MS)
        window.addEventListener('focus', refreshAttention)
        window.addEventListener(ATTENTION_CHANGED_EVENT, refreshAttention)
        stopAttentionPolling = () => {
            window.clearInterval(interval)
            window.removeEventListener('focus', refreshAttention)
            window.removeEventListener(ATTENTION_CHANGED_EVENT, refreshAttention)
        }
    }
    return () => {
        attentionListeners.delete(listener)
        if (attentionListeners.size === 0) stopAttentionPolling()
    }
}
let stopAttentionPolling = () => {}

/** Pending attention, polled and refreshed when a run's status changes; null until the first fetch resolves. */
export function useAttentionItems() {
    const items = useSyncExternalStore(subscribeAttention, () => attentionItems)
    // A run, mission or chat leaving or reaching a question changes what waits on you; don't wait for the next poll.
    const runStatuses = useStore((state) => [
        ...state.runsListSession.runs.map((run) => `${run.run_id}:${run.status}`),
        ...state.missionBoard.map((mission) => `${mission.id}:${mission.status ?? ''}`),
        ...Object.values(state.homeConversationCache.summariesByProjectPath).flat()
            .filter((chat) => chat.needs_attention).map((chat) => chat.conversation_id),
    ].join(','))
    const firstStatuses = useRef(runStatuses)
    useEffect(() => {
        if (runStatuses !== firstStatuses.current) refreshAttention()
    }, [runStatuses])
    return items
}

/** Whether the latest check for pending attention failed; views say so instead of "nothing is waiting". */
export function useAttentionFailed() {
    return useSyncExternalStore(subscribeAttention, () => attentionFailed)
}

/**
 * Opens a chat in the Chats view and focuses its heading, as the control that opened it goes away
 * with its view; leaving a project page asks first, so a cancelled leave keeps the shown chat.
 * A `draft` replaces its composer text, unsent.
 */
export function openChat(projectPath: string, conversationId: string, draft?: string) {
    const open = () => {
        revealSelection()
        useStore.setState({ projectPagePath: null })
        useStore.getState().updateProjectSessionState(projectPath, { conversationId })
        if (draft !== undefined) useStore.getState().updateHomeProjectSession(projectPath, { chatDraft: draft })
        useStore.getState().setActiveProjectPath(projectPath)
        useStore.getState().setViewMode('home')
        requestAnimationFrame(() => document.getElementById(CHAT_HEADING_ID)?.focus({ preventScroll: true }))
    }
    if (useStore.getState().projectPagePath) requestNavigation(open)
    else open()
}

/** Opens a run in Runs; its heading takes focus once the view shows, as the control that opened it goes away. */
export function openRun(runId: string) {
    revealSelection()
    useStore.getState().setRunsSelectedRunId(runId)
    useStore.getState().setViewMode('runs')
    requestAnimationFrame(() => focusRunHeading(runId))
}

/** Opens a trigger in Triggers; a trigger's id only names it within its project. */
export function openTrigger(projectPath: string, triggerId: string) {
    revealSelection()
    useStore.getState().updateTriggersSession({ selectedTriggerId: triggerKey({ project_path: projectPath, id: triggerId }), createFormOpen: false })
    useStore.getState().setViewMode('triggers')
}

export function openMission(id: string, projectPath: string) {
    revealSelection()
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
