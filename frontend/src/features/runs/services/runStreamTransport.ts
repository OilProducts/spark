import {
    ApiHttpError,
    fetchPipelineJournalValidated,
    fetchPipelineStatusValidated,
} from '@/lib/attractorClient'

export { ApiHttpError }

export const loadSelectedRunStatus = fetchPipelineStatusValidated
export const loadSelectedRunJournal = fetchPipelineJournalValidated

export {
    fetchRunSegmentsValidated as loadRunTranscript,
    parseRunTranscriptSegment as parseLiveRunTranscriptSegment,
} from '@/lib/api/attractorApi'
