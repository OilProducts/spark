import { useEffect, useState, type ComponentProps, type ReactNode } from 'react'
import { Check, MessageSquare } from 'lucide-react'

import { SectionHeading } from '@/components/app/section-heading'
import { StatusDot, statusTextClassName, type StatusTone } from '@/components/app/status'
import { Well } from '@/components/app/well'
import { Button } from '@/components/ui/button'
import { useStore } from '@/store'
import { ATTENTION_KIND_LABELS, openAttentionItem, openChat, openMission, openRun, useAttentionItems } from '@/app/useAttention'
import { missionTime } from '@/features/missions/model/missionModel'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'
import { defaultProjectChoice, projectLabel } from '@/features/projects/model/projectChoices'
import { flowTitle, formatRunDate, runTitle } from '@/features/runs/model/runOverviewModel'
import { formatDuration } from '@/features/runs/model/shared'
import { useOverviewChats } from './hooks/useOverviewChats'
import {
    finishedSince,
    lastChat,
    markAttentionSeen,
    markFinishedSeen,
    readFinishedWatermark,
    sourceOf,
    tally,
    type Mark,
    type Source,
    type Started,
} from './model/overviewModel'

// Colour marks exceptions: failed and waiting carry a word in their tone; the rest read as a dot.
const MARKS: Record<Mark, { label: string; tone: StatusTone; exception?: true }> = {
    waiting: { label: 'Waiting on you', tone: 'warning', exception: true },
    running: { label: 'Running', tone: 'info' },
    completed: { label: 'Completed', tone: 'success' },
    failed: { label: 'Failed', tone: 'destructive', exception: true },
    ended: { label: 'Stopped', tone: 'neutral' },
    draft: { label: 'Draft', tone: 'neutral' },
}
const MARK_ORDER: Mark[] = ['waiting', 'running', 'completed', 'failed', 'ended', 'draft']

const when = (at: number, now: number) => (at ? formatRunDate(new Date(at).toISOString(), now) : '')

/** The row's status dot, named for assistive tech unless the row's ExceptionWord already says it. */
function MarkDot({ mark, ...props }: Omit<ComponentProps<typeof StatusDot>, 'tone'> & { mark: Mark }) {
    const { label, tone, exception } = MARKS[mark]
    return (
        <>
            <StatusDot tone={tone} hollow={mark === 'draft'} data-mark={mark} {...props} />
            {exception ? null : <span className="sr-only">{label}</span>}
        </>
    )
}

/** The word for an exceptional state, in its tone, after a meta line. */
function ExceptionWord({ mark }: { mark: Mark }) {
    const { label, tone, exception } = MARKS[mark]
    return exception ? <span className={`shrink-0 whitespace-pre ${statusTextClassName(tone)}`}> · {label}</span> : null
}

function Section({ heading, count, testId, children }: { heading: ReactNode; count?: ReactNode; testId: string; children: ReactNode }) {
    return (
        <section data-testid={testId} className="flex flex-col gap-2.5">
            <SectionHeading className="text-base" count={count}>{heading}</SectionHeading>
            {children}
        </section>
    )
}

/** A one-line empty state: a well with a check. */
function EmptyWell({ children, ...props }: ComponentProps<'div'>) {
    return (
        <Well className="flex items-center gap-2.5 py-3.5 text-sm" {...props}>
            <Check aria-hidden="true" className="size-4 shrink-0 text-success" />
            {children}
        </Well>
    )
}

const listPanelClass = 'divide-y divide-border overflow-hidden rounded-xl border border-border'

/**
 * A row title that opens its run or mission: navigation in the foreground colour, underlined on
 * hover. Its hit area covers the whole `relative` row; anything else clickable in the row sits above it.
 */
function TitleLink({ onClick, children }: { onClick: () => void; children: ReactNode }) {
    return (
        <Button type="button" variant="link" onClick={onClick} className="h-auto min-w-0 max-w-full shrink justify-start p-0 text-left text-sm font-medium text-foreground after:absolute after:inset-0">
            <span className="min-w-0 truncate">{children}</span>
        </Button>
    )
}

function SourceLink({ source }: { source: Source | null }) {
    if (!source) return null
    const [label, open] = source.kind === 'chat'
        ? [source.chat.title, () => openChat(source.chat.project_path, source.chat.conversation_id)]
        : [source.mission.fields.title || 'Untitled mission', () => openMission(source.mission.id, source.mission.project_path)]
    return (
        <>
            <span className="shrink-0 whitespace-pre"> · from </span>
            <Button type="button" variant="link" data-testid="overview-source-link" onClick={open} className="relative z-10 h-auto min-w-0 max-w-full shrink justify-start p-0 text-left text-xs font-normal">
                <span className="min-w-0 truncate">{label}</span>
            </Button>
        </>
    )
}

