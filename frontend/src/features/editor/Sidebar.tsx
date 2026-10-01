import { useStore, type DiagnosticEntry } from "@/store"
import { useMemo } from "react"
import { useReactFlow, useStore as useReactFlowStore, type Edge, type Node } from "@xyflow/react"
import { generateFlowYaml } from "@/lib/flowYamlUtils"
import { getHandlerType, getNodeFieldVisibility } from "@/lib/nodeVisibility"
import { getToolHookCommandWarning } from "@/lib/graphAttrValidation"
import { resolveEdgeFieldDiagnostics, resolveNodeFieldDiagnostics } from "@/lib/inspectorFieldDiagnostics"
import { toExtensionAttrEntries } from "@/lib/extensionAttrs"
import {
    getReactFlowNodeTypeForShape,
    getShapeNodeStyle,
    normalizeWorkflowNodeShape,
} from '@/lib/workflowNodeShape'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { EdgeInspectorPanel } from './components/EdgeInspectorPanel'
import { GraphInspectorPanel } from './components/GraphInspectorPanel'
import { NodeInspectorPanel } from './components/NodeInspectorPanel'
import {
    parseContextKeyDraft,
    parseContextKeyList,
    serializeContextKeyList,
} from '@/lib/flowContracts'
import { useEditorGraphBridgeRef } from './EditorGraphBridgeContext'

const DEFAULT_NODE_INSPECTOR_SESSION = {
    showAdvanced: false,
    readsContextDraft: '',
    readsContextError: null as string | null,
    writesContextDraft: '',
    writesContextError: null as string | null,
}

type InspectorScope = 'none' | 'graph' | 'node' | 'edge'
const INSPECTOR_SAVE_DEBOUNCE_MS = 600
type FlowGraphSnapshot = {
    nodes: Node[]
    edges: Edge[]
}
const CORE_NODE_ATTR_KEYS = new Set<string>([
    'label',
    'kind',
    'config',
    'context',
    'contracts',
    'runtime',
    'manager',
    'retry',
    'execution',
    'ui',
    'extensions',
    'shape',
    'flow_ref',
    'input_map',
    'decisions',
    'prompt',
    'tool.command',
    'tool.hooks.pre',
    'tool.hooks.post',
    'tool.artifacts.paths',
    'tool.artifacts.stdout',
    'tool.artifacts.stderr',
    'join_policy',
    'join_k',
    'join_quorum',
    'error_policy',
    'max_parallel',
    'type',
    'max_retries',
    'goal_gate',
    'retry_target',
    'fallback_retry_target',
    'fidelity',
    'thread_id',
    'class',
    'timeout',
    'llm_model',
    'llm_provider',
    'llm_profile',
    'reasoning_effort',
    'auto_status',
    'allow_partial',
    'manager.poll_interval',
    'manager.max_cycles',
    'manager.stop_condition',
    'manager.actions',
    'manager.steer_cooldown',
    'stack.child_autostart',
    'spark.reads_context',
    'spark.writes_context',
])
const CORE_EDGE_ATTR_KEYS = new Set<string>([
    'label',
    'condition',
    'weight',
    'fidelity',
    'thread_id',
    'loop_restart',
])
const EXCLUDED_NODE_EXTENSION_ATTR_KEYS = new Set<string>(['status', 'human.default_choice'])

const NODE_KIND_TO_SHAPE: Record<string, string> = {
    start: 'Mdiamond',
    exit: 'Msquare',
    agent_task: 'box',
    human_gate: 'hexagon',
    conditional: 'diamond',
    parallel: 'component',
    fan_in: 'tripleoctagon',
    tool: 'parallelogram',
    subflow: 'house',
}

function resolveInspectorScope({
    activeFlow,
    selectedNodeId,
    selectedEdgeId,
}: {
    activeFlow: string | null
    selectedNodeId: string | null
    selectedEdgeId: string | null
}): InspectorScope {
    if (selectedEdgeId) return 'edge'
    if (selectedNodeId) return 'node'
    if (activeFlow) return 'graph'
    return 'none'
}

