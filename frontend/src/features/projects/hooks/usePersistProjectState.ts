import { useCallback } from 'react'

import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { updateProjectStateValidated } from '@/lib/workspaceClient'
import { homeSessionStart, opensByItself } from '@/state/homeSessionSlice'
import { useStore } from '@/store'

import { toHydratedProjectRecord } from '../model/projectsHomeState'

type UpsertProjectRegistryEntry = (project: ReturnType<typeof toHydratedProjectRecord>) => void

type PersistProjectStatePatch = {
  last_accessed_at?: string | null
  active_conversation_id?: string | null
  is_favorite?: boolean | null
}

/**
 * Saves a project's state; its answer shows only while the environment it was saved in is shown, and not
 * once a switch away and back has loaded its projects anew, so it can't move the chat opened since. The
 * answer to the app's own choice of a chat, such as a project's latest once its list loads, leaves whichever
 * chat is open by then, such as a refused chat opened again in its place.
 */
async function sendProjectState(projectPath: string, patch: PersistProjectStatePatch, upsert: UpsertProjectRegistryEntry) {
  const environmentId = getActiveEnvironmentId()
  const session = homeSessionStart()
  const byItself = opensByItself()
  const project = await updateProjectStateValidated({ project_path: projectPath, ...patch })
  if (getActiveEnvironmentId() !== environmentId || homeSessionStart() !== session) return
  upsert({ ...toHydratedProjectRecord(project), ...(byItself ? { activeConversationId: null } : {}) })
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