const itemTitle = (item: Started) => (item.kind === 'run' ? runTitle(item.run) : item.mission.fields.title || 'Untitled mission')
const openItem = (item: Started) => (item.kind === 'run' ? openRun(item.id) : openMission(item.id, item.mission.project_path))
const itemProject = (item: Started) => (item.kind === 'run' ? item.run.project_path : item.mission.project_path)
const itemDuration = (item: Started) => (item.kind === 'run'
    ? formatDuration(item.run.started_at, item.run.ended_at)
    : formatDuration(missionTime(item.mission.started_at), missionTime(item.mission.closed?.at)))

/** Pick up where you left off: the chat you were last in, what needs you, what finished, and what to start. */
export function OverviewPanel() {
    // ponytail: polls beside the activity bar's own poll while open; share one store-held list if the endpoint gets costly.
    const loadedAttention = useAttentionItems()
    const attention = loadedAttention ?? []
    const registry = useStore((state) => state.projectRegistry)
    const runs = useStore((state) => state.runsListSession.runs)
    const missions = useStore((state) => state.missionBoard)
    const listsLoaded = useStore((state) => Boolean(state.runsListSession.fetched) && state.missionBoardLoaded)
    const { chats, loaded } = useOverviewChats()
    // What this visit lists is judged against what earlier visits showed; what it shows is seen from the next one on.
    const [finishedWatermark] = useState(readFinishedWatermark)
    const [showRuns, setShowRuns] = useState(false)
    useEffect(() => { if (loadedAttention) markAttentionSeen(loadedAttention) }, [loadedAttention])
    useEffect(() => { if (listsLoaded) markFinishedSeen(runs, missions) }, [listsLoaded, runs, missions])
    // eslint-disable-next-line react-hooks/purity -- render-time clock for relative dates; the view re-renders on run and mission updates
    const now = Date.now()

    const last = lastChat(chats, runs, missions)
    const counts = last ? tally(last.started) : {}
    const finished = finishedSince(runs, missions, finishedWatermark, now)
    const failures = finished.filter((item) => item.mark === 'failed').length

    return (
        <div data-testid="overview-view" className="h-full overflow-y-auto">
            <div className="mx-auto flex w-full max-w-4xl flex-col gap-9 px-6 pb-12 pt-8 lg:px-8">
                <header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-4">
                    <div>
                        <h1 className="text-xl font-semibold tracking-tight">Pick up where you left off</h1>
                        <p className="mt-1 text-sm text-muted-foreground">Across all projects</p>
                    </div>
                    <div data-testid="overview-start" className="flex flex-wrap gap-2">
                        <Button type="button" variant="ghost" data-testid="overview-run-flow" onClick={() => useStore.getState().setViewMode('editor')}>Run a flow…</Button>
                        <ProjectPicker
                            heading="New mission in"
                            testId="overview-new-mission-picker"
                            defaultProjectPath={defaultProjectChoice(registry)}
                            onPick={(projectPath) => {
                                useStore.getState().setViewMode('missions')
                                window.dispatchEvent(new CustomEvent('spark:new-mission', { detail: projectPath }))
                            }}
                        >
                            <Button type="button" variant="outline" data-testid="overview-new-mission">New mission</Button>
                        </ProjectPicker>
                        <ProjectPicker
                            heading="New chat in"
                            testId="overview-new-chat-picker"
                            onPick={(projectPath) => {
                                useStore.getState().setViewMode('home')
                                window.dispatchEvent(new CustomEvent('spark:new-chat', { detail: projectPath }))
                            }}
                        >
                            <Button type="button" data-testid="overview-new-chat">New chat</Button>
                        </ProjectPicker>
                    </div>
                </header>

                <Section testId="overview-needs-you" heading="Needs you" count={attention.length}>
                    {attention.length === 0 ? (
                        <EmptyWell data-testid="overview-needs-you-empty">Nothing is waiting on you.</EmptyWell>
                    ) : (
                        <ul className={listPanelClass}>
                            {attention.map((item) => (
                                <li key={`${item.kind}:${item.id}`}>
                                    <button
                                        type="button"
                                        data-testid="overview-needs-you-item"
                                        onClick={() => openAttentionItem(item)}
                                        className="flex w-full items-center gap-3 px-4 py-3 text-left outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
                                    >
                                        <StatusDot tone="warning" />
                                        <span className="flex min-w-0 flex-1 flex-col gap-px">
                                            <span className="truncate text-sm font-medium">{item.title || item.run_id || item.id}</span>
                                            <span className="truncate text-xs text-muted-foreground">{ATTENTION_KIND_LABELS[item.kind]} · {projectLabel(registry, item.project_path)}</span>
                                        </span>
                                    </button>
                                </li>
                            ))}
                        </ul>
                    )}
                </Section>

                <Section testId="overview-last-chat" heading="You were last in">
                    {last ? (
                        <div className="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-border px-4 py-4">
                            <div className="flex min-w-0 flex-1 basis-80 items-start gap-3">
                                <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground"><MessageSquare aria-hidden="true" className="size-4" /></span>
                                <div className="flex min-w-0 flex-col gap-0.5">
                                    <p data-testid="overview-last-chat-title" data-conversation-id={last.chat.conversation_id} className="truncate text-base font-medium">{last.chat.title}</p>
                                    <p className="truncate text-xs text-muted-foreground">
                                        Chat · {projectLabel(registry, last.chat.project_path)} · {when(last.at, now)}
                                        {MARK_ORDER.filter((mark) => counts[mark]).map((mark) => (
                                            <span key={mark} data-testid="overview-tally">
                                                {' · '}
                                                <span data-mark={mark} className={MARKS[mark].exception ? statusTextClassName(MARKS[mark].tone) : undefined}>{counts[mark]} {MARKS[mark].label.toLowerCase()}</span>
                                            </span>
                                        ))}
                                    </p>
                                </div>
                            </div>
                            <div className="flex shrink-0 gap-2">
                                {last.started.length > 0 ? (
                                    <Button type="button" size="sm" variant="ghost" aria-expanded={showRuns} data-testid="overview-show-runs" onClick={() => setShowRuns((value) => !value)}>
                                        {showRuns ? 'Hide its runs' : 'Show its runs'}
                                    </Button>
                                ) : null}
                                <Button type="button" size="sm" variant="outline" data-testid="overview-continue" onClick={() => openChat(last.chat.project_path, last.chat.conversation_id)}>Continue</Button>
                            </div>
                            {showRuns ? (
                                <ul data-testid="overview-started-list" className="w-full basis-full divide-y divide-border border-t border-border">
                                    {last.started.map((item) => (
                                        <li key={`${item.kind}:${item.id}`} data-testid="overview-started-item" className="relative flex items-center gap-3 py-2">
                                            <MarkDot mark={item.mark} />
                                            <TitleLink onClick={() => openItem(item)}>{itemTitle(item)}</TitleLink>
                                            <span className="ml-auto flex shrink-0 items-baseline text-xs text-muted-foreground">
                                                {item.kind === 'mission' ? 'Mission' : flowTitle(item.run.flow_name)} · {when(item.at, now)}
                                                <ExceptionWord mark={item.mark} />
                                            </span>
                                        </li>
                                    ))}
                                </ul>
                            ) : null}
                        </div>
                    ) : loaded ? (
                        <EmptyWell>No chats yet.</EmptyWell>
                    ) : (
                        <Well className="py-3.5 text-sm text-muted-foreground">Loading chats…</Well>
                    )}
                </Section>

                <Section
                    testId="overview-finished"
                    heading={`Finished ${finishedWatermark === null ? 'in the last day' : 'since you last looked'}`}
                    count={<>{finished.length}{failures ? <span data-testid="overview-failure-count" className="text-destructive"> · {failures} failed</span> : null}</>}
                >
                    {finished.length === 0 ? (
                        <EmptyWell>Nothing has finished since.</EmptyWell>
                    ) : (
                        <ul className={listPanelClass}>
                            {finished.map((item) => (
                                <li key={`${item.kind}:${item.id}`} data-testid="overview-finished-item" data-item-id={item.id} className="relative flex items-center gap-3 px-4 py-3">
                                    <MarkDot mark={item.mark} />
                                    <span className="flex min-w-0 flex-1 flex-col gap-px">
                                        <TitleLink onClick={() => openItem(item)}>{itemTitle(item)}</TitleLink>
                                        <span className="flex min-w-0 items-baseline text-xs text-muted-foreground">
                                            <span className="truncate">
                                                {[item.kind === 'run' ? 'Run' : 'Mission', item.kind === 'run' ? flowTitle(item.run.flow_name) : null, projectLabel(registry, itemProject(item))].filter(Boolean).join(' · ')}
                                            </span>
                                            <SourceLink source={sourceOf(item, chats, missions)} />
                                            <ExceptionWord mark={item.mark} />
                                        </span>
                                    </span>
                                    <span className="w-14 shrink-0 text-right text-xs tabular-nums text-muted-foreground">{itemDuration(item)}</span>
                                    <span className="min-w-12 shrink-0 text-right text-xs text-muted-foreground">{when(item.at, now)}</span>
                                </li>
                            ))}
                        </ul>
                    )}
                </Section>
            </div>
        </div>
    )
}