export function applyNodePropertyChangeToData(
    data: Record<string, unknown>,
    key: string,
    value: string | boolean,
): Record<string, unknown> {
    const nextData = { ...data, [key]: value }
    if (key === 'kind' && typeof value === 'string') {
        nextData.shape = NODE_KIND_TO_SHAPE[value] ?? 'box'
        const existingConfig = nextData.config && typeof nextData.config === 'object'
            ? nextData.config as Record<string, unknown>
            : {}
        nextData.config = { ...existingConfig, kind: value }
        if (value === 'subflow' && typeof nextData.flow_ref !== 'string') {
            nextData.flow_ref = typeof existingConfig.flow_ref === 'string' ? existingConfig.flow_ref : 'child.yaml'
        }
    }
    if (key === 'join_policy') {
        if (value !== 'k_of_n') {
            delete nextData.join_k
        }
        if (value !== 'quorum') {
            delete nextData.join_quorum
        }
    }
    if (key === 'flow_ref') {
        const existingConfig = nextData.config && typeof nextData.config === 'object'
            ? nextData.config as Record<string, unknown>
            : {}
        nextData.config = { ...existingConfig, kind: 'subflow', flow_ref: value }
    }
    return nextData
}

export function Sidebar({ desktopWidthPx = 288 }: { desktopWidthPx?: number }) {
    const activeFlow = useStore((state) => state.activeFlow)
    const selectedNodeId = useStore((state) => state.selectedNodeId)
    const selectedEdgeId = useStore((state) => state.selectedEdgeId)
    const setSelectedNodeId = useStore((state) => state.setSelectedNodeId)
    const setSelectedEdgeId = useStore((state) => state.setSelectedEdgeId)
    const isNarrowViewport = useNarrowViewport()
    const diagnostics = useStore((state) => state.diagnostics)
    const edgeDiagnostics = useStore((state) => state.edgeDiagnostics)
    const flowMetadata = useStore((state) => state.flowMetadata)
    const editorNodeInspectorSessionsByNodeId = useStore((state) => state.editorNodeInspectorSessionsByNodeId)
    const preferredAdvancedControls = useStore((state) => state.preferredAdvancedControls)
    const updateEditorNodeInspectorSession = useStore((state) => state.updateEditorNodeInspectorSession)
    const editorGraphBridgeRef = useEditorGraphBridgeRef()
    const { getNodes, setNodes, getEdges, setEdges } = useReactFlow()
    const nodes = useReactFlowStore((state) => state.nodes)
    const edges = useReactFlowStore((state) => state.edges)
    const readNodes = () => editorGraphBridgeRef?.current?.getNodes() ?? getNodes()
    const readEdges = () => editorGraphBridgeRef?.current?.getEdges() ?? getEdges()
    const updateNodes = (updater: Parameters<typeof setNodes>[0]) =>
        (editorGraphBridgeRef?.current?.setNodes ?? setNodes)(updater)
    const updateEdges = (updater: Parameters<typeof setEdges>[0]) =>
        (editorGraphBridgeRef?.current?.setEdges ?? setEdges)(updater)
    const { scheduleSave } = useFlowSaveScheduler<FlowGraphSnapshot>({
        flowName: activeFlow,
        debounceMs: INSPECTOR_SAVE_DEBOUNCE_MS,
        buildContent: (snapshot, currentFlowName) => generateFlowYaml(
            currentFlowName,
            snapshot?.nodes || [],
            snapshot?.edges || [],
            flowMetadata,
        ),
    })

    const applyNodeVisualState = (node: Node, nextData: Record<string, unknown>) => {
        const nextShape = normalizeWorkflowNodeShape((nextData.shape as string) || (node.data?.shape as string) || 'box')
        return {
            ...node,
            type: getReactFlowNodeTypeForShape(nextShape),
            style: getShapeNodeStyle(nextShape),
            data: nextData,
        }
    }

    const updateNodeProperty = (nodeId: string, key: string, value: string | boolean) => {
        if (!activeFlow) return;

        let newNodes: Node[] = [];
        updateNodes((nds) => {
            newNodes = nds.map(node => {
                if (node.id === nodeId) {
                    const nextData = applyNodePropertyChangeToData(
                        (node.data ?? {}) as Record<string, unknown>,
                        key,
                        value,
                    )
                    return applyNodeVisualState(node, nextData);
                }
                return node;
            });
            return newNodes;
        });

        if (newNodes.length > 0) {
            scheduleSave({ nodes: newNodes, edges: readEdges() });
        }
    }

    const handlePropertyChange = (key: string, value: string | boolean) => {
        if (!selectedNodeId) return;
        updateNodeProperty(selectedNodeId, key, value)
    }

    const openGraphChildSettings = () => {
        setSelectedEdgeId(null)
        setSelectedNodeId(null)
    }

    const selectedNode = nodes.find(n => n.id === selectedNodeId);
    const selectedEdge = edges.find(e => e.id === selectedEdgeId);
    const selectedNodeExtensionEntries = useMemo(
        () => toExtensionAttrEntries(
            (selectedNode?.data ?? {}) as Record<string, unknown>,
            CORE_NODE_ATTR_KEYS,
            EXCLUDED_NODE_EXTENSION_ATTR_KEYS,
        ),
        [selectedNode?.data],
    )
    const selectedEdgeExtensionEntries = useMemo(
        () => toExtensionAttrEntries(
            (selectedEdge?.data ?? {}) as Record<string, unknown>,
            CORE_EDGE_ATTR_KEYS,
        ),
        [selectedEdge?.data],
    )
    const selectedEdgeDiagnosticKey = selectedEdge ? `${selectedEdge.source}->${selectedEdge.target}` : null
    const selectedEdgeConditionDiagnostics = selectedEdgeDiagnosticKey
        ? (edgeDiagnostics[selectedEdgeDiagnosticKey] || []).filter((diag) => diag.rule_id === 'condition_syntax')
        : []
    const conditionPreviewHasError = selectedEdgeConditionDiagnostics.some((diag) => diag.severity === 'error')
    const conditionPreviewHasWarning = selectedEdgeConditionDiagnostics.some((diag) => diag.severity === 'warning')
    const handlerType = getHandlerType((selectedNode?.data?.kind as string) || '')
    const selectedNodeReadsContextRaw = typeof selectedNode?.data?.['spark.reads_context'] === 'string'
        ? selectedNode.data['spark.reads_context']
        : ''
    const selectedNodeWritesContextRaw = typeof selectedNode?.data?.['spark.writes_context'] === 'string'
        ? selectedNode.data['spark.writes_context']
        : ''
    const parsedSelectedNodeReadsContext = useMemo(
        () => parseContextKeyList(selectedNodeReadsContextRaw),
        [selectedNodeReadsContextRaw],
    )
    const parsedSelectedNodeWritesContext = useMemo(
        () => parseContextKeyList(selectedNodeWritesContextRaw),
        [selectedNodeWritesContextRaw],
    )
    const selectedNodeInspectorSession = selectedNodeId
        ? (
            editorNodeInspectorSessionsByNodeId[selectedNodeId]
            ?? {
                ...DEFAULT_NODE_INSPECTOR_SESSION,
                showAdvanced: preferredAdvancedControls,
                readsContextDraft: parsedSelectedNodeReadsContext.keys.join('\n'),
                readsContextError: parsedSelectedNodeReadsContext.error,
                writesContextDraft: parsedSelectedNodeWritesContext.keys.join('\n'),
                writesContextError: parsedSelectedNodeWritesContext.error,
            }
        )
        : {
            ...DEFAULT_NODE_INSPECTOR_SESSION,
            showAdvanced: preferredAdvancedControls,
            readsContextDraft: parsedSelectedNodeReadsContext.keys.join('\n'),
            readsContextError: parsedSelectedNodeReadsContext.error,
            writesContextDraft: parsedSelectedNodeWritesContext.keys.join('\n'),
            writesContextError: parsedSelectedNodeWritesContext.error,
        }
    const showAdvanced = selectedNodeInspectorSession.showAdvanced
    const readsContextDraft = selectedNodeInspectorSession.readsContextDraft
    const readsContextError = selectedNodeInspectorSession.readsContextError
    const writesContextDraft = selectedNodeInspectorSession.writesContextDraft
    const writesContextError = selectedNodeInspectorSession.writesContextError
    const visibility = getNodeFieldVisibility(handlerType)
    const selectedNodeToolHookPreWarning = getToolHookCommandWarning((selectedNode?.data?.["tool.hooks.pre"] as string) || "")
    const selectedNodeToolHookPostWarning = getToolHookCommandWarning((selectedNode?.data?.["tool.hooks.post"] as string) || "")
    const nodeFieldDiagnostics = useMemo(() => {
        if (!selectedNodeId) {
            return {}
        }
        return resolveNodeFieldDiagnostics(diagnostics, selectedNodeId)
    }, [diagnostics, selectedNodeId])
    const edgeFieldDiagnostics = useMemo(() => {
        if (!selectedEdge) {
            return {}
        }
        return resolveEdgeFieldDiagnostics(diagnostics, selectedEdge.source, selectedEdge.target)
    }, [diagnostics, selectedEdge])
    const activeInspectorScope = resolveInspectorScope({
        activeFlow,
        selectedNodeId,
        selectedEdgeId,
    })
    const inspectorTitle = activeInspectorScope === 'edge' ? 'Edge' : activeInspectorScope === 'node' ? 'Node' : activeInspectorScope === 'graph' ? 'Graph' : 'Flows'
    const showGraphInspector = activeInspectorScope === 'graph'
    const showNodeInspector = activeInspectorScope === 'node'
    const showEdgeInspector = activeInspectorScope === 'edge'
    const showSecondaryInspector = showGraphInspector || showNodeInspector || showEdgeInspector

    const handleEdgePropertyChange = (key: string, value: string | boolean) => {
        if (!selectedEdgeId || !activeFlow) return;

        let newEdges: Edge[] = [];
        updateEdges((eds) => {
            newEdges = eds.map((edge) => {
                if (edge.id !== selectedEdgeId) return edge;
                const nextData = { ...(edge.data || {}), [key]: value };
                return {
                    ...edge,
                    data: nextData,
                    label: key === 'label' ? String(value) : edge.label,
                };
            });
            return newEdges;
        });

        if (newEdges.length > 0) {
            scheduleSave({ nodes: readNodes(), edges: newEdges });
        }
    };

    const updateSelectedNodeAttrs = (transform: (attrs: Record<string, unknown>) => Record<string, unknown>) => {
        if (!activeFlow || !selectedNodeId) {
            return
        }
        let newNodes: Node[] = []
        updateNodes((currentNodes) => {
            newNodes = currentNodes.map((node) => {
                if (node.id !== selectedNodeId) {
                    return node
                }
                const nextData = transform({ ...((node.data || {}) as Record<string, unknown>) })
                return applyNodeVisualState(node, nextData)
            })
            return newNodes
        })
        if (newNodes.length > 0) {
            scheduleSave({ nodes: newNodes, edges: readEdges() })
        }
    }

    const handleNodeExtensionValueChange = (key: string, value: string) => {
        updateSelectedNodeAttrs((attrs) => ({
            ...attrs,
            [key]: value,
        }))
    }

    const handleReadsContextChange = (value: string) => {
        const parsed = parseContextKeyDraft(value)
        if (selectedNodeId) {
            updateEditorNodeInspectorSession(selectedNodeId, {
                readsContextDraft: value,
                readsContextError: parsed.error,
            })
        }
        if (parsed.error) {
            return
        }
        handlePropertyChange('spark.reads_context', serializeContextKeyList(parsed.keys))
    }

    const handleWritesContextChange = (value: string) => {
        const parsed = parseContextKeyDraft(value)
        if (selectedNodeId) {
            updateEditorNodeInspectorSession(selectedNodeId, {
                writesContextDraft: value,
                writesContextError: parsed.error,
            })
        }
        if (parsed.error) {
            return
        }
        handlePropertyChange('spark.writes_context', serializeContextKeyList(parsed.keys))
    }

    const handleNodeExtensionRemove = (key: string) => {
        updateSelectedNodeAttrs((attrs) => {
            delete attrs[key]
            return attrs
        })
    }

    const handleNodeExtensionAdd = (key: string, value: string) => {
        updateSelectedNodeAttrs((attrs) => ({
            ...attrs,
            [key]: value,
        }))
    }

    const updateSelectedEdgeAttrs = (transform: (attrs: Record<string, unknown>) => Record<string, unknown>) => {
        if (!activeFlow || !selectedEdgeId) {
            return
        }
        let newEdges: Edge[] = []
        updateEdges((currentEdges) => {
            newEdges = currentEdges.map((edge) => {
                if (edge.id !== selectedEdgeId) {
                    return edge
                }
                const nextData = transform({ ...((edge.data || {}) as Record<string, unknown>) })
                return {
                    ...edge,
                    data: nextData,
                    label: typeof nextData.label === 'string' ? nextData.label : edge.label,
                }
            })
            return newEdges
        })
        if (newEdges.length > 0) {
            scheduleSave({ nodes: readNodes(), edges: newEdges })
        }
    }

    const handleEdgeExtensionValueChange = (key: string, value: string) => {
        updateSelectedEdgeAttrs((attrs) => ({
            ...attrs,
            [key]: value,
        }))
    }

    const handleEdgeExtensionRemove = (key: string) => {
        updateSelectedEdgeAttrs((attrs) => {
            delete attrs[key]
            return attrs
        })
    }

    const handleEdgeExtensionAdd = (key: string, value: string) => {
        updateSelectedEdgeAttrs((attrs) => ({
            ...attrs,
            [key]: value,
        }))
    }

    const renderFieldDiagnostics = (
        scope: 'node' | 'edge',
        field: string,
        fieldDiagnostics: Record<string, DiagnosticEntry[]>,
        testId: string,
    ) => {
        const diagnosticsForField = fieldDiagnostics[field] || []
        if (diagnosticsForField.length === 0) {
            return null
        }
        return (
            <div
                data-testid={testId}
                className="space-y-1 rounded-md border border-border/80 px-3 py-2"
            >
                {diagnosticsForField.map((diag, index) => {
                    const severityClassName = diag.severity === 'error'
                        ? 'text-destructive'
                        : diag.severity === 'warning'
                            ? 'text-warning'
                            : 'text-info'
                    return (
                        <p key={`${scope}-${field}-${diag.rule_id}-${index}`} className={`text-xs ${severityClassName}`}>
                            {diag.message}
                        </p>
                    )
                })}
            </div>
        )
    }

    // The flow list lives in the Flows panel; this column only inspects the open flow.
    if (!showSecondaryInspector) {
        return null
    }

    return (
        <nav
            data-testid="inspector-panel"
            data-inspector-active-scope={activeInspectorScope}
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`bg-background flex flex-col shrink-0 overflow-hidden z-40 ${
                isNarrowViewport ? 'w-full max-h-[46vh] border-b' : 'border-r'
            }`}
            style={isNarrowViewport ? undefined : { width: `${desktopWidthPx}px` }}
        >
            <div className="px-4 pb-2 pt-4">
                <div className="text-xs font-medium uppercase tracking-[0.2em] text-muted-foreground">{inspectorTitle}</div>
            </div>

            <div className="min-h-0 flex-1 flex flex-col overflow-hidden">
                {showGraphInspector ? (
                    <GraphInspectorPanel />
                ) : null}

                {showNodeInspector ? (
                    <NodeInspectorPanel
                        selectedNodeId={selectedNodeId}
                        selectedNode={selectedNode}
                        visibility={visibility}
                        readsContextDraft={readsContextDraft}
                        readsContextError={readsContextError}
                        writesContextDraft={writesContextDraft}
                        writesContextError={writesContextError}
                        showAdvanced={showAdvanced}
                        nodeFieldDiagnostics={nodeFieldDiagnostics}
                        selectedNodeExtensionEntries={selectedNodeExtensionEntries}
                        selectedNodeToolHookPreWarning={selectedNodeToolHookPreWarning}
                        selectedNodeToolHookPostWarning={selectedNodeToolHookPostWarning}
                        onPropertyChange={handlePropertyChange}
                        onOpenGraphChildSettings={openGraphChildSettings}
                        onReadsContextChange={handleReadsContextChange}
                        onWritesContextChange={handleWritesContextChange}
                        onSetShowAdvanced={(value) => {
                            if (!selectedNodeId) {
                                return
                            }
                            updateEditorNodeInspectorSession(selectedNodeId, {
                                showAdvanced: typeof value === 'function'
                                    ? value(showAdvanced)
                                    : value,
                            })
                        }}
                        onNodeExtensionValueChange={handleNodeExtensionValueChange}
                        onNodeExtensionRemove={handleNodeExtensionRemove}
                        onNodeExtensionAdd={handleNodeExtensionAdd}
                        renderFieldDiagnostics={renderFieldDiagnostics}
                    />
                ) : null}

                {showEdgeInspector ? (
                    <EdgeInspectorPanel
                        selectedEdge={selectedEdge}
                        selectedEdgeExtensionEntries={selectedEdgeExtensionEntries}
                        edgeFieldDiagnostics={edgeFieldDiagnostics}
                        selectedEdgeConditionDiagnostics={selectedEdgeConditionDiagnostics}
                        conditionPreviewHasError={conditionPreviewHasError}
                        conditionPreviewHasWarning={conditionPreviewHasWarning}
                        onPropertyChange={handleEdgePropertyChange}
                        onExtensionValueChange={handleEdgeExtensionValueChange}
                        onExtensionRemove={handleEdgeExtensionRemove}
                        onExtensionAdd={handleEdgeExtensionAdd}
                        renderFieldDiagnostics={renderFieldDiagnostics}
                    />
                ) : null}
            </div>
        </nav>
    )
}
