import { useEffect, useState } from "react"
import { ChevronDown, ChevronRight, Plus, Trash2 } from "lucide-react"

import { useStore } from "@/store"
import { cn } from "@/lib/utils"

import type { ProjectConversationSummary } from "../model/types"
import { groupChatsByProject, isFolderMissing, projectLabel } from "../model/projectChoices"
import { InlineError } from "@/components/app/inline-error"
import { SidePanelRow } from "@/components/app/side-panel-row"
import { Button } from "@/components/ui/button"
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from "@/components/ui/empty"

/** How many recent chats a project shows before its "more" link. */
export const RECENT_CHATS_PER_PROJECT = 5

type ProjectsSidebarProps = {
    activeProjectPath: string | null
    activeConversationId: string | null
    conversationSummariesByProjectPath: Record<string, ProjectConversationSummary[]>
    conversationSummariesStatusByProjectPath: Record<string, 'idle' | 'loading' | 'ready' | 'error'>
    pendingDeleteConversationId: string | null
    onCreateConversationThread: (projectPath: string) => void | Promise<void>
    onSelectConversationThread: (projectPath: string, conversationId: string) => void
    onDeleteConversationThread: (projectPath: string, conversationId: string, title: string) => void | Promise<void>
    formatConversationAgeShort: (value: string) => string
}

const iconButtonClassName = "flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"

