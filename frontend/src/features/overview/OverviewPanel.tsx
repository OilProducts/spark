import { useEffect, useState, type ReactNode } from 'react'

import { Button } from '@/components/ui/button'
import { useStore } from '@/store'
import { ATTENTION_KIND_LABELS, openAttentionItem, openChat, openMission, openRun, useAttentionItems } from '@/app/useAttention'
import { ProjectPicker } from '@/features/projects/components/ProjectPicker'
import { defaultProjectChoice, projectLabel } from '@/features/projects/model/projectChoices'
import { flowTitle, formatRunDate, runTitle } from '@/features/runs/model/runOverviewModel'
import { useOverviewChats } from './hooks/useOverviewChats'
import {
    finishedSince,
    lastChat,
    markAttentionSeen,
    markFinishedSeen,
    readSeenFinished,
    sourceOf,
    tally,
    type Mark,
    type Source,
    type Started,
} from './model/overviewModel'

const MARKS: Record<Mark, { symbol: string; label: string; className: string }> = {
    waiting: { symbol: '?', label: 'Waiting on you', className: 'text-warning' },
    running: { symbol: '•', label: 'Running', className: 'text-primary' },
    completed: { symbol: '✓', label: 'Completed', className: 'text-success' },
    failed: { symbol: '✕', label: 'Failed', className: 'text-destructive' },
    ended: { symbol: '–', label: 'Stopped', className: 'text-muted-foreground' },
    draft: { symbol: '○', label: 'Draft', className: 'text-muted-foreground' },
}
const MARK_ORDER: Mark[] = ['waiting', 'running', 'completed', 'failed', 'ended', 'draft']

const when = (at: number, now: number) => (at ? formatRunDate(new Date(at).toISOString(), now) : '')

function StatusMark({ mark }: { mark: Mark }) {
    const { symbol, label, className } = MARKS[mark]
    return <span role="img" aria-label={label} title={label} data-mark={mark} className={`inline-block w-3 shrink-0 text-center ${className}`}>{symbol}</span>
}

function Section({ label, testId, children }: { label: ReactNode; testId: string; children: ReactNode }) {
    return (
        <section data-testid={testId} className="mt-7">
            <h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{label}</h2>
            {children}
        </section>
    )
}

const linkClass = 'text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring rounded-sm'

function Link({ onClick, children, testId }: { onClick: () => void; children: ReactNode; testId?: string }) {
    return <Button type="button" variant="link" data-testid={testId} onClick={onClick} className={`h-auto min-w-0 max-w-full shrink justify-start p-0 text-left ${linkClass}`}><span className="min-w-0 truncate">{children}</span></Button>
}

const itemTitle = (item: Started) => (item.kind === 'run' ? runTitle(item.run) : item.mission.fields.title || 'Untitled mission')
const openItem = (item: Started) => (item.kind === 'run' ? openRun(item.id) : openMission(item.id, item.mission.project_path))

