import { useCallback } from 'react'

import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { updateProjectStateValidated } from '@/lib/workspaceClient'
import { useStore } from '@/store'

import { toHydratedProjectRecord } from '../model/projectsHomeState'

type UpsertProjectRegistryEntry = (project: ReturnType<typeof toHydratedProjectRecord>) => void

type PersistProjectStatePatch = {
  last_accessed_at?: string | null
  active_conversation_id?: string | null
  is_favorite?: boolean | null
}

/** Saves a project's state; its answer shows only while the environment it was saved in is shown. */
async function sendProjectState(projectPath: string, patch: PersistProjectStatePatch, upsert: UpsertProjectRegistryEntry) {
  const environmentId = getActiveEnvironmentId()
  const project = await updateProjectStateValidated({ project_path: projectPath, ...patch })
  if (getActiveEnvironmentId() === environmentId) upsert(toHydratedProjectRecord(project))
}

export function usePersistProjectState(upsertProjectRegistryEntry: UpsertProjectRegistryEntry) {
  return useCallback(async (projectPath: string, patch: PersistProjectStatePatch) => {
    try {
      await sendProjectState(projectPath, patch, upsertProjectRegistryEntry)
    } catch {
      // Keep the UI responsive if the background state sync fails.
    }
  }, [upsertProjectRegistryEntry])
}

/** Records that work just started in a project, so it becomes the last-used project. */
export async function markProjectUsed(projectPath: string) {
  try {
    await sendProjectState(projectPath, { last_accessed_at: new Date().toISOString() }, useStore.getState().upsertProjectRegistryEntry)
  } catch {
    // The last-used project is a convenience; a failed sync keeps the old one.
  }
}
