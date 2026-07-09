import { useEffect, useState } from 'react'

import { fetchLlmProfiles } from '@/lib/api/llmProfilesApi'
import type { LlmProfileMetadata } from '@/lib/llmSuggestions'

// Shared loader for the LLM profile catalog used by pickers across the
// editor, canvas, and settings surfaces.
export function useLlmProfiles(): LlmProfileMetadata[] {
    const [llmProfiles, setLlmProfiles] = useState<LlmProfileMetadata[]>([])

    useEffect(() => {
        let cancelled = false
        void fetchLlmProfiles().then((profiles) => {
            if (!cancelled) {
                setLlmProfiles(profiles)
            }
        })
        return () => {
            cancelled = true
        }
    }, [])

    return llmProfiles
}
