import { useEffect, useState } from 'react'

import { fetchLlmProfiles } from '@/lib/api/llmProfilesApi'
import type { LlmProfileMetadata } from '@/lib/llmSuggestions'

type Store = {
    value: LlmProfileMetadata[]
    listeners: Set<() => void>
    refresh: () => void
}

let shared: Store | null = null

// Shared loader for the LLM profile catalog used by pickers and composer
// commands so every surface observes the same snapshot while requests load.
export function useLlmProfiles(): LlmProfileMetadata[] {
    const [, render] = useState(0)

    useEffect(() => {
        if (!shared) {
            let revision = 0
            const created: Store = {
                value: [],
                listeners: new Set(),
                refresh: () => {
                    const request = ++revision
                    void fetchLlmProfiles().then((profiles) => {
                        if (shared !== created || request !== revision) return
                        created.value = profiles
                        created.listeners.forEach((notify) => notify())
                    })
                },
            }
            shared = created
            window.addEventListener('spark:settings-live-event', created.refresh)
            created.refresh()
        }
        const store = shared
        const notify = () => render((version) => version + 1)
        store.listeners.add(notify)
        notify()
        return () => {
            store.listeners.delete(notify)
            // Keep the in-flight request across React StrictMode's effect replay.
            queueMicrotask(() => {
                if (store.listeners.size || shared !== store) return
                window.removeEventListener('spark:settings-live-event', store.refresh)
                shared = null
            })
        }
    }, [])

    return shared?.value ?? []
}
