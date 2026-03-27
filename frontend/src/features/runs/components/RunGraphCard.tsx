import { useCallback, useEffect, useRef, useState } from 'react'
import {
    Background,
    Controls,
    MiniMap,
    ReactFlow,
    ReactFlowProvider,
    applyEdgeChanges,
    applyNodeChanges,
    useEdgesState,
    useNodesState,
    type Edge,
    type EdgeChange,
    type Node,
    type NodeChange,
} from '@xyflow/react'
import '@xyflow/react/dist/style.css'

import { isAbortError } from '@/lib/api/shared'
import { useStore } from '@/store'
import {
    buildHydratedFlowGraph,
    CanvasSessionModeProvider,
    edgeTypes,
    layoutWithElk,
    nodeTypes,
    nowMs,
} from '@/features/workflow-canvas'
import { Button, EmptyState, InlineNotice, Panel, PanelContent, PanelHeader, SectionHeader } from '@/ui'

import type { RunRecord } from '../model/shared'
import { loadRunGraphPreview } from '../services/runGraphTransport'
import { RunSectionToggleButton } from './RunSectionToggleButton'

type RunGraphCanvasInnerProps = {
    run: RunRecord
    refreshToken: number
    onLoadingChange: (loading: boolean) => void
    onErrorChange: (error: string | null) => void
}

function RunGraphCanvasInner({
    run,
    refreshToken,
    onLoadingChange,
    onErrorChange,
}: RunGraphCanvasInnerProps) {
    const replaceRunGraphAttrs = useStore((state) => state.replaceRunGraphAttrs)
    const setRunDiagnostics = useStore((state) => state.setRunDiagnostics)
    const clearRunDiagnostics = useStore((state) => state.clearRunDiagnostics)
    const [nodes, setNodes] = useNodesState<Node>([])
    const [edges, setEdges] = useEdgesState<Edge>([])
    const [lastLayoutMs, setLastLayoutMs] = useState(0)
    const activeLoadRef = useRef(0)

    useEffect(() => {
        const loadId = activeLoadRef.current + 1
        activeLoadRef.current = loadId
        const controller = new AbortController()
        let cancelled = false
        const isCurrentLoad = () => !cancelled && activeLoadRef.current === loadId

        replaceRunGraphAttrs({})
        clearRunDiagnostics()
        setNodes([])
        setEdges([])
        setLastLayoutMs(0)
        onLoadingChange(true)
        onErrorChange(null)

        const startLoad = async () => {
            try {
                const preview = await loadRunGraphPreview(run.run_id, { signal: controller.signal })
                if (!isCurrentLoad()) {
                    return
                }

                if (preview.diagnostics) {
                    setRunDiagnostics(preview.diagnostics)
                } else {
                    clearRunDiagnostics()
                }

                const hydratedGraph = buildHydratedFlowGraph(
                    run.flow_name || run.run_id,
                    preview,
                    {
                        llm_model: '',
                        llm_provider: '',
                        reasoning_effort: '',
                    },
                )
                if (!hydratedGraph) {
                    onErrorChange('Run graph preview did not include a renderable graph.')
                    return
                }

                const layoutStart = nowMs()
                const laidOutGraph = await layoutWithElk(hydratedGraph.nodes, hydratedGraph.edges)
                if (!isCurrentLoad()) {
                    return
                }

                replaceRunGraphAttrs(hydratedGraph.graphAttrs)
                setLastLayoutMs(Math.max(0, nowMs() - layoutStart))
                setNodes(laidOutGraph.nodes)
                setEdges(laidOutGraph.edges)
            } catch (error) {
                if (controller.signal.aborted || isAbortError(error)) {
                    return
                }
                console.error(error)
                onErrorChange(error instanceof Error ? error.message : 'Unable to load the run graph preview.')
            } finally {
                if (isCurrentLoad()) {
                    onLoadingChange(false)
                }
            }
        }

        void startLoad()

        return () => {
            cancelled = true
            controller.abort()
        }
    }, [
        clearRunDiagnostics,
        onErrorChange,
        onLoadingChange,
        refreshToken,
        replaceRunGraphAttrs,
        run.flow_name,
        run.run_id,
        setEdges,
        setNodes,
        setRunDiagnostics,
    ])

    const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {
        setNodes((currentNodes) => applyNodeChanges(changes, currentNodes))
    }, [setNodes])

    const onEdgesChange = useCallback((changes: EdgeChange<Edge>[]) => {
        setEdges((currentEdges) => applyEdgeChanges(changes, currentEdges))
    }, [setEdges])

    if (nodes.length === 0 && edges.length === 0) {
        return (
            <div className="flex h-[28rem] items-center justify-center rounded-md border border-dashed border-border bg-muted/20">
                <EmptyState description="Run graph will appear once the preview loads." />
            </div>
        )
    }

    return (
        <div data-testid="run-graph-canvas" className="h-[32rem] overflow-hidden rounded-md border border-border/80 bg-background">
            <ReactFlow
                nodes={nodes}
                edges={edges}
                onNodesChange={onNodesChange}
                onEdgesChange={onEdgesChange}
                fitView
                nodesDraggable={false}
                nodesConnectable={false}
                elementsSelectable={true}
                nodeTypes={nodeTypes}
                edgeTypes={edgeTypes}
                deleteKeyCode={null}
                multiSelectionKeyCode={null}
                proOptions={{ hideAttribution: true }}
            >
                <MiniMap pannable zoomable />
                <Controls showInteractive={false} />
                <Background gap={24} size={1} />
            </ReactFlow>
            <div className="border-t border-border/70 bg-muted/20 px-3 py-2 text-[11px] text-muted-foreground">
                Last layout: {Math.round(lastLayoutMs)}ms
            </div>
        </div>
    )
}

