import { fetchFlowListValidated } from '@/lib/attractorClient'

export async function loadFlowCatalog(): Promise<string[]> {
    return fetchFlowListValidated()
}
