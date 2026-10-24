import { useEffect, useState } from 'react'

import { loadFlowCatalog } from '../services/launchTransport'

export interface FlowCatalogState {
    flows: string[]
    isLoaded: boolean
}

export function useFlowCatalog(projectPath: string | null, enabled = true): FlowCatalogState {
    const [catalog, setCatalog] = useState<{ projectPath: string | null; flows: string[] } | null>(null)
    const isLoaded = catalog !== null && catalog.projectPath === projectPath

    useEffect(() => {
        if (!enabled || isLoaded) {
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
                if (!cancelled) {
                    setCatalog({ projectPath, flows: [] })
                }
            })
        return () => {
            cancelled = true
        }
    }, [enabled, isLoaded, projectPath])

    return { flows: isLoaded ? catalog.flows : [], isLoaded }
}
