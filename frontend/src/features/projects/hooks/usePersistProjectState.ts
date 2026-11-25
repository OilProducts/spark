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
const projectsListed = () => Object.keys(useStore.getState().projectRegistry).length > 0

/**
 * Runs `apply` once the shown projects have loaded, unless another environment shows first. An entry
 * added to a list still loading would stop that load, and the load would replace the entry.
 */
// ponytail: an environment with no projects keeps the wait, a check per store update, until the next switch.
function whenProjectsListed(apply: () => void) {
  if (projectsListed()) return apply()
  const session = homeSessionStart()
  const unsubscribe = useStore.subscribe(() => {
    if (homeSessionStart() !== session) return unsubscribe()
    if (!projectsListed()) return
    unsubscribe()
    apply()
  })
}

/**
 * Saves a project's state; its answer shows only while the environment it was saved in is shown. An answer
 * that lands once a switch away and back has started loading the projects anew, or that answers the app's own
 * choice of a chat, such as a project's latest once its list loads, waits for that list, then updates only a
 * project it lists, leaving whichever chat is open by then, such as a refused chat opened again in its place,
 * any newer use or pin, and the rest of the project as that list has it, such as its execution profile.
 */
async function sendProjectState(projectPath: string, patch: PersistProjectStatePatch, upsert: UpsertProjectRegistryEntry) {
  const environmentId = getActiveEnvironmentId()
  const session = homeSessionStart()
  const byItself = opensByItself()
  const project = toHydratedProjectRecord(await updateProjectStateValidated({ project_path: projectPath, ...patch }))
  if (getActiveEnvironmentId() !== environmentId) return
  if (!byItself && homeSessionStart() === session) return upsert(project)
  whenProjectsListed(() => {
    const stored = useStore.getState().projectRegistry[normalizeProjectPath(project.directoryPath)]
    if (!stored) return
    // The answer predates the list loaded since: only what this save set comes from it.
    upsert({
      ...stored,
      lastAccessedAt: usedAt(project.lastAccessedAt) > usedAt(stored.lastAccessedAt) ? project.lastAccessedAt : stored.lastAccessedAt,
      isFavorite: patch.is_favorite === undefined ? stored.isFavorite : project.isFavorite,
    })
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
