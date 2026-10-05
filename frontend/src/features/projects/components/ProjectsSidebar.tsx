import { useEffect, useState } from "react"
import { ChevronDown, ChevronRight, Plus, Trash2 } from "lucide-react"

import { useStore } from "@/store"
import { cn } from "@/lib/utils"

import { HomeProjectSidebar } from "./HomeProjectSidebar"
import type { ProjectConversationSummary } from "../model/types"
import { groupChatsByProject, isFolderMissing, projectLabel } from "../model/projectChoices"
import { InlineError } from "@/components/app/inline-error"
import {
    Empty,
    EmptyDescription,
    EmptyHeader,
} from "@/components/ui/empty"

/** How many recent chats a project shows before its "more" link. */
export const RECENT_CHATS_PER_PROJECT = 5

type ProjectsSidebarProps = {
    isNarrowViewport: boolean
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

export function ProjectsSidebar({
    isNarrowViewport,
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
        <HomeProjectSidebar className={isNarrowViewport ? "gap-4" : "h-full"}>
            <div
                data-testid="home-sidebar-stack"
                className={`flex ${isNarrowViewport ? "flex-col gap-4" : "h-full min-h-0 flex-col"}`}
            >
                <div
                    data-testid="home-sidebar-primary-surface"
                    className={isNarrowViewport ? "" : "min-h-0 flex-1 overflow-y-auto"}
                >
                    <ul data-testid="project-thread-list" aria-label="Chats by project">
                        {groups.length === 0 ? (
                            <li>
                                <Empty className="px-4 py-4 text-xs text-muted-foreground">
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
                                        "group/project flex items-center gap-1.5 px-3 py-1",
                                        projectPagePath === projectPath && "shadow-[inset_2px_0_0_hsl(var(--primary))]",
                                    )}>
                                        <button
                                            type="button"
                                            data-testid="chats-project-toggle"
                                            aria-expanded={expanded}
                                            aria-label={`${expanded ? 'Collapse' : 'Expand'} ${label}`}
                                            onClick={() => setExpandedByPath((current) => ({ ...current, [projectPath]: !expanded }))}
                                            className="shrink-0 text-muted-foreground hover:text-foreground outline-none focus-visible:ring-2 focus-visible:ring-ring"
                                        >
                                            {expanded ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
                                        </button>
                                        <button
                                            type="button"
                                            data-testid="chats-project-name"
                                            title={missing ? `${projectPath} (folder missing)` : projectPath}
                                            onClick={() => openProjectPage(projectPath)}
                                            className={cn(
                                                "min-w-0 truncate text-left text-sm outline-none hover:text-primary focus-visible:ring-2 focus-visible:ring-ring",
                                                projectPagePath === projectPath ? "text-primary" : "text-foreground",
                                                missing && "text-muted-foreground line-through",
                                            )}
                                        >
                                            {label}
                                        </button>
                                        {missing ? (
                                            <span data-testid="chats-project-missing" className="shrink-0 text-xs text-destructive">folder missing</span>
                                        ) : project.isDefault ? (
                                            <span className="shrink-0 text-xs text-muted-foreground">default</span>
                                        ) : null}
                                        {!missing ? (
                                            <button
                                                type="button"
                                                data-testid="chats-project-new-chat"
                                                aria-label={`New chat in ${label}`}
                                                title={`New chat in ${label}`}
                                                onClick={() => { void onCreateConversationThread(projectPath) }}
                                                className="ml-auto shrink-0 rounded-sm px-1 text-muted-foreground opacity-0 transition-opacity hover:text-primary focus-visible:opacity-100 group-hover/project:opacity-100 group-focus-within/project:opacity-100"
                                            >
                                                <Plus className="size-3.5" />
                                            </button>
                                        ) : null}
                                    </div>
                                    {expanded ? (
                                        <ul className="pb-1">
                                            {status === 'error' && chats.length === 0 ? (
                                                <li className="py-1 pl-9 pr-3"><InlineError dense>Unable to restore the thread list.</InlineError></li>
                                            ) : chats.length === 0 ? (
                                                <li className="py-1 pl-9 pr-3 text-xs text-muted-foreground" {...(status === 'ready' ? {} : { 'data-testid': 'project-thread-list-loading', 'aria-live': 'polite' as const })}>
                                                    {status !== 'ready' ? 'Restoring chats…' : project.isDefault ? 'Chats outside a repository land here.' : 'No chats yet.'}
                                                </li>
                                            ) : recentChats.map((conversation) => {
                                                const isActiveConversation = !projectPagePath
                                                    && projectPath === activeProjectPath
                                                    && conversation.conversation_id === activeConversationId
                                                const isDeletingConversation = pendingDeleteConversationId === conversation.conversation_id
                                                return (
                                                    <li key={conversation.conversation_id} className="group/thread relative">
                                                        <button
                                                            type="button"
                                                            data-testid="chats-chat-row"
                                                            data-conversation-id={conversation.conversation_id}
                                                            onClick={() => onSelectConversationThread(projectPath, conversation.conversation_id)}
                                                            aria-current={isActiveConversation ? "true" : undefined}
                                                            aria-label={`Open thread ${conversation.title}`}
                                                            title={conversation.conversation_handle ?? conversation.title}
                                                            className={cn(
                                                                "flex w-full min-w-0 items-center gap-2 py-1 pl-9 pr-9 text-left text-sm outline-none focus-visible:ring-2 focus-visible:ring-ring",
                                                                isActiveConversation
                                                                    ? "text-primary shadow-[inset_2px_0_0_hsl(var(--primary))]"
                                                                    : "text-foreground/90 hover:text-primary",
                                                            )}
                                                        >
                                                            <span className="min-w-0 flex-1 truncate">{conversation.title}</span>
                                                            <span className="shrink-0 text-xs text-muted-foreground transition-opacity group-hover/thread:opacity-0 group-focus-within/thread:opacity-0">
                                                                {formatConversationAgeShort(conversation.updated_at)}
                                                            </span>
                                                        </button>
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
                                                    <button
                                                        type="button"
                                                        data-testid="chats-project-more"
                                                        onClick={() => openProjectPage(projectPath)}
                                                        className="py-0.5 pl-9 text-xs text-muted-foreground hover:text-primary outline-none focus-visible:ring-2 focus-visible:ring-ring"
                                                    >
                                                        {chats.length - RECENT_CHATS_PER_PROJECT} more
                                                    </button>
                                                </li>
                                            ) : null}
                                        </ul>
                                    ) : null}
                                </li>
                            )
                        })}
                    </ul>
                </div>
            </div>
        </HomeProjectSidebar>
    )
}
