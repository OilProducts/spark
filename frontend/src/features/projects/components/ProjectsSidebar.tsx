import { useEffect, useState, useSyncExternalStore } from "react"
import { ChevronDown, ChevronRight, Pin, Plus, Settings, Trash2 } from "lucide-react"

import { useStore } from "@/store"
import { cn } from "@/lib/utils"

import type { ProjectConversationSummary } from "../model/types"
import { chatStatus, markChatRead, readChatReadState, subscribeChatReadState, type ChatStatus } from "../model/chatReadState"
import { completePreferenceInteraction } from "@/features/settings/services/clientPreferences"
import { updateProjectStateValidated } from "@/lib/workspaceClient"
import { chatProjectKey, groupChatsByProject, isFolderMissing, projectLabel } from "../model/projectChoices"
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
    /** The project list couldn't load (the chat area says why); it keeps retrying. */
    loadFailed?: boolean
}

/** A standing role's state, shown on its pinned row in place of the age. */
export function StandingRoleChip({ state }: { state: 'idle' | 'working' | 'stopped' }) {
    return (
        <span data-testid="standing-role-state" className={cn(
            "shrink-0 rounded-full px-1.5 py-px text-xs font-medium",
            state === 'working' ? "bg-primary/10 text-primary" : state === 'stopped' ? "bg-warning/10 text-warning" : "bg-muted text-muted-foreground",
        )}>
            {state}
        </span>
    )
}

// A chat row's status: a dot in the indent before its title, and words on its second line when it is busy or waits on you.
const STATUS_DOT: Record<ChatStatus, string | null> = {
    'needs-you': "bg-warning",
    working: "bg-info animate-pulse",
    unread: "bg-info",
    read: null,
}

function subscribeVisibility(listener: () => void) {
    document.addEventListener("visibilitychange", listener)
    return () => document.removeEventListener("visibilitychange", listener)
}

