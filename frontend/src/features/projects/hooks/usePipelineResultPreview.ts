import { useState } from 'react'
import { fetchPipelineResultValidated, type PipelineResultResponse } from '@/lib/attractorClient'

export function usePipelineResultPreview(runId: string): {
    result: PipelineResultResponse | null
    isLoading: boolean
    error: string | null
    viewResult: () => Promise<void>
} {
    const [result, setResult] = useState<PipelineResultResponse | null>(null)
    const [isLoading, setIsLoading] = useState(false)
    const [error, setError] = useState<string | null>(null)

    const viewResult = async () => {
        setIsLoading(true)
        setError(null)
        try {
            setResult(await fetchPipelineResultValidated(runId))
        } catch (err) {
            console.error(err)
            setResult(null)
            setError("Unable to load result.")
        } finally {
            setIsLoading(false)
        }
    }

    return { result, isLoading, error, viewResult }
}
