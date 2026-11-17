import { cn } from '@/lib/utils'
import { useNow } from '@/lib/useNow'
import { useEffect, useState, type ComponentProps, type ReactNode } from 'react'
import { Check, FolderPlus, MessageSquare } from 'lucide-react'

import { SectionHeading } from '@/components/app/section-heading'
import { StatusDot, statusTextClassName, type StatusTone } from '@/components/app/status'
import { Well } from '@/components/app/well'
import { Button } from '@/components/ui/button'
import { useStore } from '@/store'
import { ATTENTION_KIND_LABELS, attentionFlow, attentionTimes, attentionTitle, openAttentionItem, openChat, openMission, openRun, openTrigger, refreshAttention, useAttentionFailed, useAttentionItems } from '@/app/useAttention'
import { triggerProblem } from '@/features/triggers/model/triggerForm'
import { triggerKey } from '@/lib/api/triggersApi'
import { missionTime } from '@/features/missions/model/missionModel'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'
import { defaultProjectChoice, projectLabel } from '@/features/projects/model/projectChoices'
import { formatRunDate, runFlowTitle, runTitle } from '@/features/runs/model/runOverviewModel'
import { formatDuration, formatRunStatusLabel, formatTimestamp, isCompletedFailure } from '@/features/runs/model/shared'
import { openRunFlowDialog } from '@/features/launch'
import { useOverviewChats } from './hooks/useOverviewChats'
import {
    finishedRows,
    inProgress,
    lastChat,
    markAttentionSeen,
    markFinishedSeen,
    readFinishedWatermark,
    recentlyFinished,
    sourceOf,
    tally,
    type Mark,
    type Source,
    type Started,
} from './model/overviewModel'

