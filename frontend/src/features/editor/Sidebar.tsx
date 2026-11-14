import { useStore, type DiagnosticEntry } from "@/store"
import { useEffect, useMemo, useRef } from "react"
import { useReactFlow, useStore as useReactFlowStore, type Edge, type Node } from "@xyflow/react"
import {
    buildCanonicalFlowModelFromEditorState,
    buildFlowDefinitionFromCanonicalFlowModel,
    EDGE_CORE_ATTR_KEYS,
    NODE_CORE_ATTR_KEYS,
} from "@/lib/canonicalFlowModel"
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
import { filterAuthoredEdges, filterAuthoredNodes } from '@/features/workflow-canvas/derivedPreview'
import { useEditorGraphBridgeRef } from './EditorGraphBridgeContext'
import { flowLoadFailed } from './model/flowLoadFailure'

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
const NODE_KIND_TO_SHAPE: Record<string, string> = {
    start: 'Mdiamond',
    exit: 'Msquare',
    agent_task: 'box',
    human_gate: 'hexagon',
    switch: 'diamond',
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

export type NodePropertyValue = string | boolean | Record<string, unknown>

// `config.<field>` keys edit the node's typed config directly; an empty
// value removes the field.
export function applyNodePropertyChangeToData(
    data: Record<string, unknown>,
    key: string,
    value: NodePropertyValue,
): Record<string, unknown> {
    const existingConfig = data.config && typeof data.config === 'object'
        ? data.config as Record<string, unknown>
        : {}
    if (key.startsWith('config.')) {
        const nextConfig = { ...existingConfig }
        const field = key.slice('config.'.length)
        if (value === '' || value === false) {
            delete nextConfig[field]
        } else {
            nextConfig[field] = value
        }
        return { ...data, config: nextConfig }
    }
    const nextData = { ...data, [key]: value }
    if (key === 'kind' && typeof value === 'string') {
        nextData.shape = NODE_KIND_TO_SHAPE[value] ?? 'box'
        // Each kind has its own config fields; the server rejects foreign ones.
        nextData.config = { kind: value }
        if (value === 'subflow' && typeof nextData.flow_ref !== 'string') {
            nextData.flow_ref = typeof existingConfig.flow_ref === 'string' ? existingConfig.flow_ref : 'child'
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
    // The file view owns edits in Files mode, and takes this column's room: a structured save
    // here would leave its file text stale.
    const isFilesMode = useStore((state) => state.editorMode === 'files')
    const setSelectedNodeId = useStore((state) => state.setSelectedNodeId)
    const setSelectedEdgeId = useStore((state) => state.setSelectedEdgeId)
    const isNarrowViewport = useNarrowViewport()
    const diagnostics = useStore((state) => state.diagnostics)
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
    const navRef = useRef<HTMLElement>(null)
    // Opening a step or route from the inspector moves focus to the inspector that replaces it.
    const focusInspectorRef = useRef(false)
    useEffect(() => {
        if (!focusInspectorRef.current) return
        focusInspectorRef.current = false
        navRef.current?.querySelector<HTMLElement>('h2')?.focus()
    }, [selectedNodeId, selectedEdgeId])
    const { scheduleSave } = useFlowSaveScheduler<FlowGraphSnapshot>({
        flowName: activeFlow,
        debounceMs: INSPECTOR_SAVE_DEBOUNCE_MS,
        buildContent: (snapshot, currentFlowName) => ({
            flow: buildFlowDefinitionFromCanonicalFlowModel(currentFlowName, buildCanonicalFlowModelFromEditorState(currentFlowName, {
                nodes: snapshot?.nodes || [],
                edges: snapshot?.edges || [],
                flowMetadata,
                flow: editorGraphBridgeRef?.current?.getFlow(),
            })),
        }),
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

    const updateNodeProperty = (nodeId: string, key: string, value: NodePropertyValue) => {
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

    const handlePropertyChange = (key: string, value: NodePropertyValue) => {
        if (!selectedNodeId) return;
        updateNodeProperty(selectedNodeId, key, value)
    }

    // Flow settings show even when the preference keeps them closed on open.
    const openGraphChildSettings = () => {
        setSelectedEdgeId(null)
        setSelectedNodeId(null)
        if (activeFlow) useStore.getState().setEditorGraphSettingsPanelOpen(activeFlow, true)
    }

    // As a click on the canvas would: it alone is selected, and the inspector shows it.
    const openOnCanvas = (nodeId: string | null, edgeId: string | null) => {
        setSelectedNodeId(nodeId)
        setSelectedEdgeId(edgeId)
        updateNodes((current) => current.map((node) => (
            node.selected === (node.id === nodeId) ? node : { ...node, selected: node.id === nodeId }
        )))
        updateEdges((current) => current.map((edge) => (
            edge.selected === (edge.id === edgeId) ? edge : { ...edge, selected: edge.id === edgeId }
        )))
        focusInspectorRef.current = true
    }

    const addRoute = (target: string, decision: string) => {
        if (!selectedNodeId) return
        editorGraphBridgeRef?.current?.connect({
            source: selectedNodeId,
            target,
            sourceHandle: null,
            targetHandle: null,
            ...(decision ? { data: { decision } } : {}),
        })
    }

    const selectedNode = nodes.find(n => n.id === selectedNodeId);
    const selectedEdge = edges.find(e => e.id === selectedEdgeId);
    const selectedNodeExtensionEntries = useMemo(
        () => toExtensionAttrEntries(
            (selectedNode?.data ?? {}) as Record<string, unknown>,
            NODE_CORE_ATTR_KEYS,
        ),
        [selectedNode?.data],
    )
    const selectedEdgeExtensionEntries = useMemo(
        () => toExtensionAttrEntries(
            (selectedEdge?.data ?? {}) as Record<string, unknown>,
            EDGE_CORE_ATTR_KEYS,
        ),
        [selectedEdge?.data],
    )
    // A step goes by its label, as the canvas shows it.
    const stepTitle = (nodeId: string) => {
        const label = nodes.find((node) => node.id === nodeId)?.data?.label
        return typeof label === 'string' && label.trim() ? label.trim() : nodeId
    }
    // A fork's connections are its branches and join; other steps' are routes, named by decision.
    const selectedNodeRoutes = filterAuthoredEdges(edges).filter((edge) => edge.source === selectedNodeId).map((edge) => ({
        id: edge.id,
        name: selectedNode?.data?.kind === 'parallel'
            ? (edge.data?.kind === 'join' ? 'join' : 'branch')
            : String(edge.data?.decision || 'next'),
        label: typeof edge.data?.label === 'string' ? edge.data.label : '',
        target: stepTitle(edge.target),
    }))
    // Nothing routes back into a start step.
    const routeTargets = filterAuthoredNodes(nodes)
        .filter((node) => node.data?.kind !== 'start')
        .map((node) => ({ id: node.id, title: stepTitle(node.id) }))
    const selectedEdgeIsFork = nodes.find((node) => node.id === selectedEdge?.source)?.data?.kind === 'parallel'
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
        return resolveEdgeFieldDiagnostics(
            diagnostics,
            selectedEdge.source,
            selectedEdge.target,
            typeof selectedEdge.data?.decision === 'string' ? selectedEdge.data.decision : undefined,
        )
    }, [diagnostics, selectedEdge])
    const activeInspectorScope = resolveInspectorScope({
        activeFlow,
        selectedNodeId,
        selectedEdgeId,
    })
    const showGraphInspector = activeInspectorScope === 'graph'
    const showNodeInspector = activeInspectorScope === 'node'
    const showEdgeInspector = activeInspectorScope === 'edge'
    const flowFailed = useStore((state) => flowLoadFailed(state.flowLoadFailure, state.activeFlow, state.activeProjectPath))
    const showSecondaryInspector = !flowFailed && (showGraphInspector || showNodeInspector || showEdgeInspector)

    const handleEdgePropertyChange = (key: string, value: string | boolean) => {
        if (!selectedEdgeId || !activeFlow) return;

        let newEdges: Edge[] = [];
        updateEdges((eds) => {
            const selected = eds.find((edge) => edge.id === selectedEdgeId);
            newEdges = eds.map((edge) => {
                // A fork has one join; choosing a new one demotes the old to a branch.
                if (key === 'kind' && value === 'join' && edge.id !== selectedEdgeId
                    && edge.source === selected?.source && edge.data?.kind === 'join') {
                    return { ...edge, data: { ...edge.data, kind: 'branch' } };
                }
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

    // Seed a new session from the derived one, or its drafts start empty and the other list's keys vanish from view.
    const updateSelectedNodeInspectorSession = (patch: Partial<typeof selectedNodeInspectorSession>) => {
        if (selectedNodeId) {
            updateEditorNodeInspectorSession(selectedNodeId, { ...selectedNodeInspectorSession, ...patch })
        }
    }

    const handleReadsContextChange = (value: string) => {
        const parsed = parseContextKeyDraft(value)
        updateSelectedNodeInspectorSession({
            readsContextDraft: value,
            readsContextError: parsed.error,
        })
        if (parsed.error) {
            return
        }
        handlePropertyChange('spark.reads_context', serializeContextKeyList(parsed.keys))
    }

    const handleWritesContextChange = (value: string) => {
        const parsed = parseContextKeyDraft(value)
        updateSelectedNodeInspectorSession({
            writesContextDraft: value,
            writesContextError: parsed.error,
        })
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
                className="space-y-1 rounded-lg bg-muted px-3 py-2"
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
    if (!showSecondaryInspector || isFilesMode) {
        return null
    }

    return (
        <nav
            ref={navRef}
            data-testid="inspector-panel"
            data-inspector-active-scope={activeInspectorScope}
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`bg-background flex flex-col shrink-0 overflow-hidden z-40 ${
                isNarrowViewport ? 'w-full max-h-[40vh] border-t' : 'border-r'
            }`}
            style={isNarrowViewport ? undefined : { width: `${desktopWidthPx}px` }}
        >
            <div className="flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden">
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
                        selectedNodeRoutes={selectedNodeRoutes}
                        routeTargets={routeTargets}
                        onOpenRoute={(edgeId) => openOnCanvas(null, edgeId)}
                        onAddRoute={addRoute}
                        onPropertyChange={handlePropertyChange}
                        onOpenGraphChildSettings={openGraphChildSettings}
                        onReadsContextChange={handleReadsContextChange}
                        onWritesContextChange={handleWritesContextChange}
                        onSetShowAdvanced={(value) => updateSelectedNodeInspectorSession({
                            showAdvanced: typeof value === 'function' ? value(showAdvanced) : value,
                        })}
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
                        isForkEdge={selectedEdgeIsFork}
                        stepTitle={stepTitle}
                        onOpenStep={(nodeId) => openOnCanvas(nodeId, null)}
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
