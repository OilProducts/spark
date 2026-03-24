import { Button, InlineNotice } from '@/ui'

type LaunchFailureDiagnostics = {
    message: string
    failedAt: string
    flowSource: string | null
}

interface ExecutionNoticeStackProps {
    showValidationWarningBanner: boolean
    runStartGitPolicyWarning: string | null
    runStartError: string | null
    lastLaunchFailure: LaunchFailureDiagnostics | null
    canRetryLaunch: boolean
    onRetry: () => void
}

export function ExecutionNoticeStack({
    showValidationWarningBanner,
    runStartGitPolicyWarning,
    runStartError,
    lastLaunchFailure,
    canRetryLaunch,
    onRetry,
}: ExecutionNoticeStackProps) {
    return (
        <div className="flex flex-wrap items-center gap-2">
            {showValidationWarningBanner ? (
                <InlineNotice
                    data-testid="execute-warning-banner"
                    tone="warning"
                    className="px-2 py-1 text-[11px] font-medium leading-none"
                >
                    Warnings present; run allowed.
                </InlineNotice>
            ) : null}
            {runStartGitPolicyWarning ? (
                <InlineNotice
                    data-testid="run-start-git-policy-warning-banner"
                    tone="warning"
                    className="max-w-sm truncate px-2 py-1 text-[11px] font-medium leading-none"
                >
                    {runStartGitPolicyWarning}
                </InlineNotice>
            ) : null}
            {runStartError ? (
                <InlineNotice
                    data-testid="run-start-error-banner"
                    tone="error"
                    className="max-w-sm truncate px-2 py-1 text-[11px] font-medium leading-none"
                >
                    Failed to start run: {runStartError}
                </InlineNotice>
            ) : null}
            {lastLaunchFailure ? (
                <InlineNotice
                    data-testid="launch-failure-diagnostics"
                    tone="error"
                    className="max-w-sm px-2 py-1 text-[11px]"
                >
                    <p className="font-medium">Last launch failure</p>
                    <p data-testid="launch-failure-message" className="truncate">
                        {lastLaunchFailure.message}
                    </p>
                    <p className="truncate">
                        Flow source: <span className="font-mono">{lastLaunchFailure.flowSource || 'none'}</span>
                    </p>
                    <p>Failed at: {new Date(lastLaunchFailure.failedAt).toLocaleString()}</p>
                    <Button
                        data-testid="launch-retry-button"
                        onClick={onRetry}
                        disabled={!canRetryLaunch}
                        size="xs"
                        variant="outline"
                        className="mt-1 h-7 border-destructive/40 text-destructive hover:bg-destructive/5"
                    >
                        Retry launch
                    </Button>
                    {!canRetryLaunch ? (
                        <p data-testid="launch-retry-disabled-reason" className="mt-1">
                            Resolve launch blockers to retry.
                        </p>
                    ) : null}
                </InlineNotice>
            ) : null}
        </div>
    )
}
