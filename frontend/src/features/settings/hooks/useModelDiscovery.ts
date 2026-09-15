import { useEffect, useState } from "react"
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from "@/lib/api/projectsApi"

export function useModelDiscovery(activeProjectPath: string | null) {
    const [connectionRevision, setConnectionRevision] = useState(0)
    useEffect(() => {
        const refresh = () => setConnectionRevision((revision) => revision + 1)
        window.addEventListener('spark:codex-connected', refresh)
        return () => window.removeEventListener('spark:codex-connected', refresh)
    }, [])
    const [discovery, setDiscovery] = useState<{
        projectPath: string
        payload?: ProjectChatModelsResponse
        failed?: boolean
    } | null>(null)

    useEffect(() => {
        if (!activeProjectPath) return
        let cancelled = false
        setDiscovery(null)
        void fetchProjectChatModelsValidated(activeProjectPath).then(
            (payload) => {
                if (!cancelled) setDiscovery({ projectPath: activeProjectPath, payload })
            },
            () => {
                if (!cancelled) setDiscovery({ projectPath: activeProjectPath, failed: true })
            },
        )
        return () => { cancelled = true }
    }, [activeProjectPath, connectionRevision])

    return discovery?.projectPath === activeProjectPath ? discovery : null
}
