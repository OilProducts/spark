import { useMemo, useState, type ReactNode } from 'react'
import { Check, CircleHelp, Flag, Pencil, Play, X, Zap } from 'lucide-react'
import { useStore } from '@/store'
import { buildRunsHash } from '@/app/runsRouting'
import { InlineError } from '@/components/app/inline-error'
import { SectionHeading } from '@/components/app/section-heading'
import { statusTextClassName } from '@/components/app/status'
import { groupTranscript } from '@/components/app/transcript/groupTranscript'
import {
    AssistantTurnHeader,
    EventLine,
    MessageRow,
    RunCard,
    ShowThinkingToggle,
    ThinkingRow,
    ToolCallGroupRow,
    ToolCallRow,
} from '@/components/app/transcript/SegmentRows'
import { Empty, EmptyDescription } from '@/components/ui/empty'
import { ProjectConversationRequestUserInputCard } from '@/features/projects/components/ProjectConversationRequestUserInputCard'
import {
    getConversationFlowLaunches,
    getConversationTimelineEntries,
    hydrateConversationRecordFromSnapshot,
} from '@/features/projects/model/projectsHomeState'
import type { ConversationTimelineEntry } from '@/features/projects/model/types'
import { loadConversationSegmentToolOutput } from '@/features/projects/services/conversationArtifacts'
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
type UserEntry = Extract<Entry, { kind: 'message' }>
type ToolCallEntry = Extract<Entry, { kind: 'tool_call' }>
type NoticeEntry = Extract<Entry, { kind: 'final_separator' }>
/** A user turn carries the mission's events; everything between two user turns is one assistant turn; a notice (such as a turn that could not start) stands alone. */
type Turn = { kind: 'user'; entry: UserEntry } | { kind: 'assistant'; entries: Entry[]; worked: string | null } | { kind: 'notice'; entry: NoticeEntry }

function splitTurns(entries: Entry[]): Turn[] {
    const turns: Turn[] = []
    let open: Extract<Turn, { kind: 'assistant' }> | null = null
    for (const entry of entries) {
        if (entry.kind === 'message' && entry.role === 'user') { open = null; turns.push({ kind: 'user', entry }); continue }
        const worked = entry.kind === 'final_separator' ? /^Worked(?: for (.+))?$/.exec(entry.label) : null
        if (entry.kind === 'final_separator' && !worked) {
            open = null
            // The outcome closes the thread instead; any other notice stays visible.
            if (!entry.label.startsWith('Closed as ')) turns.push({ kind: 'notice', entry })
            continue
        }
        if (!open) { open = { kind: 'assistant', entries: [], worked: null }; turns.push(open) }
        // "Worked for 16s" becomes the turn header's meta rather than a centered rule.
        if (worked) open.worked = worked[1] ?? open.worked
        else open.entries.push(entry)
    }
    return turns
}

