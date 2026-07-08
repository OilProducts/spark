import { useState } from "react"
import type { KeyboardEventHandler, MutableRefObject, PointerEventHandler } from "react"
import { FileText, Plus, Trash2 } from "lucide-react"

import { buildRunsHash } from "@/app/runsRouting"
import { useStore } from "@/store"
import { formatProjectPathLabel } from "@/lib/projectPaths"
import { cn } from "@/lib/utils"

import { HomeProjectSidebar } from "./HomeProjectSidebar"
import type { ProjectConversationSummary } from "../model/types"
import { Alert, AlertDescription } from "@/components/ui/alert"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from "@/components/ui/empty"
type ProjectsSidebarProps = {
    isNarrowViewport: boolean
    homeSidebarRef: MutableRefObject<HTMLDivElement | null>
    homeSidebarPrimaryHeight: number
    activeProjectPath: string | null
    activeConversationId: string | null
    activeProjectLabel: string | null
    activeProjectConversationSummaries: ProjectConversationSummary[]
    activeProjectConversationSummariesStatus: 'idle' | 'loading' | 'ready' | 'error'
    pendingDeleteConversationId: string | null
    isHomeSidebarResizing: boolean
    onCreateConversationThread: () => void
    onSelectConversationThread: (conversationId: string) => void
    onDeleteConversationThread: (conversationId: string, title: string) => void | Promise<void>
    onHomeSidebarResizePointerDown: PointerEventHandler<HTMLDivElement>
    onHomeSidebarResizeKeyDown: KeyboardEventHandler<HTMLDivElement>
    formatConversationAgeShort: (value: string) => string
    formatConversationTimestamp: (value: string) => string
}

