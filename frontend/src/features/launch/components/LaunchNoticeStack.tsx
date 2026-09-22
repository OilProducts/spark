import { Alert, AlertDescription } from '@/components/ui/alert'
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button'

import type { LaunchFailureDiagnostics } from '../model/launchTypes'

interface LaunchNoticeStackProps {
    showValidationWarningBanner: boolean
    runStartGitPolicyWarning: string | null
    runStartError: string | null
    lastLaunchFailure: LaunchFailureDiagnostics | null
    canRetryLaunch: boolean
    onRetry: () => void
}

export function LaunchNoticeStack({
    showValidationWarningBanner,
    runStartGitPolicyWarning,
    runStartError,
    lastLaunchFailure,
    canRetryLaunch,
    onRetry,
}: LaunchNoticeStackProps) {
    return (
        <div className="flex flex-wrap items-center gap-2">
            {showValidationWarningBanner ? (
                <Alert
                    data-testid="execute-warning-banner"
                    className="border-warning/40 bg-warning/10 px-2 py-1 text-xs font-medium leading-none text-warning"
                >
                    <AlertDescription className="text-inherit">
                        Warnings present; run allowed.
                    </AlertDescription>
                </Alert>
            ) : null}
            {runStartGitPolicyWarning ? (
                <Alert
                    data-testid="run-start-git-policy-warning-banner"
                    className="max-w-sm truncate border-warning/40 bg-warning/10 px-2 py-1 text-xs font-medium leading-none text-warning"
                >
                    <AlertDescription className="text-inherit">
                        {runStartGitPolicyWarning}
                    </AlertDescription>
                </Alert>
            ) : null}
            {runStartError ? (
                <InlineError data-testid="run-start-error-banner" className="max-w-sm truncate" dense>
                    Failed to start run: {runStartError}
                </InlineError>
            ) : null}
            {lastLaunchFailure ? (
                <InlineError data-testid="launch-failure-diagnostics" className="max-w-sm" title="Last launch failure" dense>
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
                </InlineError>
            ) : null}
        </div>
    )
}