export function RunGraphCard({ run }: { run: RunRecord }) {
    const diagnostics = useStore((state) => state.runDiagnostics)
    const [refreshToken, setRefreshToken] = useState(0)
    const [isLoading, setIsLoading] = useState(false)
    const [loadError, setLoadError] = useState<string | null>(null)
    const [collapsed, setCollapsed] = useState(false)

    return (
        <Panel data-testid="run-graph-panel">
            <PanelHeader>
                <SectionHeader
                    title="Run Graph"
                    description="Snapshot-accurate workflow graph captured from the run's stored DOT source."
                    action={(
                        <div className="flex items-center gap-2">
                            <Button
                                onClick={() => setRefreshToken((current) => current + 1)}
                                data-testid="run-graph-refresh-button"
                                variant="outline"
                                size="xs"
                            >
                                {isLoading ? 'Refreshing…' : 'Refresh'}
                            </Button>
                            <RunSectionToggleButton
                                collapsed={collapsed}
                                onToggle={() => setCollapsed((current) => !current)}
                                testId="run-graph-toggle-button"
                            />
                        </div>
                    )}
                />
            </PanelHeader>
            {!collapsed ? (
                <PanelContent className="space-y-3">
                {loadError ? (
                    <InlineNotice tone="error" data-testid="run-graph-error">
                        {loadError}
                    </InlineNotice>
                ) : null}
                {diagnostics.length > 0 ? (
                    <div data-testid="run-graph-diagnostics" className="rounded-md border border-border/80 bg-muted/20 p-3">
                        <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                            Graph diagnostics
                        </p>
                        <ul className="mt-2 space-y-2 text-sm">
                            {diagnostics.slice(0, 6).map((diagnostic, index) => (
                                <li key={`${diagnostic.rule_id}-${diagnostic.node_id || 'graph'}-${index}`} className="rounded border border-border/80 bg-background/80 px-3 py-2">
                                    <div className="flex flex-wrap items-center gap-2 text-xs uppercase tracking-wide text-muted-foreground">
                                        <span>{diagnostic.severity}</span>
                                        <span>{diagnostic.rule_id}</span>
                                        {diagnostic.node_id ? <span>{diagnostic.node_id}</span> : null}
                                    </div>
                                    <p className="mt-1 text-sm text-foreground">{diagnostic.message}</p>
                                </li>
                            ))}
                        </ul>
                    </div>
                ) : null}
                <CanvasSessionModeProvider mode="runs">
                    <ReactFlowProvider>
                        <RunGraphCanvasInner
                            run={run}
                            refreshToken={refreshToken}
                            onLoadingChange={setIsLoading}
                            onErrorChange={setLoadError}
                        />
                    </ReactFlowProvider>
                </CanvasSessionModeProvider>
                </PanelContent>
            ) : null}
        </Panel>
    )
}
