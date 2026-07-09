import {
    fetchFlowListValidated,
    fetchFlowPayloadValidated,
    fetchPipelineArtifactPreviewValidated,
} from '@/lib/attractorClient'

export async function loadCatalogFlowContent(flowName: string): Promise<string> {
    const payload = await fetchFlowPayloadValidated(flowName)
    return payload.content
}

export async function loadRunSnapshotFlowContent(runId: string): Promise<string> {
    return fetchPipelineArtifactPreviewValidated(runId, 'artifacts/flow/flow-source.yaml')
}

export async function loadFlowCatalog(): Promise<string[]> {
    return fetchFlowListValidated()
}
