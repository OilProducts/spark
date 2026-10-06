import { Plus, Trash2 } from 'lucide-react'

import { useStore } from '@/store'
import { InlineError } from '@/components/app/inline-error'
import { SectionHeading } from '@/components/app/section-heading'
import { Well } from '@/components/app/well'
import { Button } from '@/components/ui/button'
import { ProjectModelSettingsEditor } from '@/features/settings/ProjectModelSettingsEditor'

import { ProjectExecutionSettings } from './ProjectExecutionSettings'
import type { ProjectConversationSummary } from '../model/types'
import { isFolderMissing, projectLabel } from '../model/projectChoices'
import { useRemoveProject } from '../hooks/useRemoveProject'

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
    const sortedChats = [...chats].sort((left, right) => right.updated_at.localeCompare(left.updated_at))

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
            <header className="shrink-0 border-b border-border px-6 py-4">
                <div className="mx-auto flex max-w-4xl flex-wrap items-center gap-3">
                    <h1 data-testid="project-page-title" className="min-w-0 truncate text-xl font-semibold">{label}</h1>
                    {missing ? <span data-testid="project-page-missing" className="rounded-full bg-warning/10 px-2 py-0.5 text-xs font-medium text-warning">folder missing</span> : null}
                    {project.isDefault ? <span className="text-sm text-muted-foreground">default project</span> : null}
                    <Button
                        type="button"
                        data-testid="project-page-new-chat"
                        className="ml-auto"
                        size="sm"
                        disabled={missing}
                        onClick={() => { void onCreateConversationThread(projectPath) }}
                    >
                        <Plus className="size-3.5" />
                        New chat
                    </Button>
                    <p data-testid="project-page-path" className="w-full break-all font-mono text-xs text-muted-foreground">{projectPath}</p>
                </div>
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto">
                <div className="mx-auto flex max-w-4xl flex-col gap-8 px-6 py-5">
                    <section aria-label="Chats" className="flex flex-col gap-2">
                        <SectionHeading count={chats.length}>Chats</SectionHeading>
                        {sortedChats.length === 0 ? (
                            <p className="text-sm text-muted-foreground">
                                {project.isDefault ? 'Chats you start outside a repository live here.' : 'No chats yet.'}
                            </p>
                        ) : (
                            <ul data-testid="project-page-chats" className="-mx-2.5 flex flex-col">
                                {sortedChats.map((chat) => (
                                    <li key={chat.conversation_id} className="flex items-center gap-1 rounded-lg hover:bg-muted">
                                        <button
                                            type="button"
                                            data-testid="project-page-chat"
                                            onClick={() => onSelectConversationThread(projectPath, chat.conversation_id)}
                                            className="flex min-w-0 flex-1 flex-col gap-px rounded-lg px-2.5 py-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring"
                                        >
                                            <span className="truncate text-sm font-medium">{chat.title}</span>
                                            <span className="truncate text-xs text-muted-foreground">
                                                {chat.conversation_handle ? <><span className="font-mono">{chat.conversation_handle}</span> · </> : null}
                                                {formatConversationAgeShort(chat.updated_at)}
                                            </span>
                                        </button>
                                        <Button
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
                                        </Button>
                                    </li>
                                ))}
                            </ul>
                        )}
                    </section>

                    <section aria-label="Settings" className="flex flex-col gap-4">
                        <SectionHeading>Settings</SectionHeading>
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
                                    Removing deletes its Spark threads, workflow history, and runs; project files stay on disk.
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
