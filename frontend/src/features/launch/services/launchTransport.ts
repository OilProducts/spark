import { fetchFlowListValidated } from '@/lib/attractorClient'

export async function loadFlowCatalog(projectPath: string | null): Promise<string[]> {
    return fetchFlowListValidated(projectPath)
}
