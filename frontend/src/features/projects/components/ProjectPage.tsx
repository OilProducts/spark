import { useEffect, useRef, useState } from 'react'
import { Plus, Search, Settings, Trash2 } from 'lucide-react'

import { useStore } from '@/store'
import { InlineError } from '@/components/app/inline-error'
import { SectionHeading } from '@/components/app/section-heading'
import { Well } from '@/components/app/well'
import { Button } from '@/components/ui/button'
import { ProjectModelSettingsEditor } from '@/features/settings/ProjectModelSettingsEditor'

import { ProjectExecutionSettings } from './ProjectExecutionSettings'
import { RECENT_CHATS_PER_PROJECT, StandingRoleChip } from './ProjectsSidebar'
import type { ProjectConversationSummary } from '../model/types'
import { isFolderMissing, projectLabel } from '../model/projectChoices'
import { sortConversationSummaries } from '../model/conversationState'
import { useRemoveProject } from '../hooks/useRemoveProject'

/**
 * A chat's latest message as the server sends it, its whitespace collapsed, read as its chat shows
 * it: no fence, math or code marks. A reply renders as Markdown, so its heading and bold marks go too
 * and a link reads as its text; a message the user typed keeps every other character.
 */
export const plainPreview = (text: string, role?: string | null) => {
    const plain = text.replace(/(```|~~~)[\w+-]*/g, ' ').replace(/\$\$/g, ' ').replace(/`/g, '')
    return (role === 'user' ? plain : plain
        .replace(/!?\[([^\]]*)\]\([^)]*\)/g, '$1')
        .replace(/(^|\s)#{1,6}(?=\s)/g, '$1')
        .replace(/\*\*(\S(?:[^*]*\S)?)\*\*/g, '$1'))
        .replace(/\s+/g, ' ')
        .trim()
}

