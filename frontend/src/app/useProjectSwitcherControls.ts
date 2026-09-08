import { useMemo, useRef, useState } from 'react'

import { useStore } from '@/store'
import {
    ApiHttpError,
    deleteProjectValidated,
    fetchProjectBrowseValidated,
    fetchProjectMetadataValidated,
    registerProjectValidated,
    type ProjectBrowseResponse,
    updateProjectStateValidated,
} from '@/lib/workspaceClient'
import { useDialogController } from '@/components/app/dialog-controller'
import { useProjectRegistryBootstrap } from '@/features/projects/hooks/useProjectRegistryBootstrap'
import {
    buildOrderedProjects,
    extractApiErrorMessage,
    formatProjectListLabel,
    resolveProjectPathValidation,
    toHydratedProjectRecord,
} from '@/features/projects/model/projectsHomeState'

const DEFAULT_WORKING_DIRECTORY = './test-app'

export function useProjectSwitcherControls() {
    const { confirm } = useDialogController()
    const viewMode = useStore((state) => state.viewMode)
    const hydrateProjectRegistry = useStore((state) => state.hydrateProjectRegistry)
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const projectRegistry = useStore((state) => state.projectRegistry)
    const recentProjectPaths = useStore((state) => state.recentProjectPaths)
    const registerProject = useStore((state) => state.registerProject)
    const removeProject = useStore((state) => state.removeProject)
    const setActiveProjectPath = useStore((state) => state.setActiveProjectPath)
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const projectRegistrationError = useStore((state) => state.projectRegistrationError)
    const setProjectRegistrationError = useStore((state) => state.setProjectRegistrationError)
    const clearProjectRegistrationError = useStore((state) => state.clearProjectRegistrationError)

    const [bootstrapError, setBootstrapError] = useState<string | null>(null)
    const [controllerError, setControllerError] = useState<string | null>(null)
    const [isProjectBrowserOpen, setProjectBrowserOpen] = useState(false)
    const [isProjectBrowserLoading, setProjectBrowserLoading] = useState(false)
    const [projectBrowserState, setProjectBrowserState] = useState<ProjectBrowseResponse | null>(null)
    const [projectBrowserError, setProjectBrowserError] = useState<string | null>(null)
    const projectBrowserRequestIdRef = useRef(0)

    const orderedProjects = useMemo(
        () => buildOrderedProjects(Object.values(projectRegistry), projectRegistry, recentProjectPaths),
        [projectRegistry, recentProjectPaths],
    )
    const shouldBootstrapRegistry = orderedProjects.length === 0 && viewMode !== 'home'

    useProjectRegistryBootstrap({
        hydrateProjectRegistry,
        enabled: shouldBootstrapRegistry,
        onError: setBootstrapError,
    })

    const persistProjectState = async (
        projectPath: string,
        patch: {
            last_accessed_at?: string | null
            active_conversation_id?: string | null
            is_favorite?: boolean | null
        },
    ) => {
        try {
            const project = await updateProjectStateValidated({
                project_path: projectPath,
                ...patch,
            })
            upsertProjectRegistryEntry(toHydratedProjectRecord(project))
        } catch {
            // Keep the shell responsive if the background state sync fails.
        }
    }

    const ensureProjectGitRepository = async (projectPath: string) => {
        try {
            await fetchProjectMetadataValidated(projectPath)
            clearProjectRegistrationError()
            return true
        } catch (error) {
            const fallback = 'Unable to verify project Git state.'
            const message = error instanceof ApiHttpError && error.detail
                ? error.detail
                : extractApiErrorMessage(error, fallback)
            setProjectRegistrationError(message)
            return false
        }
    }

    const registerProjectFromPath = async (rawProjectPath: string) => {
        setControllerError(null)
        const validation = resolveProjectPathValidation(rawProjectPath, projectRegistry)
        if (!validation.ok || !validation.normalizedPath) {
            setProjectRegistrationError(validation.error ?? 'Project directory path is required.')
            return false
        }

        const normalizedProjectPath = validation.normalizedPath
        const gitReady = await ensureProjectGitRepository(normalizedProjectPath)
        if (!gitReady) {
            return false
        }

        const optimisticResult = registerProject(normalizedProjectPath)
        if (!optimisticResult.ok) {
            setProjectRegistrationError(optimisticResult.error ?? 'Unable to register the project.')
            return false
        }

        try {
            const projectRecord = await registerProjectValidated(normalizedProjectPath)
            upsertProjectRegistryEntry(toHydratedProjectRecord(projectRecord))
            clearProjectRegistrationError()
            return true
        } catch (error) {
            useStore.setState((state) => {
                const nextProjectRegistry = { ...state.projectRegistry }
                const nextProjectSessionStates = { ...state.projectSessionsByPath }
                delete nextProjectRegistry[normalizedProjectPath]
                delete nextProjectSessionStates[normalizedProjectPath]
                const nextActiveProjectPath = state.activeProjectPath === normalizedProjectPath ? null : state.activeProjectPath
                return {
                    projectRegistry: nextProjectRegistry,
                    projectSessionsByPath: nextProjectSessionStates,
                    activeProjectPath: nextActiveProjectPath,
                    activeFlow: state.activeFlow,
                    workingDir: nextActiveProjectPath ? state.workingDir : DEFAULT_WORKING_DIRECTORY,
                }
            })
            setProjectRegistrationError(extractApiErrorMessage(error, 'Unable to register the project.'))
            return false
        }
    }

    const browseProjectDirectory = async (path?: string) => {
        const requestId = projectBrowserRequestIdRef.current + 1
        projectBrowserRequestIdRef.current = requestId
        setProjectBrowserLoading(true)
        setProjectBrowserError(null)
        try {
            const response = await fetchProjectBrowseValidated(path)
            if (projectBrowserRequestIdRef.current !== requestId) {
                return false
            }
            setProjectBrowserState(response)
            return true
        } catch (error) {
            if (projectBrowserRequestIdRef.current !== requestId) {
                return false
            }
            setProjectBrowserError(extractApiErrorMessage(error, 'Unable to browse project directories.'))
            return false
        } finally {
            if (projectBrowserRequestIdRef.current === requestId) {
                setProjectBrowserLoading(false)
            }
        }
    }

    const closeProjectBrowser = () => {
        projectBrowserRequestIdRef.current += 1
        setProjectBrowserOpen(false)
        setProjectBrowserLoading(false)
        setProjectBrowserState(null)
        setProjectBrowserError(null)
    }

    const onOpenProjectDirectoryChooser = async () => {
        clearProjectRegistrationError()
        setControllerError(null)
        setProjectBrowserOpen(true)
        setProjectBrowserState(null)
        await browseProjectDirectory()
    }

    const onBrowseProjectDirectory = (path?: string) => {
        clearProjectRegistrationError()
        void browseProjectDirectory(path)
    }

    const onSelectProjectBrowserDirectory = async () => {
        if (!projectBrowserState) {
            return
        }
        const didRegister = await registerProjectFromPath(projectBrowserState.current_path)
        if (didRegister) {
            closeProjectBrowser()
        }
    }

    const onActivateProject = async (projectPath: string) => {
        if (!projectPath || projectPath === activeProjectPath) {
            return
        }
        setControllerError(null)
        const gitReady = await ensureProjectGitRepository(projectPath)
        if (!gitReady) {
            return
        }
        clearProjectRegistrationError()
        setActiveProjectPath(projectPath)
        void persistProjectState(projectPath, {
            last_accessed_at: new Date().toISOString(),
        })
    }

    const onClearActiveProject = () => {
        setControllerError(null)
        clearProjectRegistrationError()
        setActiveProjectPath(null)
    }

    const onDeleteActiveProject = async () => {
        if (!activeProjectPath) {
            return
        }
        const projectLabel = formatProjectListLabel(activeProjectPath)
        const confirmed = await confirm({
            title: 'Remove project?',
            description: `Remove project "${projectLabel}" from Spark? This deletes its local threads, workflow history, and runs, but does not delete the project files.`,
            confirmLabel: 'Remove project',
            cancelLabel: 'Keep project',
            confirmVariant: 'destructive',
        })
        if (!confirmed) {
            return
        }

        clearProjectRegistrationError()
        setControllerError(null)
        try {
            await deleteProjectValidated(activeProjectPath)
            const fallbackProjectPath = orderedProjects.find(
                (project) => project.directoryPath !== activeProjectPath,
            )?.directoryPath || null
            removeProject(activeProjectPath, fallbackProjectPath)
        } catch (error) {
            setControllerError(extractApiErrorMessage(error, 'Unable to remove the project.'))
        }
    }

    return {
        activeProjectPath,
        clearProjectRegistrationError,
        isProjectBrowserLoading,
        isProjectBrowserOpen,
        orderedProjects,
        projectBrowserErrorMessage: projectBrowserError || projectRegistrationError,
        projectBrowserState,
        projectErrorMessage: projectRegistrationError || controllerError || bootstrapError,
        onActivateProject,
        onBrowseProjectDirectory,
        onClearActiveProject,
        onDeleteActiveProject,
        onOpenProjectDirectoryChooser,
        onSelectProjectBrowserDirectory,
        onSetProjectBrowserOpen: (nextOpen: boolean) => {
            if (!nextOpen) {
                closeProjectBrowser()
                return
            }
            setProjectBrowserOpen(true)
        },
    }
}
