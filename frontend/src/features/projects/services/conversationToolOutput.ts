import { fetchConversationSegmentToolOutputValidated } from '@/lib/workspaceClient'

export async function loadConversationSegmentToolOutput(
    conversationId: string,
    segmentId: string,
    projectPath: string,
): Promise<string> {
    const payload = await fetchConversationSegmentToolOutputValidated(conversationId, segmentId, projectPath)
    return payload.output
}
