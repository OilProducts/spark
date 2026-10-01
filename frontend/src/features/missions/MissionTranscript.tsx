import { Fragment, useMemo, useState, type ReactNode } from 'react'
import { useStore } from '@/store'
import { buildRunsHash } from '@/app/runsRouting'
import { InlineError } from '@/components/app/inline-error'
import { MessageRow } from '@/components/app/transcript/SegmentRows'
import { ProjectConversationHistory } from '@/features/projects/components/ProjectConversationHistory'
import {
    getConversationFlowLaunches,
    getConversationTimelineEntries,
    hydrateConversationRecordFromSnapshot,
} from '@/features/projects/model/projectsHomeState'
import { flowTitle, formatRunDate } from '@/features/runs/model/runOverviewModel'
import { submitConversationRequestUserInputValidated, type ConversationSnapshotResponse } from '@/lib/api/conversationsApi'
import { parseTurn, runMarks } from './model/missionModel'

export function openRun(runId: string) {
    const state = useStore.getState()
    state.setRunsSelectedRunId(runId)
    state.setViewMode('runs')
}

export const formatTime = (value: string) => formatRunDate(value, Date.now())

/** A run's title linking to it on the Runs page. */
export function RunLink({ runId, children }: { runId: string; children: ReactNode }) {
    return <a href={buildRunsHash(runId)} className="text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring" onClick={e => {
        if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
        e.preventDefault(); openRun(runId)
    }}>{children}</a>
}

/** One quiet line in the thread: a mark, what happened, and when. */
function EventLine({ mark, tone = '', at, children }: { mark: string; tone?: string; at: string; children: ReactNode }) {
    return <li data-testid="mission-event" className="flex items-baseline gap-2 py-0.5 text-xs text-muted-foreground">
        <span aria-hidden="true" className={`w-4 shrink-0 text-center ${tone}`}>{mark}</span>
        <span className="min-w-0 flex-1 truncate">{children}</span>
        <time dateTime={at} className="shrink-0 tabular-nums">{formatTime(at)}</time>
    </li>
}

const noop = () => {}
type Conversation = { conversationId: string; snapshot: ConversationSnapshotResponse | null; loading: boolean; error: string; reload: () => void }

/** The mission's conversation in the Air transcript rows, with its events as quiet lines. */
export function MissionTranscript({ conversation, project, runTitles }: { conversation: Conversation; project: string; runTitles: Map<string, string> }) {
    const { conversationId, snapshot, loading, error, reload } = conversation
    const [submitting, setSubmitting] = useState<Record<string, boolean>>({})
    const [inputError, setInputError] = useState<string | null>(null)
    // The agent can ask through its own question tool mid-turn; the answer resumes that turn.
    async function submitInput(requestId: string, answers: Record<string, string>) {
        setInputError(null)
        setSubmitting(current => ({ ...current, [requestId]: true }))
        try {
            await submitConversationRequestUserInputValidated(conversationId, requestId, { project_path: project, answers })
            reload()
        } catch (e) { setInputError(e instanceof Error ? e.message : String(e)) } finally {
            setSubmitting(current => { const next = { ...current }; delete next[requestId]; return next })
        }
    }
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const [expandedThinking, setExpandedThinking] = useState<Record<string, boolean>>({})
    const record = useMemo(() => snapshot ? hydrateConversationRecordFromSnapshot(snapshot) : null, [snapshot])
    const entries = getConversationTimelineEntries(record)
    const launches = getConversationFlowLaunches(record)
    const launchesById = new Map(launches.map(launch => [launch.id, launch]))
    const firstTurnId = entries.find(entry => entry.kind === 'message' && entry.role === 'user')?.id
    const title = (runId: string | null, fallback: string) => (runId && runTitles.get(runId)) || fallback || runId || 'Run'
    return <>{error && <InlineError>{error}</InlineError>}<ProjectConversationHistory
        activeConversationId={conversationId}
        activeProjectPath={project}
        isConversationHistoryLoading={loading}
        hasRenderableConversationHistory={entries.length > 0}
        activeConversationHistory={entries}
        activeFlowRunRequestsById={new Map()}
        activeFlowLaunchesById={launchesById}
        activeProposedPlansById={new Map()}
        latestFlowRunRequestId={null}
        latestFlowLaunchId={launches.at(-1)?.id ?? null}
        expandedToolCalls={expandedToolCalls}
        expandedThinkingEntries={expandedThinking}
        pendingFlowRunRequestId={null}
        pendingProposedPlanId={null}
        requestUserInputActionError={inputError}
        submittingRequestUserInputIds={submitting}
        formatConversationTimestamp={formatTime}
        onSubmitRequestUserInput={submitInput}
        onToggleToolCallExpanded={id => setExpandedToolCalls(current => ({ ...current, [id]: !current[id] }))}
        onToggleThinkingEntryExpanded={id => setExpandedThinking(current => ({ ...current, [id]: !current[id] }))}
        onReviewFlowRunRequest={noop}
        onReviewProposedPlan={noop}
        onOpenFlowRun={({ run_id }) => { if (run_id) openRun(run_id) }}
        renderEntry={(entry, key) => {
            if (entry.kind === 'flow_launch') {
                const launch = launchesById.get(entry.artifactId)
                // A launch that never became a run keeps its full row with the error.
                if (!launch?.run_id) return undefined
                return <EventLine key={key} mark="→" tone="text-primary" at={entry.timestamp}>Launched <RunLink runId={launch.run_id}>{title(launch.run_id, launch.summary)}</RunLink></EventLine>
            }
            // The outcome closes the thread instead.
            if (entry.kind === 'final_separator' && entry.label.startsWith('Closed as ')) return null
            if (entry.kind !== 'message' || entry.role !== 'user') return undefined
            return <Fragment key={key}>{parseTurn(entry.content, entry.id === firstTurnId).map((event, index) => {
                const at = entry.timestamp
                switch (event.kind) {
                    case 'start': return <EventLine key={index} mark="▸" at={at}>Mission started</EventLine>
                    case 'result': {
                        const mark = runMarks[event.status === 'completed' ? 'completed' : 'failed']
                        return <EventLine key={index} mark={mark.mark} tone={mark.tone} at={at}>
                            <RunLink runId={event.runId}>{title(event.runId, event.summary || flowTitle(event.flowName))}</RunLink> {event.status === 'waiting' ? 'is waiting on a recovery decision' : `ended ${event.status}`}{event.error && `: ${event.error}`}
                        </EventLine>
                    }
                    case 'question': return <EventLine key={index} mark="?" tone={runMarks.waiting.tone} at={at}>
                        {event.runId ? <RunLink runId={event.runId}>{title(event.runId, flowTitle(event.flowName))}</RunLink> : flowTitle(event.flowName)} asked “{event.prompt}”
                    </EventLine>
                    default: return <MessageRow key={index} entry={{ ...entry, id: `${entry.id}:${index}`, content: event.text }} enableCopy formatConversationTimestamp={formatTime} />
                }
            })}</Fragment>
        }}
    /></>
}
