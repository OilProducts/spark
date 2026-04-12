import type { KeyboardEventHandler, MutableRefObject, PointerEventHandler } from "react"
import { FileText, Plus, Trash2 } from "lucide-react"

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
type ProjectEventLogEntry = {
    message: string
    timestamp: string
}

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
    activeProjectEventLog: ProjectEventLogEntry[]
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
    activeProjectEventLog,
    isHomeSidebarResizing,
    onCreateConversationThread,
    onSelectConversationThread,
    onDeleteConversationThread,
    onHomeSidebarResizePointerDown,
    onHomeSidebarResizeKeyDown,
    formatConversationAgeShort,
    formatConversationTimestamp,
}: ProjectsSidebarProps) {
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
                    <CardContent className={`px-4 pt-4 ${isNarrowViewport ? "" : "min-h-0 flex-1 overflow-y-auto pr-1"}`}>
                        <div className={isNarrowViewport ? "" : "min-h-0 flex-1 overflow-y-auto pr-1"}>
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
                                                    className={`h-auto w-full justify-start rounded-xl px-2 py-2 pr-9 text-left ${isActiveConversation
                                                        ? "bg-muted text-foreground shadow-sm"
                                                        : "text-foreground/90 hover:bg-muted/60"
                                                        }`}
                                                >
                                                    <div className="flex items-center gap-2">
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
                                                        <span className="shrink-0 text-[11px] text-muted-foreground transition-opacity group-hover/thread:opacity-0 group-focus-within/thread:opacity-0">
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
                    <div className="mb-3">
                        <h3 className="text-sm font-semibold text-foreground">Workflow Event Log</h3>
                    </div>
                    {!activeProjectPath ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                            Choose or add a project from the navbar to view workflow events.
                        </p>
                    ) : activeProjectEventLog.length === 0 ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                            No workflow events recorded for this project yet.
                        </p>
                    ) : (
                        <ol data-testid="project-event-log-list" className="flex-1 space-y-2 overflow-y-auto pr-1">
                            {[...activeProjectEventLog].reverse().map((entry, index) => (
                                <li key={`${entry.timestamp}-${index}`} className="rounded border border-border px-2 py-1.5">
                                    <p className="text-[10px] text-muted-foreground">{formatConversationTimestamp(entry.timestamp)}</p>
                                    <p className="text-xs text-foreground">{entry.message}</p>
                                </li>
                            ))}
                        </ol>
                    )}
                </div>
            </div>
        </HomeProjectSidebar>
    )
}
