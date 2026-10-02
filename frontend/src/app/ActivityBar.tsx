import { type KeyboardEvent, useEffect, useRef, useState } from "react"
import { Bell, GitBranch, House, MessageSquare, Play, Settings, Target, Zap, type LucideIcon } from "lucide-react"

import { useStore, type ViewMode } from "@/store"
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import type { AttentionItem } from "@/lib/api/attentionApi"
import { useOverviewUnread } from '@/features/overview/hooks/useOverviewUnread'
import { ATTENTION_KIND_LABELS, openAttentionItem, useAttentionItems } from './useAttention'
import { projectLabel } from '@/features/projects/model/projectChoices'
import { useChatRunning } from '@/features/projects/hooks/useChatRunning'
import { cn } from '@/lib/utils'

type ActivityItem = { mode: ViewMode; label: string; icon: LucideIcon; testId: string }

export const ACTIVITY_ITEMS: ActivityItem[] = [
    { mode: 'overview', label: 'Overview', icon: House, testId: 'activity-overview' },
    { mode: 'home', label: 'Chats', icon: MessageSquare, testId: 'activity-chats' },
    { mode: 'missions', label: 'Missions', icon: Target, testId: 'activity-missions' },
    { mode: 'runs', label: 'Runs', icon: Play, testId: 'activity-runs' },
    { mode: 'triggers', label: 'Triggers', icon: Zap, testId: 'activity-triggers' },
    { mode: 'editor', label: 'Flows', icon: GitBranch, testId: 'activity-flows' },
]
const ACTIVITY_ORDER: ViewMode[] = [...ACTIVITY_ITEMS.map((item) => item.mode), 'settings']

const RUNNING_RUN_STATUSES = new Set(['running', 'queued', 'pause_requested', 'abort_requested', 'cancel_requested'])

type ActivityDot = 'waiting' | 'running' | 'unread' | null
const DOT_LABELS = { waiting: 'waiting on you', running: 'running', unread: 'new since you last looked' }

/** Which views have something waiting on you or running, for the dots on their icons. */
export function activityDots(
    attention: AttentionItem[],
    runStatuses: string[],
    missionStatuses: string[],
    chatRunning = false,
): Partial<Record<ViewMode, ActivityDot>> {
    const waiting = (kinds: AttentionItem['kind'][]) => attention.some((item) => kinds.includes(item.kind))
    return {
        home: waiting(['flow_run_request', 'proposed_plan']) ? 'waiting' : chatRunning ? 'running' : null,
        missions: waiting(['mission']) || missionStatuses.includes('needs_you')
            ? 'waiting'
            : missionStatuses.includes('running') ? 'running' : null,
        runs: waiting(['run_gate']) || runStatuses.includes('waiting')
            ? 'waiting'
            : runStatuses.some((status) => RUNNING_RUN_STATUSES.has(status)) ? 'running' : null,
    }
}

