import { useEffect, useState } from 'react'

import { loadFlowCatalog } from '../services/launchTransport'

export interface FlowCatalogState {
    flows: string[]
    isLoaded: boolean
}

export function useFlowCatalog(enabled = true): FlowCatalogState {
    const [flows, setFlows] = useState<string[]>([])
    const [isLoaded, setIsLoaded] = useState(false)

    useEffect(() => {
        if (!enabled || isLoaded) {
            return
        }
        let cancelled = false
        loadFlowCatalog()
            .then((catalog) => {
                if (!cancelled) {
                    setFlows(catalog)
                    setIsLoaded(true)
                }
            })
            .catch(() => {
                if (!cancelled) {
                    setFlows([])
                    setIsLoaded(true)
                }
            })
        return () => {
            cancelled = true
        }
    }, [enabled, isLoaded])

    return { flows, isLoaded }
}