const iconButtonClassName = "flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
// A project row's actions show on hover or focus, and always on touch screens, which cannot hover.
const rowActionClassName = cn(iconButtonClassName, "opacity-0 transition-opacity focus-visible:opacity-100 group-hover/project:opacity-100 group-focus-within/project:opacity-100 pointer-coarse:opacity-100")

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
    loadFailed,
}: ProjectsSidebarProps) {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const projectPagePath = useStore((state) => state.projectPagePath)
    const openProjectPage = useStore((state) => state.openProjectPage)
    // Home starts open; the open chat's project opens whenever another chat is shown, and stays as you leave it.
    const [expandedByPath, setExpandedByPath] = useState<Record<string, boolean>>({})
    useEffect(() => {
        if (activeProjectPath) {
            setExpandedByPath((current) => current[activeProjectPath] ? current : { ...current, [activeProjectPath]: true })
        }
    }, [activeProjectPath, activeConversationId])
    const order = useStore((state) => state.chatProjectOrder)
    const groups = groupChatsByProject(projectRegistry, conversationSummariesByProjectPath, order)
    const [dragging, setDragging] = useState<string | null>(null)
    const [dropTarget, setDropTarget] = useState<string | null>(null)
    const [pinError, setPinError] = useState<string | null>(null)
    async function togglePin(path: string) {
        setPinError(null)
        try {
            const saved = await updateProjectStateValidated({ project_path: path, is_favorite: !projectRegistry[path].isFavorite })
            const state = useStore.getState()
            const current = state.projectRegistry[path]
            if (current && current.isFavorite !== saved.is_favorite) state.toggleProjectFavorite(path)
        } catch {
            setPinError(`Couldn’t save the pin for ${projectLabel(projectRegistry, path)}.`)
        }
    }

    function moveProject(path: string, targetIndex: number) {
        const from = groups.findIndex(({ project }) => project.directoryPath === path)
        const source = groups[from]?.project
        const target = groups[targetIndex]?.project
        if (!source || !target || source.isDefault || target.isDefault || source.isFavorite !== target.isFavorite || from === targetIndex) return
        const keys = groups.map(({ project }) => chatProjectKey(project))
        keys.splice(from, 1)
        keys.splice(targetIndex, 0, chatProjectKey(source))
        useStore.setState({ chatProjectOrder: keys })
        completePreferenceInteraction({ chat_project_order: keys })
    }

    // The open chat is read while it is on screen: the Chats view is showing in a visible tab.
    // ponytail: on a phone the list can show while a chat stays open; that chat still counts as seen.
    const viewMode = useStore((state) => state.viewMode)
    const pageVisible = useSyncExternalStore(subscribeVisibility, () => document.visibilityState === "visible")
    const readState = useSyncExternalStore(subscribeChatReadState, readChatReadState)
    const activeSummary = activeProjectPath && activeConversationId
        ? conversationSummariesByProjectPath[activeProjectPath]?.find((chat) => chat.conversation_id === activeConversationId)
        : undefined
    const chatOnScreen = pageVisible && (viewMode === "home" || viewMode === "projects") && !projectPagePath
    useEffect(() => {
        if (chatOnScreen && activeSummary) markChatRead(activeSummary.conversation_id, activeSummary.revision)
    }, [chatOnScreen, activeSummary])

    return (
        <ul data-testid="project-thread-list" aria-label="Chats by project" className="flex flex-col gap-1.5">
            {pinError && <li><InlineError dense>{pinError}</InlineError></li>}
            {groups.length === 0 && loadFailed ? (
                <li><InlineError dense>Couldn’t load projects. Spark keeps trying.</InlineError></li>
            ) : groups.length === 0 ? (
                <li>
                    <Empty className="py-4 text-xs">
                        <EmptyHeader>
                            <EmptyDescription>Loading projects…</EmptyDescription>
                        </EmptyHeader>
                    </Empty>
                </li>
            ) : groups.map(({ project, chats }, index) => {
                const projectPath = project.directoryPath
                const label = projectLabel(projectRegistry, projectPath)
                const missing = isFolderMissing(project)
                const expanded = expandedByPath[projectPath] ?? Boolean(project.isDefault || projectPath === activeProjectPath)
                const status = conversationSummariesStatusByProjectPath[projectPath] ?? 'idle'
                // Standing roles stay pinned above the recent chats; an older chat opened elsewhere,
                // such as from search, joins them so the open chat always has its row.
                const ordinaryChats = chats.filter((chat) => !chat.standing_role)
                const shownOrdinaryChats = ordinaryChats.filter((chat, index) => index < RECENT_CHATS_PER_PROJECT
                    || (projectPath === activeProjectPath && chat.conversation_id === activeConversationId))
                const recentChats = [...chats.filter((chat) => chat.standing_role), ...shownOrdinaryChats]
                return (
                    <li
                        key={projectPath}
                        data-testid="chats-project-group"
                        data-project-path={projectPath}
                        data-folder-missing={missing ? 'true' : undefined}
                    >
                        <div onDragOver={(event) => {
                            if (dragging && dragging !== projectPath && !project.isDefault && projectRegistry[dragging]?.isFavorite === project.isFavorite) {
                                event.preventDefault()
                                setDropTarget(projectPath)
                            }
                        }} onDragLeave={(event) => {
                            // Moving onto the row's own buttons fires dragleave; only clear when the pointer leaves the row.
                            if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropTarget(null)
                        }} onDrop={(event) => {
                            if (!dragging) return
                            event.preventDefault()
                            moveProject(dragging, index)
                            setDragging(null)
                            setDropTarget(null)
                        }} className={cn(
                            "group/project relative flex h-8 items-center gap-1 rounded-lg border pl-1 pr-1.5",
                            projectPagePath === projectPath ? "border-border bg-background" : "border-transparent",
                            dragging === projectPath && "opacity-50",
                        )}>
                            {/* The dragged project takes this row's place: it lands below the row when moving down, above when moving up. */}
                            {dragging && dropTarget === projectPath && (
                                <span aria-hidden data-testid="chats-project-drop-indicator" className={cn(
                                    "pointer-events-none absolute inset-x-1 h-0.5 rounded-full bg-primary",
                                    groups.findIndex(({ project }) => project.directoryPath === dragging) < index ? "-bottom-1" : "-top-1",
                                )} />
                            )}
                            {/* The project's name shows or hides its chats; its settings are behind the gear. */}
                            <button
                                type="button"
                                data-testid="chats-project-toggle"
                                draggable={!project.isDefault}
                                title={project.isDefault ? undefined : "Drag to reorder, or use Alt + Up or Down"}
                                aria-keyshortcuts={project.isDefault ? undefined : "Alt+ArrowUp Alt+ArrowDown"}
                                onDragStart={(event) => {
                                    event.dataTransfer.effectAllowed = "move"
                                    event.dataTransfer.setData("text/plain", label)
                                    setDragging(projectPath)
                                }}
                                onDragEnd={() => { setDragging(null); setDropTarget(null) }}
                                onKeyDown={(event) => {
                                    if (!project.isDefault && event.altKey && (event.key === "ArrowUp" || event.key === "ArrowDown")) {
                                        event.preventDefault()
                                        moveProject(projectPath, index + (event.key === "ArrowUp" ? -1 : 1))
                                    }
                                }}
                                aria-expanded={expanded}
                                aria-label={`${expanded ? 'Collapse' : 'Expand'} ${label}`}
                                onClick={() => setExpandedByPath((current) => ({ ...current, [projectPath]: !expanded }))}
                                className="flex h-full min-w-0 flex-1 items-center gap-1 rounded-md text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                            >
                                <span className="flex size-6 shrink-0 items-center justify-center text-muted-foreground">
                                    {expanded ? <ChevronDown className="size-3.5" /> : <ChevronRight className="size-3.5" />}
                                </span>
                                <span
                                    data-testid="chats-project-name"
                                    title={missing ? `${projectPath} (folder missing)` : projectPath}
                                    className={cn("min-w-0 truncate text-sm font-semibold", missing ? "text-muted-foreground" : "text-foreground")}
                                >
                                    {label}
                                </span>
                            </button>
                            {!project.isDefault && (
                                <button type="button" aria-label={`${project.isFavorite ? "Unpin" : "Pin"} ${label}`}
                                    title={project.isFavorite ? "Unpin project" : "Pin project"}
                                    className={project.isFavorite ? iconButtonClassName : rowActionClassName}
                                    onClick={() => void togglePin(projectPath)}>
                                    <Pin className={cn("size-3.5", project.isFavorite && "fill-current")} />
                                </button>
                            )}
                            {missing ? (
                                <span data-testid="chats-project-missing" className="shrink-0 rounded-full bg-warning/10 px-1.5 py-px text-xs font-medium text-warning">folder missing</span>
                            ) : project.isDefault ? (
                                <span className="shrink-0 text-xs text-muted-foreground">default</span>
                            ) : chats.length > 0 ? (
                                <span className="shrink-0 text-xs text-muted-foreground">{chats.length}</span>
                            ) : null}
                            <button
                                type="button"
                                data-testid="chats-project-settings"
                                aria-label={`Project settings for ${label}`}
                                title="Project settings"
                                onClick={() => openProjectPage(projectPath, 'settings')}
                                className={rowActionClassName}
                            >
                                <Settings className="size-3.5" />
                            </button>
                            {!missing ? (
                                <button
                                    type="button"
                                    data-testid="chats-project-new-chat"
                                    aria-label={`New chat in ${label}`}
                                    title={`New chat in ${label}`}
                                    onClick={() => { void onCreateConversationThread(projectPath) }}
                                    className={rowActionClassName}
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
                                    const chatState = chatStatus(conversation, readState)
                                    return (
                                        <li key={conversation.conversation_id} className="group/thread relative" title={conversation.title}>
                                            {STATUS_DOT[chatState] ? (
                                                <span aria-hidden="true" className={cn("pointer-events-none absolute left-3 top-[13px] size-[7px] rounded-full", STATUS_DOT[chatState])} />
                                            ) : null}
                                            <SidePanelRow
                                                data-testid="chats-chat-row"
                                                data-conversation-id={conversation.conversation_id}
                                                data-standing-role={conversation.standing_role?.name}
                                                data-chat-status={chatState}
                                                onClick={() => onSelectConversationThread(projectPath, conversation.conversation_id)}
                                                selected={isActiveConversation}
                                                // The row's name is its visible title; the ID and age are read as its description.
                                                aria-describedby={`chat-row-meta-${conversation.conversation_id}`}
                                                title={chatState === "unread" ? <span className="font-semibold">{conversation.title}</span> : conversation.title}
                                                meta={(
                                                    <span id={`chat-row-meta-${conversation.conversation_id}`} aria-hidden data-testid="chats-chat-row-meta">
                                                        {chatState === "unread" ? <span className="sr-only">Unread · </span> : null}
                                                        {conversation.conversation_handle ? <><span className="font-mono">{conversation.conversation_handle}</span> · </> : null}
                                                        {chatState === "needs-you" ? <span className="text-warning">Needs you</span>
                                                            : chatState === "working" ? <span className="text-info">Working…</span>
                                                            : formatConversationAgeShort(conversation.updated_at)}
                                                    </span>
                                                )}
                                                trailing={conversation.standing_role ? <StandingRoleChip state={conversation.standing_role.state} /> : null}
                                                className="py-1.5 pl-7 pr-8"
                                            />
                                            {conversation.standing_role ? null : <button
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
                                            </button>}
                                        </li>
                                    )
                                })}
                                {ordinaryChats.length > shownOrdinaryChats.length ? (
                                    <li>
                                        <Button
                                            type="button"
                                            variant="ghost"
                                            size="xs"
                                            data-testid="chats-project-more"
                                            onClick={() => openProjectPage(projectPath)}
                                            className="ml-7 text-muted-foreground"
                                        >
                                            {ordinaryChats.length - shownOrdinaryChats.length} more
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
