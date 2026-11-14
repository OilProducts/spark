import {
    fetchFlowPayloadValidated,
    fetchPreviewValidated,
    type PreviewResponsePayload,
} from '@/lib/attractorClient'
import { fetchWorkspaceFlowFilesValidated } from '@/lib/api/flowsApi'

export type EditorPreviewResponse = PreviewResponsePayload

export const loadEditorFlowPayload = fetchFlowPayloadValidated
export const loadEditorPreview = fetchPreviewValidated
export const loadEditorFlowFiles = fetchWorkspaceFlowFilesValidated
