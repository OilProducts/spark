import type {
    ProjectFlowLaunch,
    ProjectFlowRunRequest,
    ProjectProposedPlan,
    ProjectRunRecovery,
} from './types'

// Transcript-facing presentation (tones, tool-call/thinking helpers) lives in
// the shared segment rows module; re-exported here so existing imports hold.
export {
    getToolCallStatusPresentation,
    parseThinkingSummaryContent,
    summarizeToolCallDetail,
} from '@/components/app/transcript/SegmentRows'
export type { SurfaceTone } from '@/components/app/transcript/SegmentRows'

export type ProjectGitMetadata = {
    branch: string | null
    commit: string | null
}

export const PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT = 12

export const getFlowRunRequestStatusPresentation = (status: ProjectFlowRunRequest['status']) => {
    if (status === 'launched') {
        return { label: 'Launched', tone: 'success' as const }
    }
    if (status === 'approved') {
        return { label: 'Approved', tone: 'info' as const }
    }
    if (status === 'rejected') {
        return { label: 'Rejected', tone: 'danger' as const }
    }
    if (status === 'launch_failed') {
        return { label: 'Launch failed', tone: 'danger' as const }
    }
    return { label: 'Pending review', tone: 'warning' as const }
}

export const getFlowLaunchStatusPresentation = (status: ProjectFlowLaunch['status']) => {
    if (status === 'launched') {
        return { label: 'Launched', tone: 'success' as const }
    }
    if (status === 'launch_failed') {
        return { label: 'Launch failed', tone: 'danger' as const }
    }
    return { label: 'Launching', tone: 'info' as const }
}

/** A run recovery's operation and source run, as "Continue <run> from <node>" or "Retry <run>". */
export const describeRunRecovery = (recovery: ProjectRunRecovery) => (
    recovery.operation === 'continue'
        ? `Continue ${recovery.source_run_id}${recovery.start_node ? ` from ${recovery.start_node}` : ''}`
        : `Retry ${recovery.source_run_id}`
)

export const getRunRecoveryStatusPresentation = (status: ProjectRunRecovery['status']) => {
    if (status === 'pending') {
        return { label: 'Starting', tone: 'info' as const }
    }
    if (status === 'failed') {
        return { label: 'Failed', tone: 'danger' as const }
    }
    if (status === 'started') {
        return { label: 'Started', tone: 'success' as const }
    }
    return { label: status, tone: 'neutral' as const }
}

export const getProposedPlanStatusPresentation = (status: ProjectProposedPlan['status']) => {
    if (status === 'approved') {
        return { label: 'Approved', tone: 'success' as const }
    }
    if (status === 'rejected') {
        return { label: 'Rejected', tone: 'danger' as const }
    }
    if (status === 'launch_failed') {
        return { label: 'Launch failed', tone: 'danger' as const }
    }
    return { label: 'Pending review', tone: 'warning' as const }
}
