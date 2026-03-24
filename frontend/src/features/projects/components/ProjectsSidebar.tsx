import type { ChangeEventHandler, KeyboardEventHandler, MutableRefObject, PointerEventHandler } from "react"
import { FileText, Folder, FolderOpen, Plus, Trash2 } from "lucide-react"

import { HomeProjectSidebar } from "@/components/HomeProjectSidebar"
import type { ConversationSummaryResponse } from "@/lib/workspaceClient"
import { Button, EmptyState, Panel, PanelContent, PanelHeader, SectionHeader } from "@/ui"

type ProjectRecord = {
    directoryPath: string
}

type ProjectEventLogEntry = {
    message: string
    timestamp: string
}

type ProjectsSidebarProps = {
    isNarrowViewport: boolean
    homeSidebarRef: MutableRefObject<HTMLDivElement | null>
    homeSidebarPrimaryHeight: number
    projectDirectoryPickerInputRef: MutableRefObject<HTMLInputElement | null>
    projectRegistrationError: string | null
    orderedProjects: ProjectRecord[]
    activeProjectPath: string | null
    activeConversationId: string | null
    activeProjectConversationSummaries: ConversationSummaryResponse[]
    pendingDeleteProjectPath: string | null
    pendingDeleteConversationId: string | null
    activeProjectEventLog: ProjectEventLogEntry[]
    isHomeSidebarResizing: boolean
    onOpenProjectDirectoryChooser: () => void
    onProjectDirectorySelected: ChangeEventHandler<HTMLInputElement>
    onActivateProject: (projectPath: string) => void | Promise<void>
    onDeleteProject: (projectPath: string) => void | Promise<void>
    onCreateConversationThread: () => void
    onSelectConversationThread: (conversationId: string) => void
    onDeleteConversationThread: (conversationId: string, title: string) => void | Promise<void>
    onHomeSidebarResizePointerDown: PointerEventHandler<HTMLDivElement>
    onHomeSidebarResizeKeyDown: KeyboardEventHandler<HTMLDivElement>
    formatProjectListLabel: (projectPath: string) => string
    formatConversationAgeShort: (value: string) => string
    formatConversationTimestamp: (value: string) => string
}

