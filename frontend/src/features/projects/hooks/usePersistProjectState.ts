import { useCallback } from 'react'

import { updateProjectStateValidated } from '@/lib/workspaceClient'
import { useStore } from '@/store'

import { toHydratedProjectRecord } from '../model/projectsHomeState'

type UpsertProjectRegistryEntry = (project: ReturnType<typeof toHydratedProjectRecord>) => void

type PersistProjectStatePatch = {
  last_accessed_at?: string | null
  active_conversation_id?: string | null
  is_favorite?: boolean | null
}

export function usePersistProjectState(upsertProjectRegistryEntry: UpsertProjectRegistryEntry) {
  return useCallback(async (projectPath: string, patch: PersistProjectStatePatch) => {
    try {
      const project = await updateProjectStateValidated({
        project_path: projectPath,
        ...patch,
      })
      upsertProjectRegistryEntry(toHydratedProjectRecord(project))
    } catch {
      // Keep the UI responsive if the background state sync fails.
    }
  }, [upsertProjectRegistryEntry])
}

/** Records that work just started in a project, so it becomes the last-used project. */
export async function markProjectUsed(projectPath: string) {
  try {
    const project = await updateProjectStateValidated({
      project_path: projectPath,
      last_accessed_at: new Date().toISOString(),
    })
    useStore.getState().upsertProjectRegistryEntry(toHydratedProjectRecord(project))
  } catch {
    // The last-used project is a convenience; a failed sync keeps the old one.
  }
}
