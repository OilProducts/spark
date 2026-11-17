import { type FocusEvent, type KeyboardEvent, useEffect, useId, useRef, useState } from "react"
import { Bell, Ellipsis, GitBranch, House, MessageSquare, Network, Play, Settings, Target, Zap, type LucideIcon } from "lucide-react"

import { useStore, type ViewMode } from "@/store"
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { revealList } from '@/components/app/view-layout'
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
const SETTINGS_ITEM: ActivityItem = { mode: 'settings', label: 'Settings', icon: Settings, testId: 'activity-settings' }
// A phone's row has room for five views with their names; these open, named too, from More.
const PHONE_MORE_MODES = new Set<ViewMode>(['triggers', 'editor', 'roles', 'settings'])
const PHONE_MORE_ITEMS = [...ACTIVITY_ITEMS, SETTINGS_ITEM].filter((item) => PHONE_MORE_MODES.has(item.mode))

const RUNNING_RUN_STATUSES = new Set(['running', 'queued', 'pause_requested', 'abort_requested', 'cancel_requested'])

type ActivityDot = 'waiting' | 'running' | 'unread' | 'problem' | null
const DOT_LABELS = { waiting: 'waiting on you', running: 'running', unread: 'new since you last looked', problem: 'something is failing' }

// An icon with its name under it. On a phone-width top row the items share the row's width, so
// all of them stay on screen with their names, down to 320px wide.
const ITEM_CLASS = 'relative flex h-12 w-14 shrink-0 flex-col items-center justify-center gap-1 rounded-lg border border-transparent text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring max-sm:h-11 max-sm:w-auto max-sm:min-w-6 max-sm:flex-1 max-sm:shrink'
const LABEL_CLASS = 'text-xs leading-none max-sm:tracking-tight'
const CURRENT_CLASS = 'transition-colors aria-[current=page]:border-border aria-[current=page]:bg-background aria-[current=page]:text-primary'
const DOT_CLASS = (dot: NonNullable<ActivityDot>) => cn('absolute -right-1 -top-0.5 size-1.5 rounded-full', dot === 'waiting' ? 'bg-warning' : dot === 'problem' ? 'bg-destructive' : 'bg-primary')

/** Closes a list when focus moves on to something outside it, as tabbing past its last item does. */
const closeOnFocusLeaving = (close: () => void) => (event: FocusEvent<HTMLElement>) => {
    if (event.relatedTarget && !event.currentTarget.contains(event.relatedTarget as Node)) close()
}

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
        <div ref={containerRef} onBlur={closeOnFocusLeaving(() => setOpen(false))} className={cn('max-sm:flex max-sm:min-w-6 max-sm:flex-1', !narrow && 'relative')}>
            <button
                ref={buttonRef}
                type="button"
                data-testid="attention-bell"
                data-bar-item
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

