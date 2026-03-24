import { useState } from 'react'
import {
    approveSpecEditProposalValidated,
    rejectSpecEditProposalValidated,
    reviewExecutionCardValidated,
    reviewFlowRunRequestValidated,
    type ConversationSnapshotResponse,
    type ExecutionCardResponse,
    type FlowRunRequestResponse,
    type SpecEditProposalResponse,
} from '@/lib/workspaceClient'

type UseConversationReviewsArgs = {
    activeConversationId: string | null
    activeProjectPath: string | null
    appendLocalProjectEvent: (message: string) => void
    applyConversationSnapshot: (projectPath: string, snapshot: ConversationSnapshotResponse, source?: string) => void
    formatErrorMessage: (error: unknown, fallback: string) => string
    model: string
    setPanelError: (message: string | null) => void
}

export function useConversationReviews({
    activeConversationId,
    activeProjectPath,
    appendLocalProjectEvent,
    applyConversationSnapshot,
    formatErrorMessage,
    model,
    setPanelError,
}: UseConversationReviewsArgs) {
    const [pendingSpecProposalId, setPendingSpecProposalId] = useState<string | null>(null)
    const [pendingFlowRunRequestId, setPendingFlowRunRequestId] = useState<string | null>(null)
    const [pendingExecutionCardId, setPendingExecutionCardId] = useState<string | null>(null)

    const onApproveSpecEditProposal = async (proposal: SpecEditProposalResponse) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }
        if (!window.confirm('Approve these spec edits, commit them to git, and start execution planning?')) {
            return
        }

        setPendingSpecProposalId(proposal.id)
        setPanelError(null)
        try {
            const snapshot = await approveSpecEditProposalValidated(activeConversationId, proposal.id, {
                project_path: activeProjectPath,
                model: model.trim() || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'spec-approve')
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to approve the spec edit proposal.')
            setPanelError(message)
            appendLocalProjectEvent(`Spec edit approval failed: ${message}`)
        } finally {
            setPendingSpecProposalId(null)
        }
    }

    const onRejectSpecEditProposal = async (proposal: SpecEditProposalResponse) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        setPendingSpecProposalId(proposal.id)
        setPanelError(null)
        try {
            const snapshot = await rejectSpecEditProposalValidated(activeConversationId, proposal.id, {
                project_path: activeProjectPath,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'spec-reject')
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to reject the spec edit proposal.')
            setPanelError(message)
            appendLocalProjectEvent(`Spec edit rejection failed: ${message}`)
        } finally {
            setPendingSpecProposalId(null)
        }
    }

    const onReviewExecutionCard = async (
        executionCard: ExecutionCardResponse,
        disposition: 'approved' | 'rejected' | 'revision_requested',
    ) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        const reviewMessage = disposition === 'approved'
            ? 'Approved for dispatch.'
            : window.prompt(
                disposition === 'revision_requested'
                    ? 'Describe what should change before execution planning is regenerated.'
                    : 'Describe why this execution card should be rejected.',
                '',
            )?.trim() || ''

        if (!reviewMessage) {
            return
        }

        setPendingExecutionCardId(executionCard.id)
        setPanelError(null)
        try {
            const snapshot = await reviewExecutionCardValidated(activeConversationId, executionCard.id, {
                project_path: activeProjectPath,
                disposition,
                message: reviewMessage,
                model: model.trim() || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'execution-review')
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to review the execution card.')
            setPanelError(message)
            appendLocalProjectEvent(`Execution card review failed: ${message}`)
        } finally {
            setPendingExecutionCardId(null)
        }
    }

    const onReviewFlowRunRequest = async (
        flowRunRequest: FlowRunRequestResponse,
        disposition: 'approved' | 'rejected',
    ) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        const reviewMessage = disposition === 'approved'
            ? 'Approved for launch.'
            : window.prompt(
                'Describe why this flow run request should be rejected.',
                '',
            )?.trim() || ''

        if (!reviewMessage) {
            return
        }

        setPendingFlowRunRequestId(flowRunRequest.id)
        setPanelError(null)
        try {
            const snapshot = await reviewFlowRunRequestValidated(activeConversationId, flowRunRequest.id, {
                project_path: activeProjectPath,
                disposition,
                message: reviewMessage,
                model: model.trim() || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'flow-run-request-review')
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to review the flow run request.')
            setPanelError(message)
            appendLocalProjectEvent(`Flow run request review failed: ${message}`)
        } finally {
            setPendingFlowRunRequestId(null)
        }
    }

    return {
        onApproveSpecEditProposal,
        onRejectSpecEditProposal,
        onReviewExecutionCard,
        onReviewFlowRunRequest,
        pendingExecutionCardId,
        pendingFlowRunRequestId,
        pendingSpecProposalId,
    }
}
