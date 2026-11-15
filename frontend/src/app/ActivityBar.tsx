import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react"
import { Bell, GitBranch, House, MessageSquare, Network, Play, Settings, Target, Zap, type LucideIcon } from "lucide-react"

import { useStore, type ViewMode } from "@/store"
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import type { AttentionItem } from "@/lib/api/attentionApi"
import { useOverviewUnread } from '@/features/overview/hooks/useOverviewUnread'
import { ATTENTION_KIND_LABELS, attentionFlow, attentionTimes, attentionTitle, openAttentionItem, openTrigger, refreshAttention, useAttentionFailed, useAttentionItems } from './useAttention'
import { projectLabel } from '@/features/projects/model/projectChoices'
import { useChatRunning } from '@/features/projects/hooks/useChatRunning'
import { cn } from '@/lib/utils'
import { triggerProblem } from '@/features/triggers/model/triggerForm'
import { formatRunDate } from '@/features/runs/model/runOverviewModel'
import { triggerKey, type TriggerResponse } from '@/lib/api/triggersApi'

type ActivityItem = { mode: ViewMode; label: string; icon: LucideIcon; testId: string }

export const ACTIVITY_ITEMS: ActivityItem[] = [
    { mode: 'overview', label: 'Overview', icon: House, testId: 'activity-overview' },
    { mode: 'home', label: 'Chats', icon: MessageSquare, testId: 'activity-chats' },
    { mode: 'missions', label: 'Missions', icon: Target, testId: 'activity-missions' },
    { mode: 'runs', label: 'Runs', icon: Play, testId: 'activity-runs' },
    { mode: 'triggers', label: 'Triggers', icon: Zap, testId: 'activity-triggers' },
    { mode: 'editor', label: 'Flows', icon: GitBranch, testId: 'activity-flows' },
    { mode: 'roles', label: 'Roles', icon: Network, testId: 'activity-roles' },
]
const ACTIVITY_ORDER: ViewMode[] = [...ACTIVITY_ITEMS.map((item) => item.mode), 'settings']

const RUNNING_RUN_STATUSES = new Set(['running', 'queued', 'pause_requested', 'abort_requested', 'cancel_requested'])

type ActivityDot = 'waiting' | 'running' | 'unread' | 'problem' | null
const DOT_LABELS = { waiting: 'waiting on you', running: 'running', unread: 'new since you last looked', problem: 'something is failing' }

// An icon with its name under it; on a phone-width top row there is only room for the icon,
// and the icons share the row's width so all of them stay on screen, down to 320px wide.
const ITEM_CLASS = 'relative flex h-12 w-14 shrink-0 flex-col items-center justify-center gap-1 rounded-lg border border-transparent text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring max-sm:h-11 max-sm:w-auto max-sm:min-w-6 max-sm:flex-1 max-sm:shrink'
const LABEL_CLASS = 'text-xs leading-none max-sm:hidden'

/** Which views have something waiting on you or running, for the dots on their icons. */
export function activityDots(
    attention: AttentionItem[],
    runStatuses: string[],
    missionStatuses: string[],
    chatRunning = false,
): Partial<Record<ViewMode, ActivityDot>> {
    const waiting = (kinds: AttentionItem['kind'][]) => attention.some((item) => kinds.includes(item.kind))
    return {
        home: waiting(['chat_question', 'flow_run_request', 'proposed_plan']) ? 'waiting' : chatRunning ? 'running' : null,
        missions: waiting(['mission']) || missionStatuses.includes('needs_you')
            ? 'waiting'
            : missionStatuses.includes('running') ? 'running' : null,
        runs: waiting(['run_gate']) || runStatuses.includes('waiting')
            ? 'waiting'
            : runStatuses.some((status) => RUNNING_RUN_STATUSES.has(status)) ? 'running' : null,
    }
}

type FailingTrigger = { trigger: TriggerResponse; problem: { label: string; detail: string } }

/**
 * `items` is null until the first check answers; `failed` when the latest check failed. Failing
 * triggers wait on you too, as Overview's Needs you lists them.
 */