/** On a phone, the views the row has no room for, by name; marked current while one of them is open. */
function PhoneMoreViews({ viewMode, dots, onOpen }: { viewMode: ViewMode; dots: Partial<Record<ViewMode, ActivityDot>>; onOpen: (mode: ViewMode) => void }) {
    const [open, setOpen] = useState(false)
    const containerRef = useRef<HTMLDivElement | null>(null)
    const buttonRef = useRef<HTMLButtonElement | null>(null)
    const listId = useId()
    useEffect(() => {
        if (!open) return
        const onPointerDown = (event: PointerEvent) => {
            if (!containerRef.current?.contains(event.target as Node)) setOpen(false)
        }
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
    const shown = PHONE_MORE_ITEMS.find((item) => item.mode === viewMode) ?? null
    const dot = PHONE_MORE_ITEMS.map((item) => dots[item.mode]).find(Boolean) ?? null
    return (
        <div ref={containerRef} onBlur={closeOnFocusLeaving(() => setOpen(false))} className="flex min-w-6 flex-1 sm:hidden">
            <button
                ref={buttonRef}
                type="button"
                data-testid="activity-more"
                data-bar-item
                aria-expanded={open}
                aria-controls={open ? listId : undefined}
                aria-current={shown ? 'page' : undefined}
                aria-label={['More views', shown ? `(${shown.label} open)` : null, dot ? `(${DOT_LABELS[dot]})` : null].filter(Boolean).join(' ')}
                onClick={() => setOpen((previous) => !previous)}
                className={cn(ITEM_CLASS, CURRENT_CLASS)}
            >
                <span className="relative">
                    <Ellipsis className="size-4.5" />
                    {dot ? <span data-testid="activity-more-dot" data-dot={dot} className={DOT_CLASS(dot)} /> : null}
                </span>
                <span className={LABEL_CLASS}>More</span>
            </button>
            {open ? (
                <div id={listId} data-testid="activity-more-list" className="absolute right-2 top-full z-50 mt-1 flex w-48 flex-col rounded-lg border bg-background p-1 shadow-md">
                    {PHONE_MORE_ITEMS.map(({ mode, label, icon: ItemIcon, testId }) => {
                        const itemDot = dots[mode] ?? null
                        return (
                            <button
                                key={mode}
                                type="button"
                                data-testid={`${testId}-more`}
                                aria-current={viewMode === mode ? 'page' : undefined}
                                onClick={() => {
                                    setOpen(false)
                                    onOpen(mode)
                                }}
                                className="flex w-full items-center gap-2.5 rounded-sm px-2.5 py-2 text-left text-sm text-foreground outline-none hover:bg-muted focus-visible:ring-2 focus-visible:ring-ring aria-[current=page]:font-medium aria-[current=page]:text-primary"
                            >
                                <ItemIcon aria-hidden="true" className="size-4 text-muted-foreground" />
                                {label}
                                {itemDot ? <span className="ml-auto text-xs text-muted-foreground">{DOT_LABELS[itemDot]}</span> : null}
                            </button>
                        )
                    })}
                </div>
            ) : null}
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

    // Arrow keys only move focus along the bar's items, the Inbox and More among them; Enter, Space
    // or a click opens one. An item a phone keeps under More, or More on a wider screen, is passed over.
    const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
        const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[event.key]
        const target = event.target as HTMLElement
        if (!step || !target.hasAttribute('data-bar-item')) {
            return
        }
        event.preventDefault()
        const items = [...event.currentTarget.querySelectorAll<HTMLElement>('[data-bar-item]')]
            .filter((item) => item.checkVisibility?.() !== false)
        const index = items.indexOf(target)
        items[(index + step + items.length) % items.length]?.focus()
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
                data-bar-item
                aria-label={dot ? `${label} (${DOT_LABELS[dot]})` : label}
                aria-current={isActive ? 'page' : undefined}
                // Stacked, a second tap on the shown view goes back up to its list.
                onClick={() => {
                    setViewMode(mode)
                    if (isActive && isNarrowViewport) revealList()
                }}
                className={cn(ITEM_CLASS, CURRENT_CLASS, PHONE_MORE_MODES.has(mode) && 'max-sm:hidden')}
            >
                <span className="relative">
                    <Icon className="size-4.5" />
                    {dot ? <span data-testid={`${testId}-dot`} data-dot={dot} className={DOT_CLASS(dot)} /> : null}
                </span>
                <span className={LABEL_CLASS}>{label}</span>
            </button>
        )
    }

    return (
        <nav
            data-testid="activity-bar"
            aria-label="Views"
            onKeyDown={onKeyDown}
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
            className={cn(
                'z-50 flex shrink-0 items-center gap-1 bg-ground',
                isNarrowViewport ? 'sticky top-0 w-full flex-row px-2 py-1 max-sm:gap-0 max-sm:px-1' : 'w-16 flex-col py-3',
            )}
        >
            <img src="/assets/spark-app-icon.png" alt="Spark" width={28} height={28} className={cn('size-7 rounded-lg max-sm:hidden', isNarrowViewport ? 'mr-1' : 'mb-3')} />
            {ACTIVITY_ITEMS.map(renderItem)}
            <span className="flex-1 max-sm:hidden" />
            <AttentionBell items={loadedAttention} failed={attentionFailed} narrow={isNarrowViewport} triggers={failingTriggers} />
            {renderItem(SETTINGS_ITEM)}
            <PhoneMoreViews viewMode={viewMode} dots={dots} onOpen={(mode) => {
                setViewMode(mode)
                if (mode === viewMode) revealList()
            }} />
        </nav>
    )
}
