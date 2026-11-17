import { useEffect, useState } from 'react'

import { loadFlowCatalog } from '../services/launchTransport'

export interface FlowCatalogState {
    flows: string[]
    isLoaded: boolean
}

/**
 * A project's flow names. Loads while `reloadKey` is set and again whenever it changes, so a flow
 * created since the last load is found; the last list stays shown while the next one loads.
 */
export function useFlowCatalog(projectPath: string | null, reloadKey: string | null): FlowCatalogState {
    const [catalog, setCatalog] = useState<{ projectPath: string | null; flows: string[] } | null>(null)
    const isLoaded = catalog !== null && catalog.projectPath === projectPath

    useEffect(() => {
        if (reloadKey === null) {
            return
        }
        let cancelled = false
        loadFlowCatalog(projectPath)
            .then((flows) => {
                if (!cancelled) {
                    setCatalog({ projectPath, flows })
                }
            })
            .catch(() => {
                // A failed reload keeps the list it had for this project.
                if (!cancelled) {
                    setCatalog((current) => (current?.projectPath === projectPath ? current : { projectPath, flows: [] }))
                }
            })
        return () => {
            cancelled = true
        }
    }, [reloadKey, projectPath])

    return { flows: isLoaded ? catalog.flows : [], isLoaded }
}
