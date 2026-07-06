import {
    ApiHttpError,
    fetchPipelineJournalValidated,
    fetchPipelineTranscriptValidated,
    fetchPipelineStatusValidated,
} from '@/lib/attractorClient'

export { ApiHttpError }

export const loadSelectedRunStatus = fetchPipelineStatusValidated
export const loadSelectedRunJournal = fetchPipelineJournalValidated
export const loadSelectedRunTranscript = fetchPipelineTranscriptValidated