function AttentionBell({ items: loadedItems, failed, narrow, triggers }: { items: AttentionItem[] | null; failed: boolean; narrow: boolean; triggers: FailingTrigger[] }) {
    const items = loadedItems ?? []
    const count = items.length + triggers.length
    const registry = useStore((state) => state.projectRegistry)
    const runs = useStore((state) => state.runsListSession.runs)
    const [open, setOpen] = useState(false)
    const containerRef = useRef<HTMLDivElement | null>(null)
    const buttonRef = useRef<HTMLButtonElement | null>(null)
    const listId = useId()

    useEffect(() => {
        if (!open) {
            return
        }
        const onPointerDown = (event: PointerEvent) => {
            if (!containerRef.current?.contains(event.target as Node)) {
                setOpen(false)
            }
        }
        // Escape closes the list from anywhere, and returns focus to the bell if it was inside.
        const onKeyDown = (event: globalThis.KeyboardEvent) => {
            if (event.key !== 'Escape' || event.defaultPrevented) return
            if (containerRef.current?.contains(document.activeElement)) buttonRef.current?.focus()
            setOpen(false)
        }
        window.addEventListener('pointerdown', onPointerDown)
        window.addEventListener('keydown', onKeyDown)
        return () => {
            window.removeEventListener('pointerdown', onPointerDown)
            window.removeEventListener('keydown', onKeyDown)
        }
    }, [open])

    const openItem = (item: AttentionItem) => {
        setOpen(false)
        openAttentionItem(item)
    }
    // eslint-disable-next-line react-hooks/purity -- render-time clock for when each item asked; the list re-renders as it opens
    const now = Date.now()
    const timeOf = attentionTimes(items, runs, now)

    return (
        // Narrow, the list hangs from the top row's right edge (the sticky nav), not from the bell, so it stays on screen.
        <div ref={containerRef} className={cn('max-sm:flex max-sm:min-w-6 max-sm:flex-1', !narrow && 'relative')}>
            <button
                ref={buttonRef}
                type="button"
                data-testid="attention-bell"
                aria-expanded={open}
                aria-controls={open ? listId : undefined}
                aria-label={count > 0
                    ? `Inbox: ${count} ${count === 1 ? 'item' : 'items'} waiting on you`
                    : failed ? 'Inbox: could not check what is waiting on you' : loadedItems ? 'Inbox: nothing waiting on you' : 'Inbox'}
                title="Waiting on you"
                onClick={() => setOpen((previous) => !previous)}
                className={ITEM_CLASS}
            >
                <span className="relative">
                    <Bell className="size-4.5" />
                    {count > 0 ? (
                        <span
                            data-testid="attention-bell-count"
                            className="absolute -right-2.5 -top-2 flex h-4 min-w-4 items-center justify-center rounded-full bg-warning px-1 text-xs font-medium text-warning-foreground"
                        >
                            {count}
                        </span>
                    ) : failed ? (
                        <span
                            data-testid="attention-bell-failed"
                            aria-hidden="true"
                            className="absolute -right-2.5 -top-2 flex h-4 min-w-4 items-center justify-center rounded-full bg-destructive px-1 text-xs font-medium text-destructive-foreground"
                        >
                            !
                        </span>
                    ) : null}
                </span>
                <span className={LABEL_CLASS}>Inbox</span>
            </button>
            {open && (
                <div
                    id={listId}
                    data-testid="attention-bell-list"
                    className={cn(
                        // A long list scrolls inside the viewport instead of running off it.
                        'absolute z-50 w-96 max-w-[calc(100vw-1rem)] overflow-y-auto overscroll-contain rounded-lg border bg-background p-1 shadow-md',
                        narrow ? 'right-2 top-full mt-1 max-h-[calc(100dvh-5rem)]' : 'bottom-0 left-full ml-2 max-h-[calc(100dvh-6rem)]',
                    )}
                >
                    {failed ? (
                        <div data-testid="attention-bell-error" role="alert" className="flex items-center justify-between gap-2 px-2 py-1.5 text-xs text-destructive">
                            {items.length > 0 ? 'Couldn’t refresh; this list may be out of date.' : 'Couldn’t check what is waiting on you.'}
                            <button type="button" onClick={refreshAttention} className="shrink-0 font-medium underline underline-offset-4">Try again</button>
                        </div>
                    ) : null}
                    {count === 0 && !failed ? (
                        <div className="px-2 py-1.5 text-xs text-muted-foreground">
                            {loadedItems ? 'Nothing is waiting on you.' : 'Checking…'}
                        </div>
                    ) : null}
                    {items.map((item) => (
                        <button
                            key={`${item.kind}:${item.id}`}
                            type="button"
                            data-testid="attention-bell-item"
                            onClick={() => openItem(item)}
                            className="flex w-full flex-col items-start gap-0.5 rounded-sm px-2 py-1.5 text-left hover:bg-muted"
                        >
                            <span className="flex w-full items-baseline justify-between gap-2 text-xs">
                                <span className="font-semibold text-warning">{ATTENTION_KIND_LABELS[item.kind]}</span>
                                <span data-testid="attention-bell-item-time" className="shrink-0 text-muted-foreground">{timeOf(item)}</span>
                            </span>
                            <span className="w-full truncate text-sm">
                                {attentionTitle(item, runs)}
                            </span>
                            <span className="w-full truncate text-xs text-muted-foreground">
                                {[item.question, attentionFlow(item, runs), projectLabel(registry, item.project_path)].filter(Boolean).join(' · ')}
                            </span>
                        </button>
                    ))}
                    {triggers.map(({ trigger, problem }) => (
                        <button
                            key={`trigger:${triggerKey(trigger)}`}
                            type="button"
                            data-testid="attention-bell-trigger"
                            onClick={() => { setOpen(false); openTrigger(trigger.project_path, trigger.id) }}
                            className="flex w-full flex-col items-start gap-0.5 rounded-sm px-2 py-1.5 text-left hover:bg-muted"
                        >
                            <span className="flex w-full items-baseline justify-between gap-2 text-xs">
                                <span className="font-semibold text-destructive">Trigger · {problem.label}</span>
                                <span className="shrink-0 text-muted-foreground">{trigger.state.last_fired_at ? formatRunDate(trigger.state.last_fired_at, now) : ''}</span>
                            </span>
                            <span className="w-full truncate text-sm">{trigger.name}</span>
                            <span className="w-full truncate text-xs text-muted-foreground" title={problem.detail}>
                                {[problem.detail, projectLabel(registry, trigger.project_path)].join(' · ')}
                            </span>
                        </button>
                    ))}
                </div>
            )}
        </div>
    )
}

