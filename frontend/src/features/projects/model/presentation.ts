import type {
    ExecutionCardResponse,
    FlowLaunchResponse,
    FlowRunRequestResponse,
    SpecEditProposalResponse,
} from '@/lib/workspaceClient'
import type { ConversationTimelineToolCall } from '@/components/projects/types'

export type ProjectGitMetadata = {
    branch: string | null
    commit: string | null
}

export type SurfaceTone = 'neutral' | 'info' | 'success' | 'warning' | 'danger'

const SURFACE_TONE_CLASS_MAP: Record<SurfaceTone, string> = {
    neutral: 'bg-muted/50 text-muted-foreground',
    info: 'bg-sky-500/15 text-sky-700',
    success: 'bg-emerald-500/15 text-emerald-800',
    warning: 'bg-amber-500/15 text-amber-800',
    danger: 'bg-destructive/10 text-destructive',
}

export const PROPOSAL_DIFF_COLLAPSE_LINE_LIMIT = 12

export const getSurfaceToneClassName = (tone: SurfaceTone) => (
    `rounded px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${SURFACE_TONE_CLASS_MAP[tone]}`
)

export const getSpecEditStatusPresentation = (status: SpecEditProposalResponse['status']) => {
    if (status === 'applied') {
        return { label: 'Applied', tone: 'success' as const }
    }
    if (status === 'rejected') {
        return { label: 'Rejected', tone: 'danger' as const }
    }
    return { label: 'Pending review', tone: 'warning' as const }
}

export const getExecutionCardStatusPresentation = (status: ExecutionCardResponse['status']) => {
    if (status === 'approved') {
        return { label: 'Approved', tone: 'success' as const }
    }
    if (status === 'rejected') {
        return { label: 'Rejected', tone: 'danger' as const }
    }
    if (status === 'revision-requested') {
        return { label: 'Revision requested', tone: 'warning' as const }
    }
    return { label: 'Draft', tone: 'info' as const }
}

export const getFlowRunRequestStatusPresentation = (status: FlowRunRequestResponse['status']) => {
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

export const getFlowLaunchStatusPresentation = (status: FlowLaunchResponse['status']) => {
    if (status === 'launched') {
        return { label: 'Launched', tone: 'success' as const }
    }
    if (status === 'launch_failed') {
        return { label: 'Launch failed', tone: 'danger' as const }
    }
    return { label: 'Launching', tone: 'info' as const }
}

export const getToolCallStatusPresentation = (status: 'running' | 'completed' | 'failed') => {
    if (status === 'running') {
        return { label: 'Running', tone: 'info' as const }
    }
    if (status === 'failed') {
        return { label: 'Failed', tone: 'danger' as const }
    }
    return { label: 'Completed', tone: 'success' as const }
}

export const summarizeToolCallDetail = (toolCall: ConversationTimelineToolCall): string | null => {
    if (toolCall.command) {
        return toolCall.command
    }
    if (toolCall.filePaths.length > 0) {
        return toolCall.filePaths[0]
    }
    if (toolCall.output) {
        return toolCall.output.split(/\r?\n/, 1)[0]?.trim() || null
    }
    return null
}

export const parseThinkingSummaryContent = (content: string): { heading: string | null; details: string } => {
    const trimmed = content.trim()
    const headingMatch = trimmed.match(/^\*\*(.+?)\*\*(?:\s*[\r\n]+|\s+|$)/)
    if (!headingMatch) {
        return {
            heading: trimmed.length > 0 ? trimmed : null,
            details: '',
        }
    }
    const heading = headingMatch[1]?.trim() || null
    const details = trimmed.slice(headingMatch[0].length).trim()
    return { heading, details }
}

export const buildProposalDiffLines = (change: SpecEditProposalResponse['changes'][number]) => ([
    ...change.before.split('\n').map((line) => ({ type: 'removed' as const, text: line })),
    ...change.after.split('\n').map((line) => ({ type: 'added' as const, text: line })),
])

export const buildProposalChangeKey = (proposalId: string, changePath: string, index: number) => (
    `${proposalId}:${changePath}:${index}`
)