// Colour marks exceptions: failed, waiting and canceled carry a word in their tone; the rest read as a dot.
const MARKS: Record<Mark, { label: string; tone: StatusTone; exception?: true }> = {
    waiting: { label: 'Waiting on you', tone: 'warning', exception: true },
    running: { label: 'Running', tone: 'info' },
    completed: { label: 'Completed', tone: 'success' },
    failed: { label: 'Failed', tone: 'destructive', exception: true },
    ended: { label: 'Canceled', tone: 'neutral', exception: true },
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

/** The word for an exceptional state, in its tone, after a meta line; a run that ended at a failure exit names the exit. */
function ExceptionWord({ item }: { item: Started }) {
    const { label, tone, exception } = MARKS[item.mark]
    const word = item.kind === 'run' && isCompletedFailure(item.run) ? formatRunStatusLabel(item.run) : label
    return exception ? <span className={`shrink-0 whitespace-pre ${statusTextClassName(tone)}`}> · {word}</span> : null
}

/**
 * A row's meta line. What the row is gives way first; the project keeps its name unless it alone
 * is wider than the line, so a long flow title can't push it out of sight on a phone.
 */
function MetaLine({ lead, project, children }: { lead: string; project: string | null; children?: ReactNode }) {
    return (
        <span className="flex min-w-0 items-baseline text-xs text-muted-foreground">
            <span className="min-w-0 shrink-[100] truncate whitespace-pre">{lead}</span>
            {/* The lead gives way first; the project truncates only past the row's whole width. */}
            {project ? <span data-testid="overview-row-project" className="max-w-full shrink-0 truncate whitespace-pre">{lead ? ' · ' : ''}{project}</span> : null}
            {children}
        </span>
    )
}

function Section({ heading, count, testId, children }: { heading: ReactNode; count?: ReactNode; testId: string; children: ReactNode }) {
    return (
        <section data-testid={testId} className="flex flex-col gap-2.5">
            <SectionHeading className="text-base" count={count}>{heading}</SectionHeading>
            {children}
        </section>
    )
}

/** A one-line empty state: a well, with a check when nothing left to do is good news. */
function EmptyWell({ children, check = true, ...props }: ComponentProps<'div'> & { check?: boolean }) {
    return (
        <Well className={cn('flex items-center gap-2.5 py-3.5 text-sm', !check && 'text-muted-foreground')} {...props}>
            {check ? <Check aria-hidden="true" className="size-4 shrink-0 text-success" /> : null}
            {children}
        </Well>
    )
}

const listPanelClass = 'divide-y divide-border overflow-hidden rounded-xl border border-border'
// Rows the Finished list shows before "Show more"; failures come first, so they are never the ones hidden.
const FINISHED_LIMIT = 8
// Rows Needs you shows before "Show more"; the Inbox lists every one.
const NEEDS_YOU_LIMIT = 6

const retryRuns = () => window.dispatchEvent(new Event('spark:runs-overview-resync-required'))

/** Opens the Runs view showing only failed runs, from every project. */
function openFailedRuns() {
    useStore.getState().updateRunsListSession({ statusFilter: 'failed', projectFilter: null })
    useStore.getState().setViewMode('runs')
}

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
// A run's flow is worth naming under its title only when the title doesn't already say it.
const distinctFlowTitle = (item: Started & { kind: 'run' }) => {
    const flow = runFlowTitle(item.run)
    return flow.toLowerCase() === itemTitle(item).toLowerCase() ? null : flow
}
const openItem = (item: Started) => (item.kind === 'run' ? openRun(item.id) : openMission(item.id, item.mission.project_path))
const itemProject = (item: Started) => (item.kind === 'run' ? item.run.project_path : item.mission.project_path)
const itemDuration = (item: Started) => (item.kind === 'run'
    ? formatDuration(item.run.started_at, item.run.ended_at, undefined, undefined, item.run.idle_ms)
    : formatDuration(missionTime(item.mission.started_at), missionTime(item.mission.closed?.at)))

/** Pick up where you left off: the chat you were last in, what needs you, what finished, and what to start. */
export function OverviewPanel() {
    // ponytail: polls beside the activity bar's own poll while open; share one store-held list if the endpoint gets costly.
    const loadedAttention = useAttentionItems()
    const attentionFailed = useAttentionFailed()
    const attention = loadedAttention ?? []
    const registry = useStore((state) => state.projectRegistry)
    const runs = useStore((state) => state.runsListSession.runs)
    const missions = useStore((state) => state.missionBoard)
    const listsLoaded = useStore((state) => Boolean(state.runsListSession.fetched) && state.missionBoardLoaded)
    // Until the run history loads, an empty list says nothing about what ran.
    const runsFetched = useStore((state) => Boolean(state.runsListSession.fetched))
    const runsFailed = useStore((state) => state.runsListSession.status === 'error')
    const runsLoading = !runsFetched && !runsFailed
    const { chats, loaded, failed: chatsFailed, retry: retryChats } = useOverviewChats()
    // What this visit lists is judged against what earlier visits showed; what it shows is seen from the next one on.
    const [finishedWatermark] = useState(readFinishedWatermark)
    const [showRuns, setShowRuns] = useState(false)
    const [showAllFinished, setShowAllFinished] = useState(false)
    const [showAllNeedsYou, setShowAllNeedsYou] = useState(false)
    useEffect(() => { if (loadedAttention) markAttentionSeen(loadedAttention) }, [loadedAttention])
    useEffect(() => { if (listsLoaded) markFinishedSeen(runs, missions) }, [listsLoaded, runs, missions])
    const now = useNow(runs.some((run) => run.status === 'running'))
    const attentionTime = attentionTimes(attention, runs, now)
    // A trigger that can't run, or whose firings fail, quietly starts nothing: it needs you too.
    const triggers = useStore((state) => state.triggersSession.triggers)
    const failingTriggers = triggers.flatMap((trigger) => {
        const problem = triggerProblem(trigger)
        return problem ? [{ trigger, problem }] : []
    })

    const last = lastChat(chats, runs, missions)
    const counts = last ? tally(last.started) : {}
    const { items: finished, sinceLastLook } = recentlyFinished(runs, missions, finishedWatermark, now)
    const failures = finished.filter((item) => item.mark === 'failed').length
    const rows = finishedRows(finished)
    const shownRows = showAllFinished ? rows : rows.slice(0, FINISHED_LIMIT)
    const running = inProgress(runs, missions, chats)

    return (
        <div data-testid="overview-view" className="h-full overflow-y-auto">
            <div className="mx-auto flex w-full max-w-4xl flex-col gap-9 px-6 pb-12 pt-8 lg:px-8">
                <header className="flex flex-wrap items-end justify-between gap-x-6 gap-y-4">
                    <div>
                        <h1 className="text-xl font-semibold tracking-tight">Pick up where you left off</h1>
                        <p className="mt-1 text-sm text-muted-foreground">Across all projects</p>
                    </div>
                    <div data-testid="overview-start" className="flex flex-wrap gap-2">
                        <Button type="button" variant="outline" data-testid="overview-run-flow" onClick={() => openRunFlowDialog()}>Run a flow…</Button>
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

                {/* A fresh install has nothing to pick up: point at the first step. */}
                {listsLoaded && (loaded || Object.keys(registry).length === 0) && runs.length === 0 && missions.length === 0 && chats.length === 0 ? (
                    <Well data-testid="overview-get-started" className="flex flex-wrap items-center justify-between gap-3 py-3.5 text-sm">
                        <p className="min-w-0 flex-1 basis-64">
                            <span className="font-medium">New to Spark?</span>{' '}
                            <span className="text-muted-foreground">Add the folder of a project for agents to work in, then start a chat or run a flow from the buttons above.</span>
                        </p>
                        <Button
                            type="button"
                            size="sm"
                            data-testid="overview-add-project"
                            onClick={() => {
                                useStore.getState().setViewMode('home')
                                window.dispatchEvent(new Event('spark:add-project'))
                            }}
                        >
                            <FolderPlus aria-hidden="true" />
                            Add a project
                        </Button>
                    </Well>
                ) : null}

                <Section testId="overview-needs-you" heading="Needs you" count={loadedAttention ? attention.length + failingTriggers.length : undefined}>
                    {/* A failed check is not an all-clear: say so, and keep what was last known. */}
                    {attentionFailed ? (
                        <Well data-testid="overview-needs-you-error" role="alert" className="flex flex-wrap items-center justify-between gap-2 py-3 text-sm text-destructive">
                            {loadedAttention ? 'Couldn’t refresh what is waiting on you; this may be out of date.' : 'Couldn’t check what is waiting on you.'}
                            <Button type="button" size="sm" variant="outline" onClick={refreshAttention}>Try again</Button>
                        </Well>
                    ) : null}
                    {!loadedAttention ? (
                        attentionFailed ? null : <Well className="py-3.5 text-sm text-muted-foreground">Checking what is waiting on you…</Well>
                    ) : attention.length === 0 && failingTriggers.length === 0 ? (
                        attentionFailed ? null : <EmptyWell data-testid="overview-needs-you-empty">Nothing is waiting on you.</EmptyWell>
                    ) : (
                        <ul className={listPanelClass}>
                            {(showAllNeedsYou ? attention : attention.slice(0, NEEDS_YOU_LIMIT)).map((item) => (
                                <li key={`${item.kind}:${item.id}`}>
                                    <button
                                        type="button"
                                        data-testid="overview-needs-you-item"
                                        onClick={() => openAttentionItem(item)}
                                        className="flex w-full items-center gap-3 px-4 py-3 text-left outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
                                    >
                                        <StatusDot tone="warning" />
                                        <span className="flex min-w-0 flex-1 flex-col gap-px">
                                            <span className="truncate text-sm font-medium">{attentionTitle(item, runs)}</span>
                                            {/* A gate's question says what this run waits on, where the kind alone reads the same for every run of a flow.
                                                A long question gives way; the project stays readable. On a phone the question gets two lines of its own. */}
                                            <span className="flex min-w-0 text-xs text-muted-foreground max-sm:flex-col">
                                                <span className="min-w-0 truncate max-sm:line-clamp-2 max-sm:whitespace-normal">{item.question || ATTENTION_KIND_LABELS[item.kind]}</span>
                                                <span className="flex min-w-0 shrink-0 max-sm:shrink">
                                                    <span className="whitespace-pre max-sm:hidden"> · </span>
                                                    {/* On a phone a long flow name gives way before the project. */}
                                                    <MetaLine lead={attentionFlow(item, runs) ?? ''} project={projectLabel(registry, item.project_path)} />
                                                </span>
                                            </span>
                                        </span>
                                        <span data-testid="overview-needs-you-time" className="min-w-12 shrink-0 text-right text-xs text-muted-foreground">{attentionTime(item)}</span>
                                    </button>
                                </li>
                            ))}
                            {failingTriggers.map(({ trigger, problem }) => (
                                <li key={`trigger:${triggerKey(trigger)}`}>
                                    <button
                                        type="button"
                                        data-testid="overview-needs-you-trigger"
                                        onClick={() => openTrigger(trigger.project_path, trigger.id)}
                                        className="flex w-full items-center gap-3 px-4 py-3 text-left outline-none transition-colors hover:bg-muted/60 focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
                                    >
                                        <StatusDot tone="destructive" />
                                        <span className="flex min-w-0 flex-1 flex-col gap-px">
                                            <span className="truncate text-sm font-medium">{trigger.name}</span>
                                            <span className="flex min-w-0 text-xs text-muted-foreground max-sm:flex-col">
                                                <span className="min-w-0 truncate max-sm:line-clamp-2 max-sm:whitespace-normal" title={problem.detail}>
                                                    <span className="text-destructive">Trigger · {problem.label}</span>: {problem.detail}
                                                </span>
                                                <span className="flex min-w-0 shrink-0 max-sm:shrink">
                                                    <span className="whitespace-pre max-sm:hidden"> · </span>
                                                    <MetaLine lead="" project={projectLabel(registry, trigger.project_path)} />
                                                </span>
                                            </span>
                                        </span>
                                        <span className="min-w-12 shrink-0 text-right text-xs text-muted-foreground">{trigger.state.last_fired_at ? formatRunDate(trigger.state.last_fired_at, now) : ''}</span>
                                    </button>
                                </li>
                            ))}
                        </ul>
                    )}
                    {attention.length > NEEDS_YOU_LIMIT ? (
                        <Button type="button" variant="ghost" size="sm" data-testid="overview-needs-you-show-all" aria-expanded={showAllNeedsYou} onClick={() => setShowAllNeedsYou((value) => !value)} className="self-start">
                            {showAllNeedsYou ? 'Show fewer' : `Show ${attention.length - NEEDS_YOU_LIMIT} more`}
                        </Button>
                    ) : null}
                </Section>

                {/* Until the runs load, a count would say nothing is running. */}
                <Section testId="overview-running" heading="Running now" count={runsFetched ? running.length : undefined}>
                    {runsFailed ? (
                        <Well data-testid="overview-runs-error" role="alert" className="flex flex-wrap items-center justify-between gap-2 py-3 text-sm text-destructive">
                            {runsFetched ? 'Couldn’t refresh your runs; what is shown here may be out of date.' : 'Couldn’t load your runs, so what is running and what finished can’t be shown.'}
                            <Button type="button" size="sm" variant="outline" onClick={retryRuns}>Try again</Button>
                        </Well>
                    ) : null}
                    {running.length === 0 ? (
                        runsFailed ? null : runsLoading ? (
                            <Well className="py-3.5 text-sm text-muted-foreground">Loading runs…</Well>
                        ) : <EmptyWell check={false} data-testid="overview-running-empty">Nothing is running.</EmptyWell>
                    ) : (
                        <ul className={listPanelClass}>
                            {running.map((item) => (
                                <li key={`${item.kind}:${item.id}`} data-testid="overview-running-item" data-item-id={item.id} className="relative flex items-center gap-3 px-4 py-3">
                                    <MarkDot mark="running" />
                                    <span className="flex min-w-0 flex-1 flex-col gap-px">
                                        {item.kind === 'chat' ? (
                                            <TitleLink onClick={() => openChat(item.chat.project_path, item.chat.conversation_id)}>{item.chat.title}</TitleLink>
                                        ) : (
                                            <TitleLink onClick={() => openItem(item)}>{itemTitle(item)}</TitleLink>
                                        )}
                                        <MetaLine
                                            lead={(item.kind === 'chat'
                                                ? ['Chat, replying']
                                                : [item.kind === 'run' ? 'Run' : 'Mission', item.kind === 'run' ? distinctFlowTitle(item) : null]
                                            ).filter(Boolean).join(' · ')}
                                            project={projectLabel(registry, item.kind === 'chat' ? item.chat.project_path : itemProject(item))}
                                        />
                                    </span>
                                    {item.kind === 'run' ? (
                                        <span className="shrink-0 text-right text-xs tabular-nums text-muted-foreground" title="Running for">{formatDuration(item.run.started_at, null, 'running', now, item.run.idle_ms)}</span>
                                    ) : null}
                                    <span className="min-w-12 shrink-0 text-right text-xs text-muted-foreground">{when(item.at, now)}</span>
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
                                                {item.kind === 'mission' ? 'Mission' : runFlowTitle(item.run)} · {when(item.at, now)}
                                                <ExceptionWord item={item} />
                                            </span>
                                        </li>
                                    ))}
                                </ul>
                            ) : null}
                        </div>
                    ) : chatsFailed ? (
                        <Well data-testid="overview-chats-error" role="alert" className="flex flex-wrap items-center justify-between gap-2 py-3 text-sm text-destructive">
                            Couldn’t load your chats.
                            <Button type="button" size="sm" variant="outline" onClick={retryChats}>Try again</Button>
                        </Well>
                    ) : loaded ? (
                        <EmptyWell check={false}>No chats yet.</EmptyWell>
                    ) : (
                        <Well className="py-3.5 text-sm text-muted-foreground">Loading chats…</Well>
                    )}
                </Section>

                <Section
                    testId="overview-finished"
                    heading={`Finished ${sinceLastLook ? 'since you last looked' : 'in the last day'}`}
                    count={runsFetched ? finished.length : undefined}
                >
                    {failures ? (
                        <p className="-mt-1 text-sm">
                            <Button type="button" variant="link" data-testid="overview-failure-count" onClick={openFailedRuns} title="See every failed run in Runs" className="h-auto p-0 text-sm font-medium text-destructive underline underline-offset-4">
                                {failures} failed
                            </Button>
                            <span className="text-muted-foreground"> · listed first</span>
                        </p>
                    ) : null}
                    {finished.length === 0 ? (
                        runsFailed ? (
                            <EmptyWell check={false} data-testid="overview-finished-unavailable">Finished runs can’t be shown until your runs load.</EmptyWell>
                        ) : runsLoading ? (
                            <Well className="py-3.5 text-sm text-muted-foreground">Loading runs…</Well>
                        ) : <EmptyWell>{sinceLastLook ? 'Nothing has finished since you last looked.' : 'Nothing has finished in the last day.'}</EmptyWell>
                    ) : (
                        <ul className={listPanelClass}>
                            {shownRows.map(({ key, latest: item, count, isNew }) => (
                                <li key={key} data-testid="overview-finished-item" data-item-id={item.id} data-count={count} className="relative flex items-center gap-3 px-4 py-3">
                                    <MarkDot mark={item.mark} />
                                    <span className="flex min-w-0 flex-1 flex-col gap-px">
                                        <TitleLink onClick={() => openItem(item)}>{itemTitle(item)}</TitleLink>
                                        <MetaLine
                                            lead={[
                                                count > 1 ? `${count} runs, latest shown` : item.kind === 'run' ? 'Run' : 'Mission',
                                                item.kind === 'run' ? distinctFlowTitle(item) : null,
                                            ].filter(Boolean).join(' · ')}
                                            project={projectLabel(registry, itemProject(item))}
                                        >
                                            {count > 1 ? null : <SourceLink source={sourceOf(item, chats, missions)} />}
                                            <ExceptionWord item={item} />
                                        </MetaLine>
                                    </span>
                                    {isNew ? <span data-testid="overview-finished-new" className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-xs font-medium text-primary">New</span> : null}
                                    <span className="w-14 shrink-0 text-right text-xs tabular-nums text-muted-foreground max-sm:hidden">{itemDuration(item)}</span>
                                    {/* When it finished, said so: Runs lists when each started. */}
                                    <span className="min-w-12 shrink-0 whitespace-nowrap text-right text-xs text-muted-foreground" title={item.at ? `Finished ${formatTimestamp(new Date(item.at).toISOString())}` : undefined}>{item.at ? `ended ${when(item.at, now)}` : ''}</span>
                                </li>
                            ))}
                        </ul>
                    )}
                    {rows.length > FINISHED_LIMIT ? (
                        <Button type="button" variant="ghost" size="sm" data-testid="overview-finished-show-all" aria-expanded={showAllFinished} onClick={() => setShowAllFinished((value) => !value)} className="self-start">
                            {showAllFinished ? 'Show fewer' : `Show ${rows.length - FINISHED_LIMIT} more`}
                        </Button>
                    ) : null}
                </Section>
            </div>
        </div>
    )
}
