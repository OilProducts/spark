import { useEffect, useState } from 'react'
import { fetchLlmProfiles } from '@/lib/api/llmProfilesApi'
import type { LlmProfileMetadata } from '@/lib/llmSuggestions'

export function useLlmProfiles(): LlmProfileMetadata[] {
    const [llmProfiles, setLlmProfiles] = useState<LlmProfileMetadata[]>([])
    useEffect(() => {
        void fetchLlmProfiles().then(setLlmProfiles)
    }, [])
    return llmProfiles
}
