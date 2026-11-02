import { useMemo, useState, type ReactNode } from 'react'
import { Check, CircleHelp, Flag, Pencil, Play, X, Zap } from 'lucide-react'
import { useStore } from '@/store'
import { buildRunsHash } from '@/app/runsRouting'
import { InlineError } from '@/components/app/inline-error'
import { SectionHeading } from '@/components/app/section-heading'
import { statusTextClassName } from '@/components/app/status'
import { EventLine, MessageRow, RunCard, ShowThinkingToggle } from '@/components/app/transcript/SegmentRows'
import { Transcript } from '@/components/app/transcript/Transcript'
import { ProjectConversationRequestUserInputCard } from '@/features/projects/components/ProjectConversationRequestUserInputCard'
import {
    getConversationFlowLaunches,
    getConversationTimelineEntries,
    hydrateConversationRecordFromSnapshot,
} from '@/features/projects/model/projectsHomeState'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'
import { flowTitle, formatRunDate, runStatusKind } from '@/features/runs/model/runOverviewModel'
import { STATUS_LABELS } from '@/features/runs/model/shared'
import { retryConversationTurnValidated, submitConversationRequestUserInputValidated, type ConversationSnapshotResponse } from '@/lib/api/conversationsApi'
import { cn } from '@/lib/utils'
import { parseTurn, runKinds, runStatusLabel, type MissionRun } from './model/missionModel'

export function openRun(runId: string) {
    const state = useStore.getState()
    state.setRunsSelectedRunId(runId)
    state.setViewMode('runs')
}

export const formatTime = (value: string) => formatRunDate(value, Date.now())

/** A run's title linking to it on the Runs page. */
export function RunLink({ runId, className, children }: { runId: string; className?: string; children: ReactNode }) {
    return <a href={buildRunsHash(runId)} className={cn('text-primary hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring', className)} onClick={e => {
        if (e.button !== 0 || e.metaKey || e.ctrlKey || e.shiftKey || e.altKey) return
        e.preventDefault(); openRun(runId)
    }}>{children}</a>
}

type Entry = ConversationTimelineEntry

type Conversation = { conversationId: string; snapshot: ConversationSnapshotResponse | null; loading: boolean; error: string; reload: () => void }

