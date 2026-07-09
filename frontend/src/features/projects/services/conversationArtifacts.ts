import { fetchPipelineResultValidated } from '@/lib/attractorClient'
import { fetchConversationSegmentToolOutputValidated } from '@/lib/workspaceClient'

export const loadConversationSegmentToolOutput = fetchConversationSegmentToolOutputValidated
export const loadPipelineResult = fetchPipelineResultValidated
