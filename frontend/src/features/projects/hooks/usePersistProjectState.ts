import { useCallback } from 'react'

import { getActiveEnvironmentId } from '@/lib/activeEnvironment'
import { normalizeProjectPath } from '@/lib/projectPaths'
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

const usedAt = (at: string | null | undefined) => Date.parse(at ?? '') || 0

/**
 * Saves a project's state; its answer shows only while the environment it was saved in is shown. An answer
 * that lands once a switch away and back has loaded the projects anew, or that answers the app's own choice
 * of a chat, such as a project's latest once its list loads, leaves whichever chat is open by then, such as
 * a refused chat opened again in its place, and any newer use or pin; it updates only a project still listed,
 * since a lone entry in a list not yet loaded would stop that list's load.
 */
async function sendProjectState(projectPath: string, patch: PersistProjectStatePatch, upsert: UpsertProjectRegistryEntry) {
  const environmentId = getActiveEnvironmentId()
  const session = homeSessionStart()
  const byItself = opensByItself()
  const project = toHydratedProjectRecord(await updateProjectStateValidated({ project_path: projectPath, ...patch }))
  if (getActiveEnvironmentId() !== environmentId) return
  if (!byItself && homeSessionStart() === session) return upsert(project)
  const stored = useStore.getState().projectRegistry[normalizeProjectPath(project.directoryPath)]
  if (!stored) return
  upsert({
    ...project,
    activeConversationId: null,
    lastAccessedAt: usedAt(stored.lastAccessedAt) > usedAt(project.lastAccessedAt) ? stored.lastAccessedAt ?? null : project.lastAccessedAt,
    isFavorite: patch.is_favorite === undefined ? stored.isFavorite : project.isFavorite,
  })
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
