import { useEffect, useState } from 'react'
import { fetchProjectChatModelsValidated, type ProjectChatModelsResponse } from '@/lib/api/projectsApi'
import { ApiHttpError } from '@/lib/api/shared'

type Discovery = { projectPath: string | null; payload?: ProjectChatModelsResponse; failed?: boolean }
type Entry = { value: Discovery | null; listeners: Set<() => void>; refresh: (event?: Event) => void; load: (quiet: boolean) => void }
const projects = new Map<string, Entry>()

/** Refetches without clearing what is shown: a harness may offer values its sessions acknowledged since. */
export function refreshModelOptions(projectPath: string | null) {
    projects.get(projectPath ?? '')?.load(true)
}

export function useModelOptions(projectPath: string | null) {
    const scope = projectPath ?? ''
    const [, render] = useState(0)
    useEffect(() => {
        let entry = projects.get(scope)
        if (!entry) {
            let revision = 0
            const created: Entry = { value: null, listeners: new Set(), refresh: (event) => {
                // A harness check ended: its result replaces "checking" in place.
                if (event?.type === 'spark:harness-checked') return created.load(true)
                const detail = (event as CustomEvent | undefined)?.detail
                if (detail?.payload?.section && !['providers', 'llm_profiles', 'agents', 'codex', 'claude_code'].includes(detail.payload.section)) return
                created.load(false)
            }, load: (quiet) => {
                const request = ++revision
                if (!quiet) {
                    created.value = null
                    created.listeners.forEach((notify) => notify())
                }
                void fetchProjectChatModelsValidated(projectPath).then(
                    (payload) => ({ projectPath, payload }),
                    (error: unknown) => ({ projectPath, failed: true, payload: {
                        models: [], providers: { codex: { status: 'unavailable' as const, label: 'Codex',
                            error: error instanceof ApiHttpError && error.detail ? error.detail
                                : error instanceof Error ? error.message : 'Unable to discover Codex models.' } },
                    } }),
                ).then((value: Discovery) => {
                    if (request !== revision || (quiet && value.failed && created.value)) return
                    created.value = value
                    created.listeners.forEach((notify) => notify())
                })
            } }
            entry = created
            projects.set(scope, entry)
            window.addEventListener('spark:settings-live-event', entry.refresh)
            window.addEventListener('spark:harness-checked', entry.refresh)
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
                window.removeEventListener('spark:harness-checked', entry.refresh)
                projects.delete(scope)
            })
        }
    }, [projectPath, scope])
    return projects.get(scope)?.value ?? null
}
