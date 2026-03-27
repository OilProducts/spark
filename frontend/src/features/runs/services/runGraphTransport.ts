import { fetchPipelineGraphPreviewValidated, type PipelineGraphPreviewResponse } from '@/lib/attractorClient'

export type RunGraphPreviewResponse = PipelineGraphPreviewResponse

export const loadRunGraphPreview = fetchPipelineGraphPreviewValidated