/** The column of view icons on the left; on a narrow viewport, a row across the top. */
export function ActivityBar() {
    const viewMode = useStore((state) => state.viewMode)
    const setViewMode = useStore((state) => state.setViewMode)
    const runs = useStore((state) => state.runsListSession.runs)
    const missions = useStore((state) => state.missionBoard)
    const chatRunning = useChatRunning()
    const isNarrowViewport = useNarrowViewport()
    const loadedAttention = useAttentionItems()
    const attentionFailed = useAttentionFailed()
    const attention = loadedAttention ?? []
    const overviewUnread = useOverviewUnread(attention)
    // A trigger that can't run, or whose firings fail, starts nothing until someone looks.
    const triggers = useStore((state) => state.triggersSession.triggers)
    const failingTriggers = triggers.flatMap((trigger) => {
        const problem = triggerProblem(trigger)
        return problem ? [{ trigger, problem }] : []
    })
    const triggersFailing = failingTriggers.length > 0
    const dots = {
        ...activityDots(
            attention,
            runs.map((run) => run.status),
            missions.map((mission) => mission.status ?? 'draft'),
            chatRunning,
        ),
        overview: overviewUnread ? 'unread' as const : null,
        triggers: triggersFailing ? 'problem' as const : null,
    }

    // Arrow keys only move focus along the bar; Enter, Space or a click opens the view.
    const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, mode: ViewMode) => {
        const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[event.key]
        if (!step) {
            return
        }
        event.preventDefault()
        const index = Math.max(0, ACTIVITY_ORDER.indexOf(mode))
        const next = ACTIVITY_ORDER[(index + step + ACTIVITY_ORDER.length) % ACTIVITY_ORDER.length]
        document.querySelector<HTMLButtonElement>(`[data-activity-mode="${next}"]`)?.focus()
    }

    const renderItem = ({ mode, label, icon: Icon, testId }: ActivityItem) => {
        const isActive = viewMode === mode || (mode === 'home' && viewMode === 'projects')
        const dot = dots[mode] ?? null
        return (
            <button
                key={mode}
                type="button"
                data-testid={testId}
                data-activity-mode={mode}
                aria-label={dot ? `${label} (${DOT_LABELS[dot]})` : label}
                aria-current={isActive ? 'page' : undefined}
                onClick={() => setViewMode(mode)}
                onKeyDown={(event) => onKeyDown(event, mode)}
                className={cn(ITEM_CLASS, 'transition-colors aria-[current=page]:border-border aria-[current=page]:bg-background aria-[current=page]:text-primary')}
            >
                <span className="relative">
                    <Icon className="size-4.5" />
                    {dot ? (
                        <span
                            data-testid={`${testId}-dot`}
                            data-dot={dot}
                            className={cn('absolute -right-1 -top-0.5 size-1.5 rounded-full', dot === 'waiting' ? 'bg-warning' : dot === 'problem' ? 'bg-destructive' : 'bg-primary')}
                        />
                    ) : null}
                </span>
                <span className={LABEL_CLASS}>{label}</span>
            </button>
        )
    }

    return (
        <nav
            data-testid="activity-bar"
            aria-label="Views"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
            className={cn(
                'z-50 flex shrink-0 items-center gap-1 bg-ground',
                isNarrowViewport ? 'sticky top-0 w-full flex-row px-2 py-1 max-sm:gap-0.5' : 'w-16 flex-col py-3',
            )}
        >
            <img src="/assets/spark-app-icon.png" alt="Spark" width={28} height={28} className={cn('size-7 rounded-lg', isNarrowViewport ? 'mr-1' : 'mb-3')} />
            {ACTIVITY_ITEMS.map(renderItem)}
            <span className="flex-1 max-sm:hidden" />
            <AttentionBell items={loadedAttention} failed={attentionFailed} narrow={isNarrowViewport} triggers={failingTriggers} />
            {renderItem({ mode: 'settings', label: 'Settings', icon: Settings, testId: 'activity-settings' })}
        </nav>
    )
}
