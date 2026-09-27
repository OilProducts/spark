import { useEffect, useState } from 'react'
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from '@/lib/api/projectsApi'
import { ApiHttpError } from '@/lib/api/shared'

type Discovery = { projectPath: string | null; payload?: ProjectChatModelsResponse; failed?: boolean }
type Entry = { value: Discovery | null; listeners: Set<() => void>; refresh: (event?: Event) => void }
const projects = new Map<string, Entry>()

export function useModelOptions(projectPath: string | null) {
    const scope = projectPath ?? ''
    const [, render] = useState(0)
    useEffect(() => {
        let entry = projects.get(scope)
        if (!entry) {
            let revision = 0
            const created: Entry = { value: null, listeners: new Set(), refresh: (event) => {
                const detail = (event as CustomEvent | undefined)?.detail
                if (detail?.payload?.section && !['providers', 'llm_profiles', 'agents', 'codex'].includes(detail.payload.section)) return
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
            projects.set(scope, entry)
            window.addEventListener('spark:settings-live-event', entry.refresh)
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
                window.removeEventListener('spark:settings-live-event', entry.refresh)
                projects.delete(scope)
            })
        }
    }, [projectPath, scope])
    return projects.get(scope)?.value ?? null
}
