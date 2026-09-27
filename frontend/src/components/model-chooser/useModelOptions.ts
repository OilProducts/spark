import { useEffect, useState } from 'react'
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from '@/lib/api/projectsApi'
import { ApiHttpError } from '@/lib/api/shared'

type Discovery = { projectPath: string; payload?: ProjectChatModelsResponse; failed?: boolean }
type Entry = { value: Discovery | null; listeners: Set<() => void>; refresh: () => void }
const projects = new Map<string, Entry>()

export function useModelOptions(projectPath: string | null) {
    const [, render] = useState(0)
    useEffect(() => {
        if (!projectPath) return
        let entry = projects.get(projectPath)
        if (!entry) {
            let revision = 0
            const created: Entry = { value: null, listeners: new Set(), refresh: () => {
                const request = ++revision
                created.value = null
                created.listeners.forEach((notify) => notify())
                void fetchProjectChatModelsValidated(projectPath).then(
                    (payload) => ({ projectPath, payload }),
                    (error: unknown) => ({ projectPath, failed: true, payload: {
                        models: [], providers: { codex: { status: 'unavailable' as const,
                            error: error instanceof ApiHttpError && error.detail ? error.detail
                                : error instanceof Error ? error.message : 'Unable to discover Codex models.' } },
                    } }),
                ).then((value) => {
                    if (request !== revision) return
                    created.value = value
                    created.listeners.forEach((notify) => notify())
                })
            } }
            entry = created
            projects.set(projectPath, entry)
            window.addEventListener('spark:codex-connected', entry.refresh)
            entry.refresh()
        }
        const notify = () => render((version) => version + 1)
        entry.listeners.add(notify)
        notify()
        return () => {
            entry.listeners.delete(notify)
            // Keep the in-flight request across React StrictMode's effect replay.
            queueMicrotask(() => {
                if (entry.listeners.size) return
                window.removeEventListener('spark:codex-connected', entry.refresh)
                projects.delete(projectPath)
            })
        }
    }, [projectPath])
    return projectPath ? projects.get(projectPath)?.value ?? null : null
}