const isThinking = (entry: Entry) => entry.kind === 'message' && entry.role === 'assistant' && entry.presentation === 'thinking'
const live = (status: string) => status !== 'complete' && status !== 'failed'
const toggleKey = (set: (update: (current: Record<string, boolean>) => Record<string, boolean>) => void) => (id: string) => set(current => ({ ...current, [id]: !current[id] }))

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
    const [openGroups, setOpenGroups] = useState<Record<string, boolean>>({})
    const [expandedToolCalls, setExpandedToolCalls] = useState<Record<string, boolean>>({})
    const [expandedThinking, setExpandedThinking] = useState<Record<string, boolean>>({})
    // A truncated output is fetched in full the first time its call opens; `{}` while loading.
    const [fullOutputs, setFullOutputs] = useState<Record<string, { output?: string; error?: string }>>({})
    function toggleToolCall(entry: ToolCallEntry) {
        const id = entry.toolCall.id
        setExpandedToolCalls(current => ({ ...current, [id]: !current[id] }))
        if (expandedToolCalls[id] || !entry.toolCall.outputTruncated || fullOutputs[entry.id]) return
        setFullOutputs(current => ({ ...current, [entry.id]: {} }))
        void loadConversationSegmentToolOutput(conversationId, entry.id, project)
            .then(payload => setFullOutputs(current => ({ ...current, [entry.id]: { output: payload.output } })))
            .catch(() => setFullOutputs(current => ({ ...current, [entry.id]: { error: 'Unable to load full output.' } })))
    }
    const record = useMemo(() => snapshot ? hydrateConversationRecordFromSnapshot(snapshot) : null, [snapshot])
    const entries = getConversationTimelineEntries(record)
    const launchesById = new Map(getConversationFlowLaunches(record).map(launch => [launch.id, launch]))
    const firstTurnId = entries.find(entry => entry.kind === 'message' && entry.role === 'user')?.id
    const title = (runId: string | null, fallback: string) => (runId && runs.get(runId)?.title) || fallback || runId || 'Run'
    const when = (at: string) => <time dateTime={at}>{formatTime(at)}</time>

    /** One entry of an assistant turn as its row; tool calls arrive through their group. */
    function row(entry: Entry): ReactNode {
        const key = `${entry.kind}:${entry.id}`
        switch (entry.kind) {
            case 'message':
                if (isThinking(entry)) {
                    // Thinking stays hidden until asked for, except while the agent is still at it.
                    return showThinking || live(entry.status)
                        ? <ThinkingRow key={key} entry={entry} isExpanded={expandedThinking[entry.id] === true} onToggleThinkingEntryExpanded={toggleKey(setExpandedThinking)} formatConversationTimestamp={formatTime} testIdPrefix="mission" />
                        : null
                }
                return <MessageRow key={key} entry={entry} enableCopy formatConversationTimestamp={formatTime} onRetryTurn={retryTurn} />
            case 'plan': return <MessageRow key={key} entry={entry} enableCopy formatConversationTimestamp={formatTime} />
            case 'flow_launch': {
                const launch = launchesById.get(entry.artifactId)
                if (!launch) return <li key={key}><EventLine at={when(entry.timestamp)}>Flow launch unavailable</EventLine></li>
                // A launch that never became a run keeps its error as an event line.
                if (!launch.run_id) {
                    return <li key={key}><EventLine icon={launch.status === 'launch_failed' ? <X className="text-destructive" /> : <Play className="text-info" />} at={when(entry.timestamp)} title={launch.launch_error ?? undefined}>
                        {launch.status === 'launch_failed' ? <><b>{launch.summary}</b> failed to launch{launch.launch_error && `: ${launch.launch_error}`}</> : <>Launching <b>{launch.summary}</b>…</>}
                    </EventLine></li>
                }
                const run = runs.get(launch.run_id)
                return <li key={key}><RunCard data-testid="mission-run-card" kicker={`Launched run · ${flowTitle(launch.flow_name)}`}
                    title={<RunLink runId={launch.run_id}>{title(launch.run_id, launch.summary)}</RunLink>}
                    tone={run ? runKinds[run.kind].tone : 'neutral'} status={run ? runStatusLabel(run) : 'Launched'} duration={run?.duration || undefined} /></li>
            }
            case 'request_user_input': return <li key={key} className="flex min-w-0 justify-start">
                <ProjectConversationRequestUserInputCard actionError={inputError} entry={entry} formatConversationTimestamp={formatTime} isSubmitting={submitting[entry.requestUserInput.requestId] === true} onSubmitRequestUserInput={submitInput} />
            </li>
            case 'mode_change': return <li key={key}><EventLine at={when(entry.timestamp)}>{entry.mode === 'plan' ? 'Switched to Plan mode' : 'Switched to Chat mode'}</EventLine></li>
            case 'context_compaction': return <li key={key}><EventLine at={when(entry.timestamp)}>{entry.content}</EventLine></li>
            // The worked separator is the turn header's meta; missions never raise run requests.
            default: return null
        }
    }

    function userTurn(entry: UserEntry) {
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

    /** A notice the runtime appended on its own, such as a turn that could not start, stays visible as an event line. */
    function noticeTurn(entry: NoticeEntry) {
        return <li key={`notice:${entry.id}`}><EventLine data-testid="mission-event" icon={<X className={statusTextClassName('destructive')} />} at={when(entry.timestamp)} title={entry.label}>{entry.label.split('\n')[0]}</EventLine></li>
    }

    function assistantTurn(turn: Extract<Turn, { kind: 'assistant' }>, key: string) {
        const blocks = groupTranscript(turn.entries, entry => entry.kind === 'tool_call' ? { kind: 'tool_call', toolCall: entry.toolCall } : isThinking(entry) ? { kind: 'thinking' } : { kind: 'other' })
        const meta = [formatTime(turn.entries[0]?.timestamp ?? ''), turn.worked && `worked ${turn.worked}`].filter(Boolean).join(' · ')
        return <li key={key} className="flex min-w-0 flex-col gap-3">
            <AssistantTurnHeader meta={meta} />
            <ul className="flex min-w-0 flex-col gap-3 pl-[30px]">
                {blocks.map(block => block.kind === 'tool_group'
                    ? <ToolCallGroupRow key={block.id} summary={block.summary} isExpanded={openGroups[block.id] === true} onToggle={() => toggleKey(setOpenGroups)(block.id)} testId={`mission-tool-group-${block.id}`}>
                        {block.members.map(member => member.kind === 'tool_call' && member.item.kind === 'tool_call'
                            ? <ToolCallRow key={member.toolCall.id} variant="line" entry={member.item} isExpanded={expandedToolCalls[member.toolCall.id] === true} onToggleToolCallExpanded={() => toggleToolCall(member.item as ToolCallEntry)}
                                fullOutput={fullOutputs[member.item.id]?.output ?? null} isLoadingFullOutput={fullOutputs[member.item.id] !== undefined && fullOutputs[member.item.id].output === undefined && fullOutputs[member.item.id].error === undefined}
                                loadFullOutputError={fullOutputs[member.item.id]?.error ?? null} testIdPrefix="mission" />
                            : row(member.item))}
                    </ToolCallGroupRow>
                    : row(block.item))}
            </ul>
        </li>
    }

    const turns = splitTurns(entries)
    return <>
        {error && <InlineError>{error}</InlineError>}
        {retryError && <InlineError>{retryError}</InlineError>}
        <SectionHeading as="h3" action={<ShowThinkingToggle pressed={showThinking} onPressedChange={setShowThinking} />}>Log</SectionHeading>
        {loading && !entries.length
            ? <p className="text-xs text-muted-foreground" aria-live="polite">Restoring thread history...</p>
            : !entries.length
                ? <Empty><EmptyDescription>No conversation history for this thread yet.</EmptyDescription></Empty>
                : <ol className="flex min-w-0 flex-col gap-4">{turns.map((turn, index) => turn.kind === 'user' ? userTurn(turn.entry) : turn.kind === 'notice' ? noticeTurn(turn.entry) : assistantTurn(turn, `turn:${turn.entries[0]?.id ?? index}`))}</ol>}
    </>
}
