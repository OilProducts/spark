import { useCallback, useState } from 'react'
import {
    reviewFlowRunRequestValidated,
    reviewProposedPlanValidated,
    type ConversationSnapshotResponse,
    type FlowRunRequestResponse,
    type ProposedPlanArtifactResponse,
} from '@/lib/workspaceClient'
import { useDialogController } from '@/components/app/dialog-controller'
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
    }, [
        activeConversationId,
        activeProjectPath,
        appendLocalProjectEvent,
        applyConversationSnapshot,
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
            const snapshot = await reviewProposedPlanValidated(activeConversationId, proposedPlan.id, {
                project_path: activeProjectPath,
                disposition,
                review_note: reviewNote?.trim() || null,
            })
            applyConversationSnapshot(activeProjectPath, snapshot, 'proposed-plan-review')
        } catch (error) {
            const message = formatErrorMessage(error, 'Unable to review the proposed plan.')
            setPanelError(message)
            appendLocalProjectEvent(`Proposed plan review failed: ${message}`)
        } finally {
            setPendingProposedPlanId(null)
        }
    }, [
        activeConversationId,
        activeProjectPath,
        appendLocalProjectEvent,
        applyConversationSnapshot,
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
