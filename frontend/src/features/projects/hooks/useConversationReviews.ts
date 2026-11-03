import { useCallback, useState } from 'react'
import {
    reviewFlowRunRequestValidated,
    reviewProposedPlanValidated,
    type ConversationBatch,
    type FlowRunRequestResponse,
    type ProposedPlanArtifactResponse,
} from '@/lib/workspaceClient'
import { useDialogController } from '@/components/app/dialog-controller'
type UseConversationReviewsArgs = {
    activeConversationId: string | null
    activeProjectPath: string | null
    applyConversationBatch: (
        projectPath: string,
        conversationId: string,
        batch: ConversationBatch,
        kind: 'live' | 'mutation',
        source?: string,
    ) => void
    formatErrorMessage: (error: unknown, fallback: string) => string
    model: string
    setPanelError: (message: string | null) => void
}

export function useConversationReviews({
    activeConversationId,
    activeProjectPath,
    applyConversationBatch,
    formatErrorMessage,
    model,
    setPanelError,
}: UseConversationReviewsArgs) {
    const { prompt } = useDialogController()
    const [pendingFlowRunRequestId, setPendingFlowRunRequestId] = useState<string | null>(null)
    const [pendingProposedPlanId, setPendingProposedPlanId] = useState<string | null>(null)

    const onReviewFlowRunRequest = useCallback(async (
        flowRunRequest: FlowRunRequestResponse,
        disposition: 'approved' | 'rejected',
    ) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }

        const reviewMessage = disposition === 'approved'
            ? 'Approved for launch.'
            : await prompt({
                title: 'Reject flow run request',
                description: 'Describe why this flow run request should be rejected.',
                label: 'Review feedback',
                confirmLabel: 'Reject request',
                cancelLabel: 'Cancel',
                multiline: true,
                requireInput: true,
            }) || ''

        if (!reviewMessage) {
            return
        }

        setPendingFlowRunRequestId(flowRunRequest.id)
        setPanelError(null)
        try {
            const ack = await reviewFlowRunRequestValidated(activeConversationId, flowRunRequest.id, {
                project_path: activeProjectPath,
                disposition,
                message: reviewMessage,
                model: model.trim() || null,
            })
            applyConversationBatch(activeProjectPath, activeConversationId, ack.changes, 'mutation', 'flow-run-request-review')
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to review the flow run request.')
            setPanelError(message)
        } finally {
            setPendingFlowRunRequestId(null)
        }
    }, [
        activeConversationId,
        activeProjectPath,
            applyConversationBatch,
        formatErrorMessage,
        model,
        prompt,
        setPanelError,
    ])

    const onReviewProposedPlan = useCallback(async (
        proposedPlan: ProposedPlanArtifactResponse,
        disposition: 'approved' | 'rejected',
        reviewNote?: string | null,
    ) => {
        if (!activeProjectPath || !activeConversationId) {
            return
        }
        setPendingProposedPlanId(proposedPlan.id)
        setPanelError(null)
        try {
            const ack = await reviewProposedPlanValidated(activeConversationId, proposedPlan.id, {
                project_path: activeProjectPath,
                disposition,
                review_note: reviewNote?.trim() || null,
            })
            applyConversationBatch(activeProjectPath, activeConversationId, ack.changes, 'mutation', 'proposed-plan-review')
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to review the proposed plan.')
            setPanelError(message)
        } finally {
            setPendingProposedPlanId(null)
        }
    }, [
        activeConversationId,
        activeProjectPath,
            applyConversationBatch,
        formatErrorMessage,
        setPanelError,
    ])

    return {
        onReviewFlowRunRequest,
        onReviewProposedPlan,
        pendingFlowRunRequestId,
        pendingProposedPlanId,
    }
}
