import { useEffect, useState } from "react"
import { FolderPlus, Plus } from "lucide-react"

import { ViewLayout } from "@/components/app/view-layout"
import { InlineError } from "@/components/app/inline-error"
import { Button } from "@/components/ui/button"
import { ProjectBrowserDialog } from "./components/ProjectBrowserDialog"
import { ProjectConversationHistory } from "./components/ProjectConversationHistory"
import { ProjectConversationSurface } from "./components/ProjectConversationSurface"
import { ProjectPage } from "./components/ProjectPage"
import { ProjectPicker } from "./components/ProjectPicker"
import { ProjectsSidebar } from "./components/ProjectsSidebar"
import { useAddProject } from "./hooks/useAddProject"
import { useProjectsHomeController } from "./hooks/useProjectsHomeController"
import { useProjectRegistryBootstrap } from "./hooks/useProjectRegistryBootstrap"
import { useStore } from "@/store"

/** The Chats view: chats grouped by project in the panel, the chat or a project page in the main area. */
export function ProjectsPanel() {
    const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
    const shouldBootstrapRegistry = useStore((state) => Object.keys(state.projectRegistry).length === 0)
    const projectPagePath = useStore((state) => state.projectPagePath)
    const [registryBootstrapError, setRegistryBootstrapError] = useState<string | null>(null)
    const { historyProps, sidebarProps, surfaceProps } = useProjectsHomeController()
    const addProject = useAddProject()
    const { onCreateConversationThread } = sidebarProps

    // The Overview's New chat starts one here, in the project it picked.
    useEffect(() => {
        const onNewChat = (event: Event) => { void onCreateConversationThread((event as CustomEvent<string>).detail) }
        window.addEventListener('spark:new-chat', onNewChat)
        return () => window.removeEventListener('spark:new-chat', onNewChat)
    }, [onCreateConversationThread])

    useProjectRegistryBootstrap({
        hydrateProjectRegistry,
        enabled: shouldBootstrapRegistry,
        onError: setRegistryBootstrapError,
    })

    return (
        <section data-testid="projects-panel" data-home-panel="true" data-responsive-layout={sidebarProps.isNarrowViewport ? "stacked" : "split"} className="h-full">
            <ViewLayout
                view="chats"
                title="Chats"
                actions={(
                    <ProjectPicker
                        heading="New chat in"
                        testId="new-chat-project-picker"
                        onPick={(projectPath) => { void sidebarProps.onCreateConversationThread(projectPath) }}
                    >
                        <Button type="button" variant="outline" size="sm" data-testid="project-thread-new-button">
                            <Plus className="size-3.5" />
                            New
                        </Button>
                    </ProjectPicker>
                )}
                panel={(
                    <div className="flex h-full min-h-0 flex-col">
                        <div className="min-h-0 flex-1 overflow-y-auto">
                            <ProjectsSidebar {...sidebarProps} />
                        </div>
                        <div className="shrink-0 pt-2">
                            <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                data-testid="add-project-button"
                                onClick={() => { void addProject.onOpenProjectDirectoryChooser() }}
                                className="text-muted-foreground"
                            >
                                <FolderPlus />
                                Add project
                            </Button>
                            {addProject.projectBrowserErrorMessage && !addProject.isProjectBrowserOpen ? (
                                <InlineError dense className="mt-2">{addProject.projectBrowserErrorMessage}</InlineError>
                            ) : null}
                        </div>
                    </div>
                )}
            >
                {projectPagePath ? (
                    <ProjectPage
                        projectPath={projectPagePath}
                        chats={sidebarProps.conversationSummariesByProjectPath[projectPagePath] ?? []}
                        formatConversationAgeShort={sidebarProps.formatConversationAgeShort}
                        onCreateConversationThread={sidebarProps.onCreateConversationThread}
                        onSelectConversationThread={sidebarProps.onSelectConversationThread}
                        onDeleteConversationThread={sidebarProps.onDeleteConversationThread}
                        pendingDeleteConversationId={sidebarProps.pendingDeleteConversationId}
                    />
                ) : (
                    <ProjectConversationSurface
                        {...surfaceProps}
                        panelError={registryBootstrapError || surfaceProps.panelError}
                        historyContent={<ProjectConversationHistory {...historyProps} />}
                    />
                )}
            </ViewLayout>
            <ProjectBrowserDialog
                open={addProject.isProjectBrowserOpen}
                currentPath={addProject.projectBrowserState?.current_path ?? null}
                parentPath={addProject.projectBrowserState?.parent_path ?? null}
                roots={addProject.projectBrowserState?.roots ?? []}
                entries={addProject.projectBrowserState?.entries ?? []}
                errorMessage={addProject.projectBrowserErrorMessage}
                isLoading={addProject.isProjectBrowserLoading}
                onBrowse={addProject.onBrowseProjectDirectory}
                onOpenChange={addProject.onSetProjectBrowserOpen}
                onSelectCurrentFolder={addProject.onSelectProjectBrowserDirectory}
            />
        </section>
    )
}