/** Pick up where you left off: the chat you were last in, what needs you, what finished, and what to start. */
export function OverviewPanel() {
    // ponytail: polls beside the activity bar's own poll while open; share one store-held list if the endpoint gets costly.
    const loadedAttention = useAttentionItems()
    const attention = loadedAttention ?? []
    const registry = useStore((state) => state.projectRegistry)
    const runs = useStore((state) => state.runsListSession.runs)
    const missions = useStore((state) => state.missionBoard)
    const { chats, loaded } = useOverviewChats()
    // What this visit lists is judged against what earlier visits showed; what it shows is seen from the next one on.
    const [seenFinished] = useState(readSeenFinished)
    const [showRuns, setShowRuns] = useState(false)
    useEffect(() => { if (loadedAttention) markAttentionSeen(loadedAttention) }, [loadedAttention])
    useEffect(() => markFinishedSeen(runs, missions), [runs, missions])
    // eslint-disable-next-line react-hooks/purity -- render-time clock for relative dates; the view re-renders on run and mission updates
    const now = Date.now()

    const last = lastChat(chats, runs, missions)
    const counts = last ? tally(last.started) : {}
    const finished = finishedSince(runs, missions, seenFinished, now)
    const failures = finished.filter((item) => item.mark === 'failed').length
    const project = (path: string | null | undefined) => <span className="shrink-0 truncate text-xs text-muted-foreground">{projectLabel(registry, path)}</span>
    const sourceLink = (source: Source | null) => {
        if (!source) return null
        return source.kind === 'chat'
            ? <><span className="shrink-0 whitespace-pre"> · from </span><Link testId="overview-source-link" onClick={() => openChat(source.chat.project_path, source.chat.conversation_id)}>{source.chat.title}</Link></>
            : <><span className="shrink-0 whitespace-pre"> · from </span><Link testId="overview-source-link" onClick={() => openMission(source.mission.id, source.mission.project_path)}>{source.mission.fields.title || 'Untitled mission'}</Link></>
    }

    return (
        <div data-testid="overview-view" className="h-full overflow-y-auto">
            <div className="max-w-3xl px-6 pb-16 pt-6 lg:px-9">
                <h1 className="text-2xl font-light tracking-tight">Pick up where you left off</h1>
                <p className="mt-1 text-sm text-muted-foreground">Across all projects</p>

                <section data-testid="overview-last-chat" className="mt-4 rounded-lg border border-border px-5 py-4">
                    <h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">You were last in</h2>
                    {last ? (
                        <>
                            <p data-testid="overview-last-chat-title" data-conversation-id={last.chat.conversation_id} className="mt-1 truncate text-lg font-light">{last.chat.title}</p>
                            <div className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted-foreground">
                                {project(last.chat.project_path)}
                                <span>{when(last.at, now)}</span>
                                {MARK_ORDER.filter((mark) => counts[mark]).map((mark) => (
                                    <span key={mark} data-testid="overview-tally" className="inline-flex items-center gap-1"><StatusMark mark={mark} />{counts[mark]}</span>
                                ))}
                            </div>
                            <div className="mt-3 flex gap-2">
                                <Button type="button" size="sm" data-testid="overview-continue" onClick={() => openChat(last.chat.project_path, last.chat.conversation_id)}>Continue</Button>
                                {last.started.length > 0 ? (
                                    <Button type="button" size="sm" variant="outline" aria-expanded={showRuns} data-testid="overview-show-runs" onClick={() => setShowRuns((value) => !value)}>
                                        {showRuns ? 'Hide its runs' : 'Show its runs'}
                                    </Button>
                                ) : null}
                            </div>
                            {showRuns ? (
                                <ul data-testid="overview-started-list" className="mt-2 space-y-0.5 text-sm">
                                    {last.started.map((item) => (
                                        <li key={`${item.kind}:${item.id}`} data-testid="overview-started-item" className="flex items-baseline gap-2">
                                            <StatusMark mark={item.mark} />
                                            <Link onClick={() => openItem(item)}>{itemTitle(item)}</Link>
                                            <span className="ml-auto shrink-0 text-xs text-muted-foreground">{item.kind === 'mission' ? 'Mission' : flowTitle(item.run.flow_name)} · {when(item.at, now)}</span>
                                        </li>
                                    ))}
                                </ul>
                            ) : null}
                        </>
                    ) : (
                        <p className="mt-1 text-sm text-muted-foreground">{loaded ? 'No chats yet.' : 'Loading chats…'}</p>
                    )}
                </section>

                <Section testId="overview-needs-you" label={`Needs you${attention.length ? ` · ${attention.length}` : ''}`}>
                    {attention.length === 0 ? (
                        <p data-testid="overview-needs-you-empty" className="mt-2 text-sm text-muted-foreground">Nothing is waiting on you.</p>
                    ) : (
                        <ul className="mt-1">
                            {attention.map((item) => (
                                <li key={`${item.kind}:${item.id}`} className="border-b border-border">
                                    <Button
                                        type="button"
                                        variant="ghost"
                                        data-testid="overview-needs-you-item"
                                        onClick={() => openAttentionItem(item)}
                                        className="grid h-auto w-full grid-cols-[minmax(0,1fr)_auto] items-baseline gap-3 rounded-none px-0 py-2 text-left hover:bg-transparent hover:text-primary"
                                    >
                                        <span className="min-w-0">
                                            <span className="block text-xs uppercase tracking-wide text-warning">{ATTENTION_KIND_LABELS[item.kind]}</span>
                                            <span className="block truncate text-sm">{item.title || item.run_id || item.id}</span>
                                        </span>
                                        {project(item.project_path)}
                                    </Button>
                                </li>
                            ))}
                        </ul>
                    )}
                </Section>

                <Section
                    testId="overview-finished"
                    label={<>Finished {seenFinished === null ? 'in the last day' : 'since you last looked'} · {finished.length}{failures ? <span data-testid="overview-failure-count" className="text-destructive"> · {failures} failed</span> : null}</>}
                >
                    {finished.length === 0 ? (
                        <p className="mt-2 text-sm text-muted-foreground">Nothing has finished since.</p>
                    ) : (
                        <ul className="mt-1">
                            {finished.map((item) => (
                                <li key={`${item.kind}:${item.id}`} data-testid="overview-finished-item" data-item-id={item.id} className="grid grid-cols-[0.75rem_minmax(0,1fr)_auto_auto] items-baseline gap-3 border-b border-border py-2">
                                    <StatusMark mark={item.mark} />
                                    <span className="min-w-0">
                                        <Link onClick={() => openItem(item)}>{itemTitle(item)}</Link>
                                        <span className="flex min-w-0 items-baseline text-xs text-muted-foreground">
                                            <span className="shrink-0">{item.kind === 'run' ? flowTitle(item.run.flow_name) : 'Mission'}</span>
                                            {sourceLink(sourceOf(item, chats, missions))}
                                        </span>
                                    </span>
                                    {project(item.kind === 'run' ? item.run.project_path : item.mission.project_path)}
                                    <span className="text-right text-xs text-muted-foreground">{when(item.at, now)}</span>
                                </li>
                            ))}
                        </ul>
                    )}
                </Section>

                <Section testId="overview-start" label="Start something">
                    <div className="mt-2 flex flex-wrap gap-2">
                        <ProjectPicker
                            heading="New chat in"
                            testId="overview-new-chat-picker"
                            onPick={(projectPath) => {
                                useStore.getState().setViewMode('home')
                                window.dispatchEvent(new CustomEvent('spark:new-chat', { detail: projectPath }))
                            }}
                        >
                            <Button type="button" size="sm" data-testid="overview-new-chat">+ New chat</Button>
                        </ProjectPicker>
                        <ProjectPicker
                            heading="New mission in"
                            testId="overview-new-mission-picker"
                            defaultProjectPath={defaultProjectChoice(registry)}
                            onPick={(projectPath) => {
                                useStore.getState().setViewMode('missions')
                                window.dispatchEvent(new CustomEvent('spark:new-mission', { detail: projectPath }))
                            }}
                        >
                            <Button type="button" size="sm" variant="outline" data-testid="overview-new-mission">+ New mission</Button>
                        </ProjectPicker>
                        <Button type="button" size="sm" variant="outline" data-testid="overview-run-flow" onClick={() => useStore.getState().setViewMode('editor')}>Run a flow…</Button>
                    </div>
                </Section>
            </div>
        </div>
    )
}
