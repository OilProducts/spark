import { useRef, useState } from 'react'

import { useStore } from '@/store'
import {
    ApiHttpError,
    fetchProjectBrowseValidated,
    fetchProjectMetadataValidated,
    registerProjectValidated,
    type ProjectBrowseResponse,
} from '@/lib/workspaceClient'
import {
    extractApiErrorMessage,
    resolveProjectPathValidation,
    toHydratedProjectRecord,
} from '../model/projectsHomeState'

/** Adds a project by browsing to its folder, then opens a chat composer in it. */
export function useAddProject() {
    const projectRegistry = useStore((state) => state.projectRegistry)
    const registerProject = useStore((state) => state.registerProject)
    const removeProject = useStore((state) => state.removeProject)
    const upsertProjectRegistryEntry = useStore((state) => state.upsertProjectRegistryEntry)
    const projectRegistrationError = useStore((state) => state.projectRegistrationError)
    const setProjectRegistrationError = useStore((state) => state.setProjectRegistrationError)
    const clearProjectRegistrationError = useStore((state) => state.clearProjectRegistrationError)

    const [isProjectBrowserOpen, setProjectBrowserOpen] = useState(false)
    const [isProjectBrowserLoading, setProjectBrowserLoading] = useState(false)
    const [projectBrowserState, setProjectBrowserState] = useState<ProjectBrowseResponse | null>(null)
    const [projectBrowserError, setProjectBrowserError] = useState<string | null>(null)
    const projectBrowserRequestIdRef = useRef(0)

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
            useStore.getState().setActiveProjectPath(normalizedProjectPath)
            return true
        } catch (error) {
            removeProject(normalizedProjectPath)
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

    return {
        isProjectBrowserLoading,
        isProjectBrowserOpen,
        projectBrowserErrorMessage: projectBrowserError || projectRegistrationError,
        projectBrowserState,
        onBrowseProjectDirectory,
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
