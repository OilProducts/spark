export interface SpecEditProposalChange {
    path: string
    before: string
    after: string
}

export interface SpecEditProposalPreview {
    id: string
    createdAt: string
    summary: string
    changes: SpecEditProposalChange[]
    status?: 'pending' | 'applied'
    isDemo?: boolean
}

export type ProjectSpecEditProposalMap = Record<string, SpecEditProposalPreview>

export const getProjectSpecEditProposal = (
    proposals: ProjectSpecEditProposalMap,
    projectPath: string | null
): SpecEditProposalPreview | null => {
    if (!projectPath) {
        return null
    }
    return proposals[projectPath] || null
}

export const upsertProjectSpecEditProposal = (
    proposals: ProjectSpecEditProposalMap,
    projectPath: string,
    proposal: SpecEditProposalPreview
): ProjectSpecEditProposalMap => ({
    ...proposals,
    [projectPath]: proposal,
})

export const updateProjectSpecEditProposal = (
    proposals: ProjectSpecEditProposalMap,
    projectPath: string,
    updater: (proposal: SpecEditProposalPreview) => SpecEditProposalPreview
): ProjectSpecEditProposalMap => {
    const current = proposals[projectPath]
    if (!current) {
        return proposals
    }
    return {
        ...proposals,
        [projectPath]: updater(current),
    }
}

export const clearProjectSpecEditProposal = (
    proposals: ProjectSpecEditProposalMap,
    projectPath: string
): ProjectSpecEditProposalMap => {
    const next = { ...proposals }
    delete next[projectPath]
    return next
}
