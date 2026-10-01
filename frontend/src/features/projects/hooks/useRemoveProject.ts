import { useState } from 'react'

import { useDialogController } from '@/components/app/dialog-controller'
import { deleteProjectValidated } from '@/lib/workspaceClient'
import { useStore } from '@/store'

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
        setRemoveError(null)
        setRemoving(true)
        try {
            await deleteProjectValidated(projectPath)
            const state = useStore.getState()
            const rest = Object.fromEntries(Object.entries(state.projectRegistry).filter(([path]) => path !== projectPath))
            state.removeProject(projectPath, lastUsedProjectPath(rest))
            useStore.getState().openProjectPage(null)
        } catch (error) {
            setRemoveError(extractApiErrorMessage(error, 'Unable to remove the project.'))
        } finally {
            setRemoving(false)
        }
    }

    return { removeProject, removeError, removing }
}
