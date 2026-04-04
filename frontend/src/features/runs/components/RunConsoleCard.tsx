import { useEffect, useMemo, useRef, useState } from 'react'

import { useStore } from '@/store'
import {
    EmptyState,
    Panel,
    PanelContent,
    PanelHeader,
    SectionHeader,
    Button,
} from '@/ui'

import { useRunExplainability } from '../hooks/useRunExplainability'
import { STATUS_LABELS } from '../model/shared'
import { RunSectionToggleButton } from './RunSectionToggleButton'

export function RunConsoleCard() {
    const selectedRunId = useStore((state) => state.selectedRunId)
    const selectedRunRecord = useStore((state) => state.selectedRunRecord)
    const logs = useStore((state) => state.logs)
    const clearLogs = useStore((state) => state.clearLogs)
    const runtimeStatus = useStore((state) => state.runtimeStatus)
    const statusLabel = selectedRunRecord?.run_id === selectedRunId
        ? (STATUS_LABELS[selectedRunRecord.status] || selectedRunRecord.status)
        : (STATUS_LABELS[runtimeStatus] || runtimeStatus)
    const latestLog = useMemo(() => logs.at(-1) ?? null, [logs])
    const logsEndRef = useRef<HTMLDivElement>(null)
    const { failureDecisions, retryDecisions, routingDecisions } = useRunExplainability(selectedRunId)
    const [collapsed, setCollapsed] = useState(false)

    useEffect(() => {
        logsEndRef.current?.scrollIntoView({ behavior: 'smooth' })
    }, [logs])

    return (
        <Panel data-testid="run-console-panel">
            <PanelHeader>
                <SectionHeader
                    title="Run Console"
                    description="Live runtime logs and recent explainability signals for the selected run."
                    action={(
                        <div className="flex items-center gap-2">
                            <span data-testid="run-console-status" className="rounded border border-border bg-background px-2 py-0.5 text-[11px] text-muted-foreground">
                                Status: {statusLabel}
                            </span>
                            <span data-testid="run-console-log-count" className="rounded border border-border bg-background px-2 py-0.5 text-[11px] text-muted-foreground">
                                Logs: {logs.length}
                            </span>
                            <Button
                                data-testid="run-console-clear-button"
                                onClick={clearLogs}
                                variant="outline"
                                size="xs"
                            >
                                Clear
                            </Button>
                            <RunSectionToggleButton
                                collapsed={collapsed}
                                onToggle={() => setCollapsed((current) => !current)}
                                testId="run-console-toggle-button"
                            />
                        </div>
                    )}
                />
            </PanelHeader>
            {!collapsed ? (
                <PanelContent className="space-y-4">
                <div className="grid gap-3 xl:grid-cols-3">
                    <section data-testid="routing-explainability-view" className="rounded-md border border-border bg-background/80 p-3">
                        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Routing Decisions</h3>
                        <ul className="mt-2 space-y-1 text-xs">
                            {routingDecisions.length === 0 ? (
                                <li className="text-muted-foreground">No routing decisions yet.</li>
                            ) : (
                                routingDecisions.map((decision) => (
                                    <li key={decision.id} className="rounded border border-border/80 bg-muted/40 px-2 py-1">
                                        <span className="font-medium">{decision.from}</span> {'->'} <span className="font-medium">{decision.to}</span>
                                        <span className="ml-2 text-muted-foreground">({decision.reason})</span>
                                    </li>
                                ))
                            )}
                        </ul>
                    </section>

                    <section data-testid="retry-explainability-view" className="rounded-md border border-border bg-background/80 p-3">
                        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Retry Decisions</h3>
                        <ul className="mt-2 space-y-1 text-xs">
                            {retryDecisions.length === 0 ? (
                                <li className="text-muted-foreground">No retry decisions yet.</li>
                            ) : (
                                retryDecisions.map((decision) => (
                                    <li key={decision.id} className="rounded border border-border/80 bg-muted/40 px-2 py-1">
                                        <span className="font-medium">{decision.node}</span>
                                        <span className="ml-2 text-muted-foreground">
                                            attempt {decision.attempt}, delay {decision.delayMs}ms
                                        </span>
                                    </li>
                                ))
                            )}
                        </ul>
                    </section>

                    <section data-testid="failure-explainability-view" className="rounded-md border border-border bg-background/80 p-3">
                        <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Failure Decisions</h3>
                        <ul className="mt-2 space-y-1 text-xs">
                            {failureDecisions.length === 0 ? (
                                <li className="text-muted-foreground">No failure decisions yet.</li>
                            ) : (
                                failureDecisions.map((decision) => (
                                    <li key={decision.id} className="rounded border border-border/80 bg-muted/40 px-2 py-1">
                                        <span className="font-medium">{decision.node}</span>
                                        <span className="ml-2 text-muted-foreground">{decision.error}</span>
                                        <span className="ml-2 text-muted-foreground">
                                            ({decision.willRetry ? 'retrying' : 'terminal'})
                                        </span>
                                    </li>
                                ))
                            )}
                        </ul>
                    </section>
                </div>

                <div className="rounded-md border border-border/80 bg-muted/20 px-3 py-2 text-xs text-muted-foreground">
                    {latestLog ? `Latest log: ${latestLog.msg}` : 'No runtime logs yet.'}
                </div>

                <div data-testid="run-console-output" className="max-h-96 overflow-y-auto rounded-md border border-border/80 bg-background p-4 font-mono text-sm">
                    {logs.length === 0 ? (
                        <EmptyState description="No runtime logs have arrived for this run yet." />
                    ) : (
                        <div className="space-y-1">
                            {logs.map((log, index) => (
                                <div key={`${log.time}-${index}`} data-testid="run-console-log-row" className="flex gap-4 rounded px-2 py-0.5 hover:bg-muted/50">
                                    <span className="w-20 shrink-0 select-none text-muted-foreground">{log.time}</span>
                                    <span className={
                                        log.type === 'success'
                                            ? 'break-all text-green-500'
                                            : log.type === 'error'
                                                ? 'break-all text-destructive'
                                                : 'break-all text-foreground'
                                    }>
                                        {log.msg}
                                    </span>
                                </div>
                            ))}
                            <div ref={logsEndRef} />
                        </div>
                    )}
                </div>
                </PanelContent>
            ) : null}
        </Panel>
    )
}
