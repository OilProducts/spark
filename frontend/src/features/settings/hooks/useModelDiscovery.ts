import { useEffect, useState } from "react"
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from "@/lib/api/projectsApi"

export function useModelDiscovery(activeProjectPath: string | null) {
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
    }, [activeProjectPath])

    return discovery?.projectPath === activeProjectPath ? discovery : null
}
