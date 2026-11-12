import { deleteFlowValidated, fetchFlowListValidated } from '@/lib/attractorClient'

export const deleteFlowCatalogEntry = deleteFlowValidated
export const loadFlowCatalog = fetchFlowListValidated

export { fetchWorkspaceFlowListValidated as loadFlowSummaries } from '@/lib/api/flowsApi'
