import { useEffect, useState } from 'react'

import { useStore } from '@/store'
import { fetchFlowListValidated } from '@/lib/attractorClient'
import { useNarrowViewport } from '@/lib/useNarrowViewport'

export function ExecutionSidebar() {
    const executionFlow = useStore((state) => state.executionFlow)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setSelectedRunId = useStore((state) => state.setSelectedRunId)
    const clearLogs = useStore((state) => state.clearLogs)
    const resetNodeStatuses = useStore((state) => state.resetNodeStatuses)
    const clearHumanGate = useStore((state) => state.clearHumanGate)
    const setRuntimeStatus = useStore((state) => state.setRuntimeStatus)
    const humanGate = useStore((state) => state.humanGate)
    const isNarrowViewport = useNarrowViewport()
    const [flows, setFlows] = useState<string[]>([])

    const handleSelectFlow = (flowName: string) => {
        if (flowName === executionFlow) {
            return
        }
        setExecutionFlow(flowName)
        setSelectedRunId(null)
        setRuntimeStatus('idle')
        clearLogs()
        resetNodeStatuses()
        clearHumanGate()
    }

    useEffect(() => {
        let cancelled = false

        const loadFlows = async () => {
            try {
                const data = await fetchFlowListValidated()
                if (!cancelled) {
                    setFlows(data)
                }
            } catch (error) {
                console.error(error)
            }
        }

        void loadFlows()
        return () => {
            cancelled = true
        }
    }, [])

    return (
        <nav
            data-testid="execution-flow-panel"
            className={`bg-background flex shrink-0 flex-col overflow-hidden z-40 ${
                isNarrowViewport ? 'w-full max-h-[46vh] border-b' : 'w-72 border-r'
            }`}
        >
            <div className="px-4 pb-2 pt-4">
                <div className="flex items-center gap-3 text-xs font-semibold uppercase tracking-[0.2em] text-foreground">
                    <span>Execution</span>
                    <span className="h-2 w-2 rounded-full bg-muted-foreground/40" />
                </div>
            </div>
            <div className="px-5 py-2">
                <h2 className="font-semibold text-sm tracking-tight">Flow Library</h2>
                <p className="mt-1 text-xs text-muted-foreground">
                    Execution keeps its own inspected flow separate from the editor.
                </p>
            </div>
            <div className="flex-1 overflow-y-auto px-3 pb-4">
                <div className="space-y-1">
                    {flows.map((flowName) => (
                        <button
                            key={flowName}
                            onClick={() => handleSelectFlow(flowName)}
                            className={`w-full rounded-md px-3 py-2 text-left text-sm transition-colors ${
                                executionFlow === flowName
                                    ? 'bg-secondary text-secondary-foreground font-medium'
                                    : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                            }`}
                        >
                            <span className="flex items-center gap-2">
                                {humanGate?.flowName === flowName ? (
                                    <span
                                        className="h-2 w-2 rounded-full bg-amber-500"
                                        title="Needs human input"
                                    />
                                ) : null}
                                {flowName}
                            </span>
                        </button>
                    ))}
                </div>
            </div>
        </nav>
    )
}