export function ProjectsSidebar({
    isNarrowViewport,
    homeSidebarRef,
    homeSidebarPrimaryHeight,
    activeProjectPath,
    activeConversationId,
    activeProjectLabel,
    activeProjectConversationSummaries,
    activeProjectConversationSummariesStatus,
    pendingDeleteConversationId,
    isHomeSidebarResizing,
    onCreateConversationThread,
    onSelectConversationThread,
    onDeleteConversationThread,
    onHomeSidebarResizePointerDown,
    onHomeSidebarResizeKeyDown,
    formatConversationAgeShort,
    formatConversationTimestamp,
}: ProjectsSidebarProps) {
    const workflowEventLog = useStore((state) => state.workflowEventLog)
    const [logScope, setLogScope] = useState<'all' | 'active'>('all')
    const scopedWorkflowEntries = logScope === 'active' && activeProjectPath
        ? workflowEventLog.filter((entry) => entry.project_path === activeProjectPath)
        : workflowEventLog

    return (
        <HomeProjectSidebar className={isNarrowViewport ? "gap-4" : "h-full"}>
            <div
                ref={homeSidebarRef}
                data-testid="home-sidebar-stack"
                className={`flex ${isNarrowViewport ? "flex-col gap-4" : "h-full min-h-0 flex-col"}`}
            >
                <div
                    data-testid="home-sidebar-primary-surface"
                    className={isNarrowViewport ? "" : "min-h-0 overflow-hidden"}
                    style={isNarrowViewport ? undefined : { height: `${homeSidebarPrimaryHeight}px` }}
                >
                    <Card className="h-full gap-4 rounded-md border border-border py-0">
                        <CardHeader className="gap-1 border-b border-border/60 px-4 py-4">
                            <div className="flex items-start justify-between gap-3">
                                <div className="min-w-0 space-y-1">
                                    <h3 className="text-sm font-semibold text-foreground">Threads</h3>
                                    <p className="text-xs leading-5 text-muted-foreground">
                                        {activeProjectPath
                                            ? `Threads for ${activeProjectLabel || 'the active project'}.`
                                            : 'Choose or add a project from the navbar to view threads.'}
                                    </p>
                                </div>
                                {activeProjectPath ? (
                                    <Button
                                        data-testid="project-thread-new-button"
                                        type="button"
                                        onClick={onCreateConversationThread}
                                        variant="outline"
                                        size="xs"
                                    >
                                        <Plus className="h-3.5 w-3.5" />
                                        New thread
                                    </Button>
                                ) : null}
                            </div>
                        </CardHeader>
                    <CardContent className={`px-4 pt-4 ${isNarrowViewport ? "" : "min-h-0 flex-1 overflow-x-hidden overflow-y-auto pr-1"}`}>
                        <div className={isNarrowViewport ? "" : "min-h-0 flex-1 overflow-x-hidden overflow-y-auto pr-1"}>
                            <ul data-testid="project-thread-list" className="space-y-1.5">
                                {!activeProjectPath ? (
                                    <li>
                                        <Empty className="px-3 py-4 text-xs text-muted-foreground">
                                            <EmptyHeader>
                                                <EmptyDescription>
                                                    Choose or add a project from the navbar to view threads.
                                                </EmptyDescription>
                                            </EmptyHeader>
                                        </Empty>
                                    </li>
                                ) : activeProjectConversationSummariesStatus === 'idle' || activeProjectConversationSummariesStatus === 'loading' ? (
                                    <li>
                                        <Alert
                                            data-testid="project-thread-list-loading"
                                            className="border-border/70 bg-muted/20 px-3 py-2 text-xs text-muted-foreground"
                                        >
                                            <AlertDescription className="text-inherit">
                                                Restoring thread list…
                                            </AlertDescription>
                                        </Alert>
                                    </li>
                                ) : activeProjectConversationSummariesStatus === 'error' && activeProjectConversationSummaries.length === 0 ? (
                                    <li>
                                        <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                                            <AlertDescription className="text-inherit">
                                                Unable to restore the thread list.
                                            </AlertDescription>
                                        </Alert>
                                    </li>
                                ) : activeProjectConversationSummaries.length === 0 ? (
                                    <li>
                                        <Empty className="px-3 py-4 text-xs text-muted-foreground">
                                            <EmptyHeader>
                                                <EmptyDescription>No threads for this project yet.</EmptyDescription>
                                            </EmptyHeader>
                                        </Empty>
                                    </li>
                                ) : (
                                    activeProjectConversationSummaries.map((conversation) => {
                                        const isActiveConversation = conversation.conversation_id === activeConversationId
                                        const ageLabel = formatConversationAgeShort(conversation.updated_at)
                                        const isDeletingConversation = pendingDeleteConversationId === conversation.conversation_id
                                        return (
                                            <li key={conversation.conversation_id} className="group/thread relative">
                                                <Button
                                                    type="button"
                                                    onClick={() => onSelectConversationThread(conversation.conversation_id)}
                                                    aria-current={isActiveConversation ? "true" : undefined}
                                                    aria-label={`Open thread ${conversation.title}`}
                                                    variant={isActiveConversation ? "secondary" : "ghost"}
                                                    size="sm"
                                                    className={`h-auto w-full min-w-0 justify-start overflow-hidden rounded-xl px-2 py-2 pr-9 text-left ${isActiveConversation
                                                        ? "bg-muted text-foreground shadow-sm"
                                                        : "text-foreground/90 hover:bg-muted/60"
                                                        }`}
                                                >
                                                    <div className="flex w-full min-w-0 items-center gap-2">
                                                        <FileText className={`h-3.5 w-3.5 shrink-0 ${isActiveConversation ? "text-foreground" : "text-muted-foreground"}`} />
                                                        <div className="min-w-0 flex-1">
                                                            <span className="block truncate text-[13px] font-medium">
                                                                {conversation.title}
                                                            </span>
                                                            {conversation.conversation_handle ? (
                                                                <span className="block truncate font-mono text-[10px] text-muted-foreground">
                                                                    {conversation.conversation_handle}
                                                                </span>
                                                            ) : null}
                                                        </div>
                                                        <span className="ml-auto shrink-0 text-[11px] text-muted-foreground transition-opacity group-hover/thread:opacity-0 group-focus-within/thread:opacity-0">
                                                            {ageLabel}
                                                        </span>
                                                    </div>
                                                </Button>
                                                <Button
                                                    type="button"
                                                    aria-label={`Delete thread ${conversation.title}`}
                                                    data-testid={`project-thread-delete-${conversation.conversation_id}`}
                                                    onClick={() => {
                                                        void onDeleteConversationThread(conversation.conversation_id, conversation.title)
                                                    }}
                                                    disabled={isDeletingConversation}
                                                    variant="ghost"
                                                    size="icon-xs"
                                                    className="absolute right-1 top-1/2 -translate-y-1/2 text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-destructive focus-visible:opacity-100 group-hover/thread:opacity-100 group-focus-within/thread:opacity-100"
                                                >
                                                    <Trash2 className="h-3.5 w-3.5" />
                                                </Button>
                                            </li>
                                        )
                                    })
                                )}
                            </ul>
                        </div>
                    </CardContent>
                    </Card>
                </div>
                {!isNarrowViewport ? (
                    <div
                        data-testid="home-sidebar-resize-handle"
                        role="separator"
                        aria-label="Resize sidebar sections"
                        aria-orientation="horizontal"
                        tabIndex={0}
                        onPointerDown={onHomeSidebarResizePointerDown}
                        onKeyDown={onHomeSidebarResizeKeyDown}
                        className={`group flex h-3 shrink-0 cursor-row-resize items-center justify-center rounded-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isHomeSidebarResizing ? "bg-muted" : "hover:bg-muted/60"}`}
                    >
                        <span className="h-1 w-12 rounded-full bg-border transition-colors group-hover:bg-muted-foreground/70" />
                    </div>
                ) : null}
                <div
                    data-testid="project-event-log-surface"
                    className={`flex min-h-[280px] flex-col rounded-md border border-border bg-card p-4 shadow-sm ${isNarrowViewport ? "" : "min-h-0 flex-1 overflow-hidden"}`}
                >
                    <div className="mb-3 flex items-center justify-between gap-2">
                        <h3 className="text-sm font-semibold text-foreground">Workflow Event Log</h3>
                        <div
                            role="group"
                            aria-label="Workflow event log scope"
                            className="inline-flex overflow-hidden rounded-md border border-border"
                        >
                            <button
                                type="button"
                                data-testid="workflow-event-log-scope-all"
                                aria-pressed={logScope === 'all'}
                                onClick={() => setLogScope('all')}
                                className={cn(
                                    'px-2 py-0.5 text-[11px] font-medium transition-colors',
                                    logScope === 'all'
                                        ? 'bg-primary text-primary-foreground'
                                        : 'bg-background text-muted-foreground hover:bg-muted/60',
                                )}
                            >
                                All projects
                            </button>
                            <button
                                type="button"
                                data-testid="workflow-event-log-scope-active"
                                aria-pressed={logScope === 'active'}
                                disabled={!activeProjectPath}
                                onClick={() => setLogScope('active')}
                                className={cn(
                                    'px-2 py-0.5 text-[11px] font-medium transition-colors disabled:opacity-50',
                                    logScope === 'active'
                                        ? 'bg-primary text-primary-foreground'
                                        : 'bg-background text-muted-foreground hover:bg-muted/60',
                                )}
                            >
                                This project
                            </button>
                        </div>
                    </div>
                    {scopedWorkflowEntries.length === 0 ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                            {logScope === 'active'
                                ? 'No workflow events recorded for this project yet.'
                                : 'No workflow events recorded yet.'}
                        </p>
                    ) : (
                        <ol data-testid="project-event-log-list" className="flex-1 space-y-2 overflow-y-auto pr-1">
                            {[...scopedWorkflowEntries].reverse().map((entry) => (
                                <li key={entry.id}>
                                    <a
                                        data-testid="workflow-event-log-row"
                                        data-kind={entry.kind}
                                        href={buildRunsHash(entry.run_id, entry.node_id ?? null)}
                                        className={cn(
                                            'block rounded border border-border border-l-2 px-2 py-1.5 transition-colors hover:bg-muted/40',
                                            entry.kind === 'run_failed' && 'border-l-destructive/70',
                                            entry.kind === 'run_waiting_on_input' && 'border-l-sky-500/70',
                                            entry.kind === 'run_completed' && 'border-l-green-500/70',
                                            entry.kind === 'run_canceled' && 'border-l-amber-500/70',
                                        )}
                                    >
                                        <p className="flex items-center justify-between gap-2 text-[10px] text-muted-foreground">
                                            <span>{formatConversationTimestamp(entry.timestamp)}</span>
                                            <span
                                                data-testid="workflow-event-log-row-project"
                                                className="truncate"
                                                title={entry.project_path}
                                            >
                                                {formatProjectPathLabel(entry.project_path)}
                                            </span>
                                        </p>
                                        <p className="text-xs text-foreground">{entry.message}</p>
                                    </a>
                                </li>
                            ))}
                        </ol>
                    )}
                </div>
            </div>
        </HomeProjectSidebar>
    )
}
