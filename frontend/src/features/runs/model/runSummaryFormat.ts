import type { RunRecord } from './shared'

// Pure display formatters shared by the run header bar and the Details
// inspector tab (moved from the retired RunSummaryCard).

export function formatTokenCount(value: number | null | undefined): string {
    return typeof value === 'number' ? value.toLocaleString() : '—'
}

export function formatEstimatedCost(amount: number | null | undefined, currency: string): string {
    if (typeof amount !== 'number') {
        return 'Unpriced'
    }
    const maximumFractionDigits = amount >= 1 ? 2 : 6
    const minimumFractionDigits = amount >= 1 ? 2 : 4
    return new Intl.NumberFormat('en-US', {
        style: 'currency',
        currency,
        minimumFractionDigits,
        maximumFractionDigits,
    }).format(amount)
}

export function formatEstimatedModelCostLabel(run: RunRecord): string {
    const estimatedCost = run.estimated_model_cost
    if (!estimatedCost) {
        return '—'
    }
    if (estimatedCost.status === 'unpriced') {
        return 'Unpriced model usage'
    }
    const prefix = formatEstimatedCost(estimatedCost.amount, estimatedCost.currency)
    if (estimatedCost.status === 'partial_unpriced') {
        return `${prefix} (partial)`
    }
    return prefix
}

export function formatEstimatedModelCostNote(run: RunRecord): string | null {
    const estimatedCost = run.estimated_model_cost
    if (!estimatedCost || estimatedCost.unpriced_models.length === 0) {
        return null
    }
    const label = estimatedCost.status === 'partial_unpriced'
        ? 'Unpriced models excluded from the subtotal'
        : 'Unpriced models'
    return `${label}: ${estimatedCost.unpriced_models.join(', ')}`
}

function compactPath(value: string): string {
    return value.trim().replace(/\/+$/, '') || value.trim()
}

export function shouldShowWorkingDirectoryDifference(run: RunRecord, activeProjectPath: string | null): boolean {
    const workingDirectory = run.working_directory ? compactPath(run.working_directory) : ''
    const projectPath = run.project_path || activeProjectPath || ''
    const compactProjectPath = projectPath ? compactPath(projectPath) : ''
    return Boolean(workingDirectory && compactProjectPath && workingDirectory !== compactProjectPath)
}

export function formatGitRef(run: RunRecord): string | null {
    const branch = run.git_branch?.trim()
    const commit = run.git_commit?.trim()
    if (branch && commit) {
        return `${branch} @ ${commit.slice(0, 7)}`
    }
    return branch || (commit ? commit.slice(0, 7) : null)
}

export function formatLineage(run: RunRecord): string | null {
    const lineageParts: string[] = []
    if (run.continued_from_run_id) {
        lineageParts.push(`Continued from ${run.continued_from_run_id}${run.continued_from_node ? ` @ ${run.continued_from_node}` : ''}`)
    }
    if (run.parent_run_id) {
        lineageParts.push(`Parent ${run.parent_run_id}${run.parent_node_id ? ` @ ${run.parent_node_id}` : ''}`)
    }
    if (run.root_run_id && run.root_run_id !== run.run_id) {
        lineageParts.push(`Root ${run.root_run_id}`)
    }
    if (run.child_invocation_index !== null && run.child_invocation_index !== undefined) {
        lineageParts.push(`Child invocation #${run.child_invocation_index}`)
    }
    return lineageParts.length > 0 ? lineageParts.join(' · ') : null
}

export function formatOutcomeReason(run: RunRecord): string | null {
    const reasonMessage = run.outcome_reason_message?.trim()
    const reasonCode = run.outcome_reason_code?.trim()
    if (reasonMessage && reasonCode) {
        return `${reasonMessage} (${reasonCode})`
    }
    if (reasonMessage || reasonCode) {
        return reasonMessage || reasonCode || null
    }
    return run.last_error?.trim() || null
}

export function hasExecutionMetadata(run: RunRecord): boolean {
    return Boolean(
        run.execution_profile_id
        || run.execution_mode
        || run.execution_container_image,
    )
}

export function hasExecutionLockMetadata(run: RunRecord): boolean {
    return Boolean(run.execution_lock?.identity)
}