/** One project: its path, all its chats, and its settings. */
export function ProjectPage({
    projectPath,
    chats,
    formatConversationAgeShort,
    onCreateConversationThread,
    onSelectConversationThread,
    onDeleteConversationThread,
    pendingDeleteConversationId,
}: {
    projectPath: string
    chats: ProjectConversationSummary[]
    formatConversationAgeShort: (value: string) => string
    onCreateConversationThread: (projectPath: string) => void | Promise<void>
    onSelectConversationThread: (projectPath: string, conversationId: string) => void
    onDeleteConversationThread: (projectPath: string, conversationId: string, title: string) => void | Promise<void>
    pendingDeleteConversationId: string | null
}) {
    const registry = useStore((state) => state.projectRegistry)
    const project = registry[projectPath]
    const { removeProject, removeError, removing } = useRemoveProject(projectPath)
    const label = projectLabel(registry, projectPath)
    const missing = isFolderMissing(project)
    const sortedChats = sortConversationSummaries(chats)
    // More chats than the side panel lists can be searched by title or latest message.
    const [chatQuery, setChatQuery] = useState('')
    const query = chatQuery.trim().toLowerCase()
    const shownChats = query
        ? sortedChats.filter((chat) => `${chat.title}\n${plainPreview(chat.last_message_preview ?? '', chat.last_message_role)}`.toLowerCase().includes(query))
        : sortedChats
    // The gear beside a project opens its page at Settings, below however many chats it has.
    const settingsHeadingRef = useRef<HTMLHeadingElement>(null)
    const showSettings = () => {
        settingsHeadingRef.current?.scrollIntoView?.({ block: 'start' })
        settingsHeadingRef.current?.focus({ preventScroll: true })
    }
    const settingsRequest = useStore((state) => state.projectPageSettingsRequest)
    useEffect(() => {
        if (settingsRequest) showSettings()
    }, [settingsRequest])

    if (!project) {
        // Until the registry loads, a restored page has nothing to show yet.
        return Object.keys(registry).length === 0 ? null : (
            <section data-testid="project-page" className="p-6 text-sm text-muted-foreground">
                This project is no longer registered.
            </section>
        )
    }

    return (
        <section data-testid="project-page" data-project-path={projectPath} className="flex h-full min-h-0 flex-col">
            <header className="shrink-0 border-b border-border py-4">
                <div className="mx-auto flex max-w-4xl flex-wrap items-center gap-3 px-6">
                    <h1 data-testid="project-page-title" className="min-w-0 truncate text-xl font-semibold">{label}</h1>
                    {missing ? <span data-testid="project-page-missing" className="rounded-full bg-warning/10 px-2 py-0.5 text-xs font-medium text-warning">folder missing</span> : null}
                    {project.isDefault ? <span className="text-sm text-muted-foreground">default project</span> : null}
                    <Button
                        type="button"
                        data-testid="project-page-settings-link"
                        className="ml-auto"
                        variant="ghost"
                        size="sm"
                        onClick={showSettings}
                    >
                        <Settings className="size-3.5" />
                        Settings
                    </Button>
                    <Button
                        type="button"
                        data-testid="project-page-new-chat"
                        size="sm"
                        disabled={missing}
                        onClick={() => { void onCreateConversationThread(projectPath) }}
                    >
                        <Plus className="size-3.5" />
                        New chat
                    </Button>
                    {/* Paths wrap after a slash or backslash, not mid-word. */}
                    <p data-testid="project-page-path" className="w-full break-words font-mono text-xs text-muted-foreground">
                        {projectPath.split(/([/\\])/).map((part, index) => (
                            <span key={index}>{part === '/' || part === '\\' ? <>{part}<wbr /></> : part}</span>
                        ))}
                    </p>
                </div>
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto">
                <div className="mx-auto flex max-w-4xl flex-col gap-8 px-6 py-5">
                    <section aria-label="Chats" className="flex flex-col gap-2">
                        <SectionHeading count={chats.length}>Chats</SectionHeading>
                        {chats.length > RECENT_CHATS_PER_PROJECT ? (
                            <label className="flex h-8 items-center gap-2 rounded-lg border border-border bg-background px-2.5 text-muted-foreground focus-within:ring-2 focus-within:ring-ring">
                                <Search aria-hidden="true" className="size-3.5 shrink-0" />
                                <input
                                    type="search"
                                    value={chatQuery}
                                    onChange={(event) => setChatQuery(event.target.value)}
                                    placeholder="Search chats"
                                    aria-label="Search chats by title or latest message"
                                    data-testid="project-page-chat-search"
                                    className="min-w-0 flex-1 bg-transparent text-sm text-foreground outline-none placeholder:text-muted-foreground"
                                />
                            </label>
                        ) : null}
                        {query && shownChats.length === 0 ? (
                            <p data-testid="project-page-chat-search-empty" className="text-sm text-muted-foreground">No chats match “{chatQuery.trim()}”.</p>
                        ) : sortedChats.length === 0 ? (
                            <p className="text-sm text-muted-foreground">
                                {project.isDefault ? 'Chats you start outside a repository live here.' : 'No chats yet.'}
                            </p>
                        ) : (
                            <ul data-testid="project-page-chats" className="-mx-2.5 flex flex-col">
                                {shownChats.map((chat) => (
                                    <li key={chat.conversation_id} className="flex items-center gap-1 rounded-lg hover:bg-muted">
                                        <button
                                            type="button"
                                            data-testid="project-page-chat"
                                            data-standing-role={chat.standing_role?.name}
                                            onClick={() => onSelectConversationThread(projectPath, chat.conversation_id)}
                                            className="flex min-w-0 flex-1 flex-col gap-px rounded-lg px-2.5 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                                        >
                                            <span className="truncate text-sm font-medium">{chat.title}</span>
                                            <span className="flex min-w-0 items-center gap-1.5 text-xs text-muted-foreground">
                                                {chat.standing_role ? <StandingRoleChip state={chat.standing_role.state} /> : <span className="shrink-0">{formatConversationAgeShort(chat.updated_at)}</span>}
                                                {plainPreview(chat.last_message_preview ?? '', chat.last_message_role) ? (
                                                    <span data-testid="project-page-chat-preview" className="min-w-0 truncate">· {plainPreview(chat.last_message_preview ?? '', chat.last_message_role)}</span>
                                                ) : null}
                                            </span>
                                        </button>
                                        {chat.standing_role ? null : <Button
                                            type="button"
                                            variant="ghost"
                                            size="icon-sm"
                                            aria-label={`Delete thread ${chat.title}`}
                                            data-testid={`project-page-chat-delete-${chat.conversation_id}`}
                                            disabled={pendingDeleteConversationId === chat.conversation_id}
                                            onClick={() => { void onDeleteConversationThread(projectPath, chat.conversation_id, chat.title) }}
                                            className="mr-1 text-muted-foreground hover:text-destructive"
                                        >
                                            <Trash2 className="size-3.5" />
                                        </Button>}
                                    </li>
                                ))}
                            </ul>
                        )}
                    </section>

                    <section aria-label="Settings" className="flex scroll-mt-5 flex-col gap-4">
                        <SectionHeading ref={settingsHeadingRef} tabIndex={-1} className="rounded-sm outline-none focus-visible:ring-2 focus-visible:ring-ring">Settings</SectionHeading>
                        <ProjectExecutionSettings key={projectPath} projectPath={projectPath} />
                        <ProjectModelSettingsEditor key={`model:${projectPath}`} projectPath={projectPath} />
                        <Well className="flex flex-col gap-3">
                            <SectionHeading as="h3">Remove project</SectionHeading>
                            {project.isDefault ? (
                                <p data-testid="project-page-remove-refused" className="text-xs text-muted-foreground">
                                    Home is Spark's default project and can't be removed.
                                </p>
                            ) : (
                                <p className="text-xs text-muted-foreground">
                                    Removing deletes its registration, Spark threads, and installed automation; its files, run history, learned memory, and working files stay, and running pipelines continue.
                                </p>
                            )}
                            {removeError ? <InlineError dense>{removeError}</InlineError> : null}
                            <Button
                                type="button"
                                data-testid="project-page-remove"
                                variant="destructive"
                                size="sm"
                                disabled={project.isDefault || removing}
                                onClick={() => { void removeProject() }}
                                className="self-start"
                            >
                                <Trash2 className="size-3.5" />
                                Remove project…
                            </Button>
                        </Well>
                    </section>
                </div>
            </div>
        </section>
    )
}
