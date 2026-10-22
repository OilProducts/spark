import { useCallback } from 'react'

import { ApiHttpError, fetchPipelineCancelValidated, fetchPipelineRetryValidated, fetchPipelineSteerValidated } from '@/lib/attractorClient'
import { useDialogController } from '@/components/app/dialog-controller'
import { useStore } from '@/store'

const logUnexpectedRunError = (error: unknown) => {
    if (error instanceof ApiHttpError) {
        return
    }
    console.error(error)
}

export type SteerResult = { delivered: boolean; text: string }

// Steers a running agent node of the viewed run. Rejections and errors come
// back as results so the caller can show them.
async function sendSteer(runId: string, nodeId: string, message: string): Promise<SteerResult> {
    try {
        const response = await fetchPipelineSteerValidated(runId, {
            message,
            target_run_id: runId,
            target_node_id: nodeId,
        })
        const delivered = response.status === 'delivered'
        const text = response.message ?? response.error ?? response.status
        return { delivered, text: delivered || !response.reason ? text : `${text} (${response.reason})` }
    } catch (err) {
        logUnexpectedRunError(err)
        return {
            delivered: false,
            text: err instanceof ApiHttpError
                ? `Unable to send message (HTTP ${err.status})${err.detail ? `: ${err.detail}` : ''}.`
                : 'Unable to send message.',
        }
    }
}

export function useRunActions() {
    const { alert, confirm } = useDialogController()

    const requestCancel = useCallback(async (runId: string, currentStatus: string) => {
        if (currentStatus !== 'running') {
            return
        }
        const confirmed = await confirm({
            title: 'Cancel run?',
            description: 'It will stop after the active node finishes.',
            confirmLabel: 'Cancel run',
            cancelLabel: 'Keep running',
            confirmVariant: 'destructive',
        })
        if (!confirmed) {
            return
        }
        const rollback = useStore.getState().optimisticallyPatchRun(runId, { status: 'cancel_requested' })
        try {
            await fetchPipelineCancelValidated(runId)
        } catch (err) {
            logUnexpectedRunError(err)
            rollback()
            await alert({
                title: 'Cancel failed',
                description: 'Failed to cancel run.',
            })
        }
    }, [alert, confirm])

    const requestRetry = useCallback(async (runId: string, currentStatus: string) => {
        if (currentStatus !== 'failed') {
            return
        }
        const confirmed = await confirm({
            title: 'Retry run?',
            description: 'It will resume this run from its checkpoint using the same run id.',
            confirmLabel: 'Retry run',
            cancelLabel: 'Keep failed',
        })
        if (!confirmed) {
            return
        }
        const rollback = useStore.getState().optimisticallyPatchRun(runId, { status: 'running', last_error: '' })
        try {
            await fetchPipelineRetryValidated(runId)
        } catch (err) {
            logUnexpectedRunError(err)
            rollback()
            await alert({
                title: 'Retry failed',
                description: 'Failed to retry run.',
            })
        }
    }, [alert, confirm])

    return {
        requestCancel,
        requestRetry,
        sendSteer,
    }
}
