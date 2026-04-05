import { useEffect, type Dispatch, type SetStateAction } from 'react'

import { ApiHttpError, fetchProjectMetadataValidated } from '@/lib/workspaceClient'
import { useStore } from '@/store'

import type { ProjectGitMetadata } from '../model/presentation'
import { asProjectGitMetadataField, EMPTY_PROJECT_GIT_METADATA } from '../model/projectsHomeState'

type UseProjectGitMetadataArgs = {
    projectPaths: string[]
    setProjectRegistrationError: (value: string | null) => void
}

export function useProjectGitMetadata({
    projectPaths,
    setProjectRegistrationError,
}: UseProjectGitMetadataArgs) {
    const projectGitMetadata = useStore((state) => state.homeProjectGitMetadataByPath)
    const setHomeProjectGitMetadata = useStore((state) => state.setHomeProjectGitMetadata)

    useEffect(() => {
        const projectPathsToFetch = projectPaths.filter((projectPath) => !(projectPath in projectGitMetadata))
        if (projectPathsToFetch.length === 0) {
            return
        }

        let isCancelled = false
        const loadBranches = async () => {
            const entries = await Promise.all(
                projectPathsToFetch.map(async (projectPath) => {
                    try {
                        const metadata = await fetchProjectMetadataValidated(projectPath)
                        return [
                            projectPath,
                            {
                                branch: asProjectGitMetadataField(metadata.branch),
                                commit: asProjectGitMetadataField(metadata.commit),
                            },
                        ] as const
                    } catch {
                        return [projectPath, { ...EMPTY_PROJECT_GIT_METADATA }] as const
                    }
                }),
            )

            if (isCancelled) {
                return
            }

            entries.forEach(([projectPath, metadata]) => {
                setHomeProjectGitMetadata(projectPath, metadata)
            })
        }

        void loadBranches()
        return () => {
            isCancelled = true
        }
    }, [projectGitMetadata, projectPaths, setHomeProjectGitMetadata])

    const fetchProjectGitMetadata = async (
        projectPath: string,
    ): Promise<{ metadata: ProjectGitMetadata; error?: string }> => {
        try {
            const payload = await fetchProjectMetadataValidated(projectPath)
            return {
                metadata: {
                    branch: asProjectGitMetadataField(payload.branch),
                    commit: asProjectGitMetadataField(payload.commit),
                },
            }
        } catch (err) {
            let message = 'Unable to verify project Git state.'
            if (err instanceof ApiHttpError && err.detail) {
                message = err.detail
            }
            return { metadata: { ...EMPTY_PROJECT_GIT_METADATA }, error: message }
        }
    }

    const ensureProjectGitRepository = async (projectPath: string): Promise<ProjectGitMetadata | null> => {
        const { metadata, error } = await fetchProjectGitMetadata(projectPath)
        setHomeProjectGitMetadata(projectPath, metadata)
        if (error) {
            setProjectRegistrationError(error)
            return null
        }
        if (!metadata.branch && !metadata.commit) {
            setProjectRegistrationError('Project directory must be a Git repository.')
            return null
        }
        return metadata
    }

    const setProjectGitMetadata: Dispatch<SetStateAction<Record<string, ProjectGitMetadata>>> = (next) => {
        const current = useStore.getState().homeProjectGitMetadataByPath
        const resolved = typeof next === 'function'
            ? next(current)
            : next
        useStore.setState(() => ({
            homeProjectGitMetadataByPath: resolved,
        }))
    }

    return {
        ensureProjectGitRepository,
        projectGitMetadata,
        setProjectGitMetadata,
    }
}
