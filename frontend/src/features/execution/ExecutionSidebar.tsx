import { useEffect, useRef, useState } from 'react'

import { useStore } from '@/store'
import { Badge } from '@/components/ui/badge'
import { Button } from '@/components/ui/button'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { FlowTree } from '@/components/app/flow-tree'
import { formatProjectPathLabel } from '@/lib/projectPaths'
import { loadExecutionFlowCatalog } from './services/flowCatalog'
export function ExecutionSidebar() {
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const executionFlow = useStore((state) => state.executionFlow)
    const executionContinuation = useStore((state) => state.executionContinuation)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const setExecutionContinuationFlowSourceMode = useStore((state) => state.setExecutionContinuationFlowSourceMode)
    const setExecutionContinuationStartNode = useStore((state) => state.setExecutionContinuationStartNode)
    const humanGate = useStore((state) => state.humanGate)
    const isNarrowViewport = useNarrowViewport()
    const [flows, setFlows] = useState<string[]>([])
    const [isRefreshingFlows, setIsRefreshingFlows] = useState(false)
    const executionFlowRef = useRef(executionFlow)
    const executionContinuationRef = useRef(executionContinuation)
    const isMountedRef = useRef(true)
    const refreshRequestIdRef = useRef(0)
    const projectLabel = activeProjectPath
        ? formatProjectPathLabel(activeProjectPath)
        : 'No active project'
    executionFlowRef.current = executionFlow
    executionContinuationRef.current = executionContinuation

    const handleSelectFlow = (flowName: string) => {
        if (!executionContinuation && flowName === executionFlow) {
            return
        }
        setExecutionFlow(flowName)
        if (executionContinuation) {
            setExecutionContinuationFlowSourceMode('flow_name')
            setExecutionContinuationStartNode(null)
        }
    }

    const refreshFlows = async () => {
        const requestId = refreshRequestIdRef.current + 1
        refreshRequestIdRef.current = requestId
        if (isMountedRef.current) {
            setIsRefreshingFlows(true)
        }

        try {
            const data = await loadExecutionFlowCatalog()
            if (!isMountedRef.current || requestId !== refreshRequestIdRef.current) {
                return
            }

            setFlows(data)

            const selectedFlow = executionFlowRef.current
            if (selectedFlow && !data.includes(selectedFlow)) {
                setExecutionFlow(null)
                if (executionContinuationRef.current?.flowSourceMode === 'flow_name') {
                    setExecutionContinuationStartNode(null)
                }
            }
        } catch (error) {
            console.error(error)
        } finally {
            if (isMountedRef.current && requestId === refreshRequestIdRef.current) {
                setIsRefreshingFlows(false)
            }
        }
    }

    useEffect(() => {
        isMountedRef.current = true

        void refreshFlows()
        return () => {
            isMountedRef.current = false
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
                <Badge
                    data-testid="execution-project-context-chip"
                    variant="outline"
                    className="mt-3"
                    title={activeProjectPath || 'No active project'}
                >
                    <span className="text-muted-foreground">Project:</span>
                    <span className="max-w-40 truncate">{projectLabel}</span>
                </Badge>
            </div>
            <div className="px-5 py-2">
                <div className="flex items-center justify-between gap-3">
                    <h2 className="font-semibold text-sm tracking-tight">Flow Library</h2>
                    <Button
                        type="button"
                        variant="outline"
                        size="xs"
                        data-testid="execution-flow-refresh-button"
                        onClick={() => {
                            void refreshFlows()
                        }}
                        disabled={isRefreshingFlows}
                    >
                        {isRefreshingFlows ? 'Refreshing…' : 'Refresh'}
                    </Button>
                </div>
                <p className="mt-1 text-xs text-muted-foreground">
                    {executionContinuation
                        ? 'Select an installed flow to override the source-run snapshot for continuation.'
                        : 'Execution keeps its own launch target separate from the editor.'}
                </p>
            </div>
            <div className="flex-1 overflow-y-auto px-3 pb-4">
                <FlowTree
                    dataTestId="execution-flow-tree"
                    flows={flows}
                    selectedFlow={executionFlow}
                    onSelectFlow={handleSelectFlow}
                    renderFlowIndicator={(flowName) => (
                        humanGate?.flowName === flowName ? (
                            <span
                                className="h-2 w-2 rounded-full bg-amber-500"
                                title="Needs human input"
                            />
                        ) : null
                    )}
                />
            </div>
        </nav>
    )
}
