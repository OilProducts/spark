import { useStore } from "@/store"
import { useState } from "react"

const buildProjectScopedArtifactId = (artifactType: "conversation" | "spec" | "plan", projectPath: string) => {
    const normalizedProjectKey = projectPath
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, "-")
        .replace(/(^-|-$)/g, "")
    const suffix = normalizedProjectKey || "project"
    return `${artifactType}-${suffix}-${Date.now()}`
}

export function ProjectsPanel() {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const projects = Object.values(projectRegistry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectScopedWorkspaces = useStore((state) => state.projectScopedWorkspaces)
    const projectRegistrationError = useStore((state) => state.projectRegistrationError)
    const registerProject = useStore((state) => state.registerProject)
    const clearProjectRegistrationError = useStore((state) => state.clearProjectRegistrationError)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const setConversationId = useStore((state) => state.setConversationId)
    const setSpecId = useStore((state) => state.setSpecId)
    const setPlanId = useStore((state) => state.setPlanId)
    const [directoryPathInput, setDirectoryPathInput] = useState("")
    const activeProjectScope = activeProjectPath ? projectScopedWorkspaces[activeProjectPath] : null

    const onRegisterProject = () => {
        const result = registerProject(directoryPathInput)
        if (result.ok) {
            setDirectoryPathInput("")
        }
    }

    const onOpenConversation = () => {
        if (!activeProjectPath) {
            return
        }
        setConversationId(activeProjectScope?.conversationId || buildProjectScopedArtifactId("conversation", activeProjectPath))
    }

    const onOpenSpec = () => {
        if (!activeProjectPath) {
            return
        }
        setSpecId(activeProjectScope?.specId || buildProjectScopedArtifactId("spec", activeProjectPath))
    }

    const onOpenPlan = () => {
        if (!activeProjectPath) {
            return
        }
        setPlanId(activeProjectScope?.planId || buildProjectScopedArtifactId("plan", activeProjectPath))
    }

    return (
        <section data-testid="projects-panel" className="flex-1 overflow-auto p-6">
            <div className="mx-auto w-full max-w-3xl space-y-6">
                <div className="space-y-1">
                    <h2 className="text-lg font-semibold">Projects</h2>
                    <p className="text-sm text-muted-foreground">
                        Project registration, selection, and workflow scoping live in this workspace.
                    </p>
                </div>
                <div className="rounded-md border border-border bg-card p-4 text-sm text-muted-foreground shadow-sm">
                    Projects workspace is now a first-class navigation area. Project registry and Git gating controls are tracked in the next checklist slices.
                </div>
                <div data-testid="project-scope-entrypoints" className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="mb-3 space-y-1">
                        <h3 className="text-sm font-semibold text-foreground">Project-Scoped Entry Points</h3>
                        <p className="text-xs text-muted-foreground">
                            Conversation, spec, and plan artifacts are scoped to the active project.
                        </p>
                    </div>
                    {!activeProjectPath ? (
                        <p className="rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
                            Select an active project to access conversation, spec, and plan entry points.
                        </p>
                    ) : (
                        <div className="space-y-3">
                            <div data-testid="project-conversation-entrypoint" className="rounded-md border border-border px-3 py-2">
                                <div className="mb-2 flex items-center justify-between gap-3">
                                    <p className="text-sm font-medium text-foreground">Conversation</p>
                                    <button
                                        type="button"
                                        onClick={onOpenConversation}
                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                                    >
                                        {activeProjectScope?.conversationId ? "Open conversation" : "Start conversation"}
                                    </button>
                                </div>
                                <p className="truncate text-xs text-muted-foreground">
                                    {activeProjectScope?.conversationId || "No conversation artifact selected yet."}
                                </p>
                            </div>
                            <div data-testid="project-spec-entrypoint" className="rounded-md border border-border px-3 py-2">
                                <div className="mb-2 flex items-center justify-between gap-3">
                                    <p className="text-sm font-medium text-foreground">Spec</p>
                                    <button
                                        type="button"
                                        onClick={onOpenSpec}
                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                                    >
                                        {activeProjectScope?.specId ? "Open spec" : "Create spec"}
                                    </button>
                                </div>
                                <p className="truncate text-xs text-muted-foreground">
                                    {activeProjectScope?.specId || "No spec artifact selected yet."}
                                </p>
                            </div>
                            <div data-testid="project-plan-entrypoint" className="rounded-md border border-border px-3 py-2">
                                <div className="mb-2 flex items-center justify-between gap-3">
                                    <p className="text-sm font-medium text-foreground">Plan</p>
                                    <button
                                        type="button"
                                        onClick={onOpenPlan}
                                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                                    >
                                        {activeProjectScope?.planId ? "Open plan" : "Create plan"}
                                    </button>
                                </div>
                                <p className="truncate text-xs text-muted-foreground">
                                    {activeProjectScope?.planId || "No plan artifact selected yet."}
                                </p>
                            </div>
                        </div>
                    )}
                </div>
                <div className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="mb-3 flex gap-2">
                        <input
                            data-testid="project-path-input"
                            type="text"
                            value={directoryPathInput}
                            onChange={(event) => {
                                setDirectoryPathInput(event.target.value)
                                clearProjectRegistrationError()
                            }}
                            placeholder="/absolute/path/to/project"
                            className="h-9 flex-1 rounded-md border border-input bg-background px-3 text-sm"
                        />
                        <button
                            data-testid="project-register-button"
                            type="button"
                            onClick={onRegisterProject}
                            className="inline-flex h-9 items-center justify-center rounded-md bg-primary px-3 text-sm font-medium text-primary-foreground hover:bg-primary/90"
                        >
                            Register
                        </button>
                    </div>
                    {projectRegistrationError ? (
                        <p data-testid="project-registration-error" className="mb-3 text-sm text-destructive">
                            {projectRegistrationError}
                        </p>
                    ) : null}
                    <ul data-testid="project-registry-list" className="space-y-2">
                        {projects.length === 0 ? (
                            <li className="rounded-md border border-dashed border-border px-3 py-2 text-sm text-muted-foreground">
                                No projects registered yet.
                            </li>
                        ) : (
                            projects.map((project) => {
                                const isActive = project.directoryPath === activeProjectPath
                                return (
                                    <li key={project.directoryPath} className="flex items-center justify-between rounded-md border border-border px-3 py-2">
                                        <span className="truncate text-sm">{project.directoryPath}</span>
                                        <button
                                            type="button"
                                            onClick={() => setActiveProjectPath(project.directoryPath)}
                                            className="ml-3 rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                                        >
                                            {isActive ? "Active" : "Set active"}
                                        </button>
                                    </li>
                                )
                            })
                        )}
                    </ul>
                </div>
            </div>
        </section>
    )
}