/** Chats grouped by project: collapsible project rows, thread rows underneath, a quiet "N more". */
export function ProjectsSidebar({
    activeProjectPath,
    activeConversationId,
    conversationSummariesByProjectPath,
    conversationSummariesStatusByProjectPath,
    pendingDeleteConversationId,
    onCreateConversationThread,
    onSelectConversationThread,
    onDeleteConversationThread,
    formatConversationAgeShort,
}: ProjectsSidebarProps) {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const projectPagePath = useStore((state) => state.projectPagePath)
    const openProjectPage = useStore((state) => state.openProjectPage)
    // Home starts open; another project opens once a chat in it is shown, and stays as you leave it.
    const [expandedByPath, setExpandedByPath] = useState<Record<string, boolean>>({})
    useEffect(() => {
        if (activeProjectPath) {
            setExpandedByPath((current) => activeProjectPath in current ? current : { ...current, [activeProjectPath]: true })
        }
    }, [activeProjectPath])
    const groups = groupChatsByProject(projectRegistry, conversationSummariesByProjectPath)

    return (
        <ul data-testid="project-thread-list" aria-label="Chats by project" className="flex flex-col gap-1.5">
            {groups.length === 0 ? (
                <li>
                    <Empty className="py-4 text-xs">
                        <EmptyHeader>
                            <EmptyDescription>Loading projects…</EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                </li>
            ) : groups.map(({ project, chats }) => {
                const projectPath = project.directoryPath
                const label = projectLabel(projectRegistry, projectPath)
                const missing = isFolderMissing(project)
                const expanded = expandedByPath[projectPath] ?? Boolean(project.isDefault || projectPath === activeProjectPath)
                const status = conversationSummariesStatusByProjectPath[projectPath] ?? 'idle'
                const recentChats = chats.slice(0, RECENT_CHATS_PER_PROJECT)
                return (
                    <li
                        key={projectPath}
                        data-testid="chats-project-group"
                        data-project-path={projectPath}
                        data-folder-missing={missing ? 'true' : undefined}
                    >
                        <div className={cn(
                            "group/project flex h-8 items-center gap-1 rounded-lg border pl-1 pr-1.5",
                            projectPagePath === projectPath ? "border-border bg-background" : "border-transparent",
                        )}>
                            <button
                                type="button"
                                data-testid="chats-project-toggle"
                                aria-expanded={expanded}
                                aria-label={`${expanded ? 'Collapse' : 'Expand'} ${label}`}
                                onClick={() => setExpandedByPath((current) => ({ ...current, [projectPath]: !expanded }))}
                                className={iconButtonClassName}
                            >
                                {expanded ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
                            </button>
                            <button
                                type="button"
                                data-testid="chats-project-name"
                                title={missing ? `${projectPath} (folder missing)` : projectPath}
                                onClick={() => openProjectPage(projectPath)}
                                className={cn(
                                    "min-w-0 flex-1 truncate text-left text-sm font-semibold outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring",
                                    missing ? "text-muted-foreground" : "text-foreground",
                                )}
                            >
                                {label}
                            </button>
                            {missing ? (
                                <span data-testid="chats-project-missing" className="shrink-0 rounded-full bg-warning/10 px-1.5 py-px text-xs font-medium text-warning">folder missing</span>
                            ) : project.isDefault ? (
                                <span className="shrink-0 text-xs text-muted-foreground">default</span>
                            ) : chats.length > 0 ? (
                                <span className="shrink-0 text-xs text-muted-foreground">{chats.length}</span>
                            ) : null}
                            {!missing ? (
                                <button
                                    type="button"
                                    data-testid="chats-project-new-chat"
                                    aria-label={`New chat in ${label}`}
                                    title={`New chat in ${label}`}
                                    onClick={() => { void onCreateConversationThread(projectPath) }}
                                    className={cn(iconButtonClassName, "opacity-0 transition-opacity focus-visible:opacity-100 group-hover/project:opacity-100 group-focus-within/project:opacity-100")}
                                >
                                    <Plus className="size-3.5" />
                                </button>
                            ) : null}
                        </div>
                        {expanded ? (
                            <ul className="flex flex-col gap-px pb-1 pt-0.5">
                                {status === 'error' && chats.length === 0 ? (
                                    <li className="py-1 pl-7 pr-2"><InlineError dense>Unable to restore the thread list.</InlineError></li>
                                ) : chats.length === 0 ? (
                                    <li className="py-1 pl-7 pr-2 text-xs text-muted-foreground" {...(status === 'ready' ? {} : { 'data-testid': 'project-thread-list-loading', 'aria-live': 'polite' as const })}>
                                        {status !== 'ready' ? 'Restoring chats…' : project.isDefault ? 'Chats outside a repository land here.' : 'No chats yet.'}
                                    </li>
                                ) : recentChats.map((conversation) => {
                                    const isActiveConversation = !projectPagePath
                                        && projectPath === activeProjectPath
                                        && conversation.conversation_id === activeConversationId
                                    const isDeletingConversation = pendingDeleteConversationId === conversation.conversation_id
                                    return (
                                        <li key={conversation.conversation_id} className="group/thread relative" title={conversation.conversation_handle ?? conversation.title}>
                                            <SidePanelRow
                                                data-testid="chats-chat-row"
                                                data-conversation-id={conversation.conversation_id}
                                                onClick={() => onSelectConversationThread(projectPath, conversation.conversation_id)}
                                                selected={isActiveConversation}
                                                aria-label={`Open thread ${conversation.title}`}
                                                title={conversation.title}
                                                trailing={(
                                                    <span className="transition-opacity group-hover/thread:opacity-0 group-focus-within/thread:opacity-0">
                                                        {formatConversationAgeShort(conversation.updated_at)}
                                                    </span>
                                                )}
                                                className="py-1.5 pl-7 pr-8"
                                            />
                                            <button
                                                type="button"
                                                aria-label={`Delete thread ${conversation.title}`}
                                                data-testid={`project-thread-delete-${conversation.conversation_id}`}
                                                onClick={() => {
                                                    void onDeleteConversationThread(projectPath, conversation.conversation_id, conversation.title)
                                                }}
                                                disabled={isDeletingConversation}
                                                className="absolute right-2 top-1/2 -translate-y-1/2 rounded-sm p-0.5 text-muted-foreground opacity-0 transition-opacity hover:text-destructive focus-visible:opacity-100 group-hover/thread:opacity-100 group-focus-within/thread:opacity-100"
                                            >
                                                <Trash2 className="size-3.5" />
                                            </button>
                                        </li>
                                    )
                                })}
                                {chats.length > RECENT_CHATS_PER_PROJECT ? (
                                    <li>
                                        <Button
                                            type="button"
                                            variant="ghost"
                                            size="xs"
                                            data-testid="chats-project-more"
                                            onClick={() => openProjectPage(projectPath)}
                                            className="ml-7 text-muted-foreground"
                                        >
                                            {chats.length - RECENT_CHATS_PER_PROJECT} more
                                        </Button>
                                    </li>
                                ) : null}
                            </ul>
                        ) : null}
                    </li>
                )
            })}
        </ul>
    )
}