export function ProjectsSidebar({
    isNarrowViewport,
    homeSidebarRef,
    homeSidebarPrimaryHeight,
    projectDirectoryPickerInputRef,
    projectRegistrationError,
    orderedProjects,
    activeProjectPath,
    activeConversationId,
    activeProjectConversationSummaries,
    pendingDeleteProjectPath,
    pendingDeleteConversationId,
    activeProjectEventLog,
    isHomeSidebarResizing,
    onOpenProjectDirectoryChooser,
    onProjectDirectorySelected,
    onActivateProject,
    onDeleteProject,
    onCreateConversationThread,
    onSelectConversationThread,
    onDeleteConversationThread,
    onHomeSidebarResizePointerDown,
    onHomeSidebarResizeKeyDown,
    formatProjectListLabel,
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
                    <Panel className="h-full rounded-md border border-border py-0 shadow-sm">
                    <PanelHeader className="border-b border-border/60 py-4">
                        <div className="mb-3 space-y-2">
                            <div
                                data-testid="quick-switch-controls"
                                data-responsive-layout={isNarrowViewport ? "stacked" : "inline"}
                                className={`items-start justify-between gap-2 ${isNarrowViewport ? "flex flex-col" : "flex"}`}
                            >
                                <SectionHeader title="Projects" />
                                <Button
                                    data-testid="quick-switch-new-button"
                                    type="button"
                                    onClick={onOpenProjectDirectoryChooser}
                                    variant="outline"
                                    size="xs"
                                >
                                    New
                                </Button>
                            </div>
                            <input
                                ref={projectDirectoryPickerInputRef}
                                data-testid="project-directory-picker-input"
                                type="file"
                                multiple
                                onChange={onProjectDirectorySelected}
                                className="hidden"
                                tabIndex={-1}
                                aria-hidden="true"
                            />
                            {projectRegistrationError ? (
                                <p data-testid="project-registration-error" className="text-xs text-destructive">
                                    {projectRegistrationError}
                                </p>
                            ) : null}
                        </div>
                    </PanelHeader>
                    <PanelContent className={`pt-4 ${isNarrowViewport ? "" : "min-h-0 flex-1 overflow-y-auto pr-1"}`}>
                        <div className={isNarrowViewport ? "" : "min-h-0 flex-1 overflow-y-auto pr-1"}>
                            <ul data-testid="projects-list" className="space-y-1.5">
                                {orderedProjects.length === 0 ? (
                                    <li>
                                        <EmptyState className="text-xs" description="No projects registered yet." />
                                    </li>
                                ) : (
                                    orderedProjects.map((project) => {
                                        const projectPath = project.directoryPath
                                        const isActive = projectPath === activeProjectPath
                                        const projectConversationSummaries = isActive ? activeProjectConversationSummaries : []
                                        const isDeletingProject = pendingDeleteProjectPath === projectPath
                                        return (
                                            <li key={projectPath} className="group/project relative space-y-1">
                                                <button
                                                    type="button"
                                                    onClick={() => {
                                                        void onActivateProject(projectPath)
                                                    }}
                                                    aria-current={isActive ? "true" : undefined}
                                                    title={projectPath}
                                                    className={`w-full rounded-md px-2 py-2 pr-9 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActive
                                                        ? "bg-primary/10 text-foreground"
                                                        : "text-foreground hover:bg-muted/70"
                                                        }`}
                                                >
                                                    <div className="flex items-center gap-2">
                                                        {isActive ? (
                                                            <FolderOpen className="h-4 w-4 shrink-0 text-primary" />
                                                        ) : (
                                                            <Folder className="h-4 w-4 shrink-0 text-muted-foreground" />
                                                        )}
                                                        <span className={`truncate text-sm font-medium ${isActive ? "text-foreground" : "text-foreground/90"}`}>
                                                            {formatProjectListLabel(projectPath)}
                                                        </span>
                                                    </div>
                                                </button>
                                                <button
                                                    type="button"
                                                    aria-label="Remove project"
                                                    title={`Remove project ${formatProjectListLabel(projectPath)}`}
                                                    data-testid={`project-delete-${projectPath}`}
                                                    onClick={() => {
                                                        void onDeleteProject(projectPath)
                                                    }}
                                                    disabled={isDeletingProject}
                                                    className="absolute right-1 top-1 inline-flex h-7 w-7 items-center justify-center rounded text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-destructive focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring group-hover/project:opacity-100 group-focus-within/project:opacity-100 disabled:cursor-not-allowed disabled:opacity-50"
                                                >
                                                    <Trash2 className="h-3.5 w-3.5" />
                                                </button>
                                                {isActive ? (
                                                    <div className="ml-5 border-l border-border/70 pl-2">
                                                        <div
                                                            data-testid="project-thread-controls"
                                                            className="mb-1 flex justify-end"
                                                        >
                                                            <button
                                                                data-testid="project-thread-new-button"
                                                                type="button"
                                                                onClick={onCreateConversationThread}
                                                                className="inline-flex items-center gap-1 rounded px-1.5 py-1 text-[11px] text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                            >
                                                                <Plus className="h-3.5 w-3.5" />
                                                                <span>New thread</span>
                                                            </button>
                                                        </div>
                                                        <ul data-testid="project-thread-list" className="space-y-1">
                                                            {projectConversationSummaries.length === 0 ? (
                                                                <li className="px-2 py-1 text-[11px] text-muted-foreground">
                                                                    No threads yet.
                                                                </li>
                                                            ) : (
                                                                projectConversationSummaries.map((conversation) => {
                                                                    const isActiveConversation = conversation.conversation_id === activeConversationId
                                                                    const ageLabel = formatConversationAgeShort(conversation.updated_at)
                                                                    const isDeletingConversation = pendingDeleteConversationId === conversation.conversation_id
                                                                    return (
                                                                        <li key={conversation.conversation_id} className="group/thread relative">
                                                                            <button
                                                                                type="button"
                                                                                onClick={() => onSelectConversationThread(conversation.conversation_id)}
                                                                                aria-current={isActiveConversation ? "true" : undefined}
                                                                                aria-label={`Open thread ${conversation.title}`}
                                                                                className={`w-full rounded-xl px-2 py-2 pr-9 text-left transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring ${isActiveConversation
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
                                                                            </button>
                                                                            <button
                                                                                type="button"
                                                                                aria-label={`Delete thread ${conversation.title}`}
                                                                                data-testid={`project-thread-delete-${conversation.conversation_id}`}
                                                                                onClick={() => {
                                                                                    void onDeleteConversationThread(conversation.conversation_id, conversation.title)
                                                                                }}
                                                                                disabled={isDeletingConversation}
                                                                                className="absolute right-1 top-1/2 inline-flex h-7 w-7 -translate-y-1/2 items-center justify-center rounded text-muted-foreground opacity-0 transition-opacity hover:bg-muted hover:text-destructive focus-visible:opacity-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring group-hover/thread:opacity-100 group-focus-within/thread:opacity-100 disabled:cursor-not-allowed disabled:opacity-50"
                                                                            >
                                                                                <Trash2 className="h-3.5 w-3.5" />
                                                                            </button>
                                                                        </li>
                                                                    )
                                                                })
                                                            )}
                                                        </ul>
                                                    </div>
                                                ) : null}
                                            </li>
                                        )
                                    })
                                )}
                            </ul>
                        </div>
                    </PanelContent>
                    </Panel>
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
                    <div className="mb-3 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Workflow Event Log</h3>
                        <p className="text-xs text-muted-foreground">
                            Project-scoped operational events and workflow progression.
                        </p>
                    </div>
                    {!activeProjectPath ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-xs text-muted-foreground">
                            Select a project to view workflow events.
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