function AttentionBell({ items, narrow }: { items: AttentionItem[]; narrow: boolean }) {
    const registry = useStore((state) => state.projectRegistry)
    const [open, setOpen] = useState(false)
    const containerRef = useRef<HTMLDivElement | null>(null)

    useEffect(() => {
        if (!open) {
            return
        }
        const onPointerDown = (event: PointerEvent) => {
            if (!containerRef.current?.contains(event.target as Node)) {
                setOpen(false)
            }
        }
        window.addEventListener('pointerdown', onPointerDown)
        return () => window.removeEventListener('pointerdown', onPointerDown)
    }, [open])

    const openItem = (item: AttentionItem) => {
        setOpen(false)
        openAttentionItem(item)
    }

    return (
        <div ref={containerRef} className="relative">
            <button
                type="button"
                data-testid="attention-bell"
                aria-label={items.length > 0 ? `${items.length} items waiting on you` : 'Nothing waiting on you'}
                title="Waiting on you"
                onClick={() => setOpen((previous) => !previous)}
                className="relative flex size-9 items-center justify-center rounded-md text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
            >
                <Bell className="size-4" />
                {items.length > 0 && (
                    <span
                        data-testid="attention-bell-count"
                        className="absolute right-0.5 top-0.5 flex h-4 min-w-4 items-center justify-center rounded-full bg-warning px-1 text-xs font-medium text-warning-foreground"
                    >
                        {items.length}
                    </span>
                )}
            </button>
            {open && (
                <div
                    data-testid="attention-bell-list"
                    className={cn(
                        'absolute z-50 w-96 max-w-[calc(100vw-1rem)] rounded-md border bg-background p-1 shadow-md',
                        narrow ? 'right-0 top-full mt-1' : 'bottom-0 left-full ml-2',
                    )}
                >
                    {items.length === 0 && (
                        <div className="px-2 py-1.5 text-xs text-muted-foreground">
                            Nothing is waiting on you.
                        </div>
                    )}
                    {items.map((item) => (
                        <button
                            key={`${item.kind}:${item.id}`}
                            type="button"
                            data-testid="attention-bell-item"
                            onClick={() => openItem(item)}
                            className="flex w-full flex-col items-start gap-0.5 rounded-sm px-2 py-1.5 text-left hover:bg-muted"
                        >
                            <span className="text-xs font-semibold uppercase tracking-wide text-warning">
                                {ATTENTION_KIND_LABELS[item.kind]}
                            </span>
                            <span className="w-full truncate text-sm">
                                {item.title || item.run_id || item.id}
                            </span>
                            <span className="w-full truncate text-xs text-muted-foreground">
                                {projectLabel(registry, item.project_path)}
                                {item.conversation_handle ? ` · ${item.conversation_handle}` : ''}
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
    const attention = useAttentionItems() ?? []
    const overviewUnread = useOverviewUnread(attention)
    const dots = {
        ...activityDots(
            attention,
            runs.map((run) => run.status),
            missions.map((mission) => mission.status ?? 'draft'),
            chatRunning,
        ),
        overview: overviewUnread ? 'unread' as const : null,
    }

    const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, mode: ViewMode) => {
        const step = { ArrowDown: 1, ArrowRight: 1, ArrowUp: -1, ArrowLeft: -1 }[event.key]
        if (!step) {
            return
        }
        event.preventDefault()
        const index = Math.max(0, ACTIVITY_ORDER.indexOf(mode))
        const next = ACTIVITY_ORDER[(index + step + ACTIVITY_ORDER.length) % ACTIVITY_ORDER.length]
        setViewMode(next)
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
                title={label}
                onClick={() => setViewMode(mode)}
                onKeyDown={(event) => onKeyDown(event, mode)}
                className="relative flex size-9 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring aria-[current=page]:bg-accent aria-[current=page]:text-primary"
            >
                <Icon className="size-4" />
                {dot ? (
                    <span
                        data-testid={`${testId}-dot`}
                        data-dot={dot}
                        className={cn('absolute right-1.5 top-1.5 size-1.5 rounded-full', dot === 'waiting' ? 'bg-warning' : 'bg-primary')}
                    />
                ) : null}
            </button>
        )
    }

    return (
        <nav
            data-testid="activity-bar"
            aria-label="Views"
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'inline'}
            className={cn(
                'z-50 flex shrink-0 items-center gap-1 border-border bg-background',
                isNarrowViewport ? 'sticky top-0 w-full flex-row border-b px-2 py-1' : 'w-12 flex-col border-r py-2.5',
            )}
        >
            <img src="/assets/spark-app-icon.png" alt="Spark" width={20} height={20} className={cn('size-5', isNarrowViewport ? 'mr-1' : 'mb-2.5')} />
            {ACTIVITY_ITEMS.map(renderItem)}
            <span className="flex-1" />
            <AttentionBell items={attention} narrow={isNarrowViewport} />
            {renderItem({ mode: 'settings', label: 'Settings', icon: Settings, testId: 'activity-settings' })}
        </nav>
    )
}
