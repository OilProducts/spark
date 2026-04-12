import { Alert, AlertDescription } from '@/components/ui/alert'
import { Button } from '@/components/ui/button'
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
                <Alert
                    data-testid="execute-warning-banner"
                    className="border-amber-500/40 bg-amber-500/10 px-2 py-1 text-[11px] font-medium leading-none text-amber-800"
                >
                    <AlertDescription className="text-inherit">
                        Warnings present; run allowed.
                    </AlertDescription>
                </Alert>
            ) : null}
            {runStartGitPolicyWarning ? (
                <Alert
                    data-testid="run-start-git-policy-warning-banner"
                    className="max-w-sm truncate border-amber-500/40 bg-amber-500/10 px-2 py-1 text-[11px] font-medium leading-none text-amber-800"
                >
                    <AlertDescription className="text-inherit">
                        {runStartGitPolicyWarning}
                    </AlertDescription>
                </Alert>
            ) : null}
            {runStartError ? (
                <Alert
                    data-testid="run-start-error-banner"
                    className="max-w-sm truncate border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] font-medium leading-none text-destructive"
                >
                    <AlertDescription className="text-inherit">
                        Failed to start run: {runStartError}
                    </AlertDescription>
                </Alert>
            ) : null}
            {lastLaunchFailure ? (
                <Alert
                    data-testid="launch-failure-diagnostics"
                    className="max-w-sm border-destructive/40 bg-destructive/10 px-2 py-1 text-[11px] text-destructive"
                >
                    <AlertDescription className="text-inherit">
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
                    </AlertDescription>
                </Alert>
            ) : null}
        </div>
    )
}
