import { Plus, Trash2 } from 'lucide-react'

import { useStore } from '@/store'
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button'
import { Card, CardContent, CardHeader } from '@/components/ui/card'
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
        <section data-testid="project-page" data-project-path={projectPath} className="h-full overflow-y-auto">
            <div className="mx-auto flex max-w-4xl flex-col gap-6 p-6">
                <header className="space-y-1">
                    <div className="flex flex-wrap items-baseline gap-3">
                        <h1 data-testid="project-page-title" className="text-2xl font-light tracking-tight">{label}</h1>
                        {missing ? <span data-testid="project-page-missing" className="text-sm text-destructive">folder missing</span> : null}
                        {project.isDefault ? <span className="text-sm text-muted-foreground">default project</span> : null}
                        <Button
                            type="button"
                            data-testid="project-page-new-chat"
                            className="ml-auto"
                            size="sm"
                            variant="outline"
                            disabled={missing}
                            onClick={() => { void onCreateConversationThread(projectPath) }}
                        >
                            <Plus className="size-3.5" />
                            New chat
                        </Button>
                    </div>
                    <p data-testid="project-page-path" className="break-all font-mono text-xs text-muted-foreground">{projectPath}</p>
                </header>

                <section aria-label="Chats" className="space-y-2">
                    <h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Chats · {chats.length}</h2>
                    {sortedChats.length === 0 ? (
                        <p className="text-sm text-muted-foreground">
                            {project.isDefault ? 'Chats you start outside a repository live here.' : 'No chats yet.'}
                        </p>
                    ) : (
                        <ul data-testid="project-page-chats" className="divide-y divide-border border-y border-border">
                            {sortedChats.map((chat) => (
                                <li key={chat.conversation_id} className="flex items-center gap-2">
                                    <button
                                        type="button"
                                        data-testid="project-page-chat"
                                        onClick={() => onSelectConversationThread(projectPath, chat.conversation_id)}
                                        className="flex min-w-0 flex-1 items-baseline gap-3 py-2 text-left text-sm outline-none hover:text-primary focus-visible:ring-2 focus-visible:ring-ring"
                                    >
                                        <span className="min-w-0 flex-1 truncate">{chat.title}</span>
                                        {chat.conversation_handle ? <span className="shrink-0 font-mono text-xs text-muted-foreground">{chat.conversation_handle}</span> : null}
                                        <span className="shrink-0 text-xs text-muted-foreground">{formatConversationAgeShort(chat.updated_at)}</span>
                                    </button>
                                    <button
                                        type="button"
                                        aria-label={`Delete thread ${chat.title}`}
                                        data-testid={`project-page-chat-delete-${chat.conversation_id}`}
                                        disabled={pendingDeleteConversationId === chat.conversation_id}
                                        onClick={() => { void onDeleteConversationThread(projectPath, chat.conversation_id, chat.title) }}
                                        className="shrink-0 rounded-sm p-1 text-muted-foreground outline-none hover:text-destructive focus-visible:ring-2 focus-visible:ring-ring"
                                    >
                                        <Trash2 className="size-3.5" />
                                    </button>
                                </li>
                            ))}
                        </ul>
                    )}
                </section>

                <section aria-label="Settings" className="space-y-4">
                    <h2 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">Settings</h2>
                    <ProjectExecutionSettings key={projectPath} projectPath={projectPath} />
                    <ProjectModelSettingsEditor key={`model:${projectPath}`} projectPath={projectPath} />
                    <Card className="gap-3 py-4">
                        <CardHeader className="px-4"><h3 className="text-lg font-light">Remove project</h3></CardHeader>
                        <CardContent className="space-y-3 px-4">
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
                            >
                                <Trash2 className="size-3.5" />
                                Remove project…
                            </Button>
                        </CardContent>
                    </Card>
                </section>
            </div>
        </section>
    )
}