/** The mission's log in the shared transcript grammar: turn headers, folded commands, run cards and quiet event lines. */
export function MissionTranscript({ conversation, project, runs }: { conversation: Conversation; project: string; runs: Map<string, MissionRun> }) {
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
    // A failed turn resends its delivered prompt as a new turn of the mission's conversation.
    const [retryError, setRetryError] = useState<string | null>(null)
    function retryTurn(turnId: string) {
        setRetryError(null)
        void retryConversationTurnValidated(conversationId, turnId, project).then(reload)
            .catch((e: unknown) => setRetryError(e instanceof Error ? e.message : String(e)))
    }
    const [showThinking, setShowThinking] = useState(false)
    const record = useMemo(() => snapshot ? hydrateConversationRecordFromSnapshot(snapshot) : null, [snapshot])
    const entries = getConversationTimelineEntries(record)
    const launchesById = new Map(getConversationFlowLaunches(record).map(launch => [launch.id, launch]))
    const firstTurnId = entries.find(entry => entry.kind === 'message' && entry.role === 'user')?.id
    const title = (runId: string | null, fallback: string) => (runId && runs.get(runId)?.title) || fallback || runId || 'Run'
    const when = (at: string) => <time dateTime={at}>{formatTime(at)}</time>

    /** The mission's own rows: user turns carry its events, launches are run cards, and the agent's questions are answered here. */
    function renderEntry(entry: Entry): ReactNode | undefined {
        switch (entry.kind) {
            case 'message': return entry.role === 'user' ? userTurn(entry) : undefined
            case 'final_separator':
                // The outcome closes the thread instead; a notice the runtime appended on its own, such as a turn that could not start, stays visible.
                if (/^Worked(?: for .+)?$/.test(entry.label)) return undefined
                if (entry.label.startsWith('Closed as ')) return null
                return <li><EventLine data-testid="mission-event" icon={<X className={statusTextClassName('destructive')} />} at={when(entry.timestamp)} title={entry.label}>{entry.label.split('\n')[0]}</EventLine></li>
            case 'flow_launch': {
                const launch = launchesById.get(entry.artifactId)
                if (!launch) return <li><EventLine at={when(entry.timestamp)}>Flow launch unavailable</EventLine></li>
                // A launch that never became a run keeps its error as an event line.
                if (!launch.run_id) {
                    return <li><EventLine icon={launch.status === 'launch_failed' ? <X className="text-destructive" /> : <Play className="text-info" />} at={when(entry.timestamp)} title={launch.launch_error ?? undefined}>
                        {launch.status === 'launch_failed' ? <><b>{launch.summary}</b> failed to launch{launch.launch_error && `: ${launch.launch_error}`}</> : <>Launching <b>{launch.summary}</b>…</>}
                    </EventLine></li>
                }
                const run = runs.get(launch.run_id)
                return <li><RunCard data-testid="mission-run-card" kicker={`Launched run · ${flowTitle(launch.flow_name)}`}
                    title={<RunLink runId={launch.run_id}>{title(launch.run_id, launch.summary)}</RunLink>}
                    tone={run ? runKinds[run.kind].tone : 'neutral'} status={run ? runStatusLabel(run) : 'Launched'} duration={run?.duration || undefined} /></li>
            }
            case 'request_user_input': return <li className="flex min-w-0 justify-start">
                <ProjectConversationRequestUserInputCard actionError={inputError} entry={entry} formatConversationTimestamp={formatTime} isSubmitting={submitting[entry.requestUserInput.requestId] === true} onSubmitRequestUserInput={submitInput} />
            </li>
            default: return undefined
        }
    }

    function userTurn(entry: Extract<Entry, { kind: 'message' }>) {
        return parseTurn(entry.content, entry.id === firstTurnId).map((event, index) => {
            const key = `${entry.id}:${index}`
            const at = when(entry.timestamp)
            switch (event.kind) {
                case 'start': return <li key={key}><EventLine data-testid="mission-event" icon={<Flag />} at={at}><b>Mission started</b></EventLine></li>
                case 'result': {
                    const kind = runStatusKind(event.status, false)
                    const Icon = kind === 'completed' ? Check : kind === 'waiting' ? CircleHelp : X
                    return <li key={key}><EventLine data-testid="mission-event" icon={<Icon className={statusTextClassName(runKinds[kind].tone)} />} at={at}>
                        <RunLink runId={event.runId}>{title(event.runId, event.summary || flowTitle(event.flowName))}</RunLink> {event.status === 'waiting' ? 'is waiting on a recovery decision' : `ended · ${STATUS_LABELS[event.status] || event.status}`}{event.error && `: ${event.error}`}
                    </EventLine></li>
                }
                case 'question': return <li key={key}><EventLine data-testid="mission-event" icon={<CircleHelp className={statusTextClassName('warning')} />} at={at}>
                    {event.runId ? <RunLink runId={event.runId}>{title(event.runId, flowTitle(event.flowName))}</RunLink> : flowTitle(event.flowName)} asked “{event.prompt}”
                </EventLine></li>
                // What the runtime told the agent (a trigger fired, the mission was edited) is an event, not prose; the full text is the tooltip.
                case 'event': return <li key={key}><EventLine data-testid="mission-event" icon={event.text.startsWith('Trigger fired') ? <Zap /> : <Pencil />} at={at} title={event.text}>{event.text.split('\n')[0]}</EventLine></li>
                default: return <MessageRow key={key} entry={{ ...entry, id: key, content: event.text }} enableCopy formatConversationTimestamp={formatTime} />
            }
        })
    }

    return <>
        {error && <InlineError>{error}</InlineError>}
        {retryError && <InlineError>{retryError}</InlineError>}
        <SectionHeading as="h3" action={<ShowThinkingToggle pressed={showThinking} onPressedChange={setShowThinking} />}>Log</SectionHeading>
        <Transcript testIdPrefix="mission" entries={entries} showThinking={showThinking} formatTimestamp={formatTime} loading={loading}
            conversation={{ id: conversationId, projectPath: project }} onRetryTurn={retryTurn} renderEntry={renderEntry} />
    </>
}
