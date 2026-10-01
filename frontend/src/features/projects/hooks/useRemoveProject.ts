import { useState } from 'react'

import { useDialogController } from '@/components/app/dialog-controller'
import { deleteProjectValidated } from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { buildRemoveProjectTransition } from '@/state/projectScopeTransitions'
import { requestNavigation } from '@/state/workspaceSlice'

import { lastUsedProjectPath, projectLabel } from '../model/projectChoices'
import { extractApiErrorMessage } from '../model/projectsHomeState'

/** Removes a project from Spark after confirming; the server refuses Home. */
export function useRemoveProject(projectPath: string) {
    const { confirm } = useDialogController()
    const [removeError, setRemoveError] = useState<string | null>(null)
    const [removing, setRemoving] = useState(false)

    const removeProject = async () => {
        const label = projectLabel(useStore.getState().projectRegistry, projectPath)
        const confirmed = await confirm({
            title: 'Remove project?',
            description: `Remove project "${label}" from Spark? This deletes its local threads, workflow history, and runs, but does not delete the project files.`,
            confirmLabel: 'Remove project',
            cancelLabel: 'Keep project',
            confirmVariant: 'destructive',
        })
        if (!confirmed) {
            return
        }
        // Unsaved or saving editors decide before anything is deleted.
        requestNavigation(() => { void remove() })
    }

    const remove = async () => {
        setRemoveError(null)
        setRemoving(true)
        try {
            await deleteProjectValidated(projectPath)
            const rest = Object.fromEntries(Object.entries(useStore.getState().projectRegistry).filter(([path]) => path !== projectPath))
            // Leaving was already allowed, so the removal is not asked about again.
            useStore.setState((state) => buildRemoveProjectTransition(state, projectPath, lastUsedProjectPath(rest)) ?? {})
        } catch (error) {
            setRemoveError(extractApiErrorMessage(error, 'Unable to remove the project.'))
        } finally {
            setRemoving(false)
        }
    }

    return { removeProject, removeError, removing }
}
