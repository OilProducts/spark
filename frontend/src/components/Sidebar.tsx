import { useStore, type DiagnosticEntry } from "@/store"
import { FilePlus, Trash2 } from "lucide-react"
import { useEffect, useMemo, useState } from "react"
import { useReactFlow, useStore as useReactFlowStore, type Edge, type Node } from "@xyflow/react"
import { generateDot, sanitizeGraphId } from "@/lib/dotUtils"
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from "@/lib/llmSuggestions"
import { getHandlerType, getNodeFieldVisibility } from "@/lib/nodeVisibility"
import { getToolHookCommandWarning } from "@/lib/graphAttrValidation"
import { resolveEdgeFieldDiagnostics, resolveNodeFieldDiagnostics } from "@/lib/inspectorFieldDiagnostics"
import { toExtensionAttrEntries } from "@/lib/extensionAttrs"
import { saveFlowContent } from "@/lib/flowPersistence"
import { deleteFlowValidated, fetchFlowListValidated } from '@/lib/attractorClient'
import { useNarrowViewport } from '@/lib/useNarrowViewport'
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler'
import { InspectorScaffold, InspectorEmptyState } from './InspectorScaffold'
import { GraphSettings } from './GraphSettings'
import { AdvancedKeyValueEditor } from './AdvancedKeyValueEditor'
import { ContextKeyListEditor } from './ContextKeyListEditor'
import {
    parseContextKeyDraft,
    parseContextKeyList,
    serializeContextKeyList,
} from '@/lib/flowContracts'

type InspectorScope = 'none' | 'graph' | 'node' | 'edge'
const INSPECTOR_SAVE_DEBOUNCE_MS = 600
type FlowGraphSnapshot = {
    nodes: Node[]
    edges: Edge[]
}
const CORE_NODE_ATTR_KEYS = new Set<string>([
    'label',
    'shape',
    'prompt',
    'tool.command',
    'tool.hooks.pre',
    'tool.hooks.post',
    'tool.artifacts.paths',
    'tool.artifacts.stdout',
    'tool.artifacts.stderr',
    'join_policy',
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
    'reasoning_effort',
    'auto_status',
    'allow_partial',
    'manager.poll_interval',
    'manager.max_cycles',
    'manager.stop_condition',
    'manager.actions',
    'human.default_choice',
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
const EXCLUDED_NODE_EXTENSION_ATTR_KEYS = new Set<string>(['status'])

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

export function Sidebar() {
    const activeFlow = useStore((state) => state.activeFlow)
    const executionFlow = useStore((state) => state.executionFlow)
    const setActiveFlow = useStore((state) => state.setActiveFlow)
    const setExecutionFlow = useStore((state) => state.setExecutionFlow)
    const selectedNodeId = useStore((state) => state.selectedNodeId)
    const selectedEdgeId = useStore((state) => state.selectedEdgeId)
    const setSelectedNodeId = useStore((state) => state.setSelectedNodeId)
    const setSelectedEdgeId = useStore((state) => state.setSelectedEdgeId)
    const isNarrowViewport = useNarrowViewport()
    const diagnostics = useStore((state) => state.diagnostics)
    const edgeDiagnostics = useStore((state) => state.edgeDiagnostics)
    const graphAttrs = useStore((state) => state.graphAttrs)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const [flows, setFlows] = useState<string[]>([])
    const [showAdvanced, setShowAdvanced] = useState(false)
    const [readsContextDraft, setReadsContextDraft] = useState('')
    const [writesContextDraft, setWritesContextDraft] = useState('')
    const [readsContextError, setReadsContextError] = useState<string | null>(null)
    const [writesContextError, setWritesContextError] = useState<string | null>(null)
    const { getNodes, setNodes, getEdges, setEdges } = useReactFlow()
    const nodes = useReactFlowStore((state) => state.nodes)
    const edges = useReactFlowStore((state) => state.edges)
    const { scheduleSave } = useFlowSaveScheduler<FlowGraphSnapshot>({
        flowName: activeFlow,
        debounceMs: INSPECTOR_SAVE_DEBOUNCE_MS,
        buildContent: (snapshot, currentFlowName) => generateDot(
            currentFlowName,
            snapshot?.nodes || [],
            snapshot?.edges || [],
            graphAttrs,
        ),
    })

    const loadFlows = async () => {
        try {
            const data = await fetchFlowListValidated()
            setFlows(data)
        } catch (error) {
            console.error(error)
        }
    }

    useEffect(() => {
        void loadFlows()
    }, [])

    const createNewFlow = async () => {
        const name = prompt("Enter flow name (e.g., demo.dot)");
        if (!name) return;

        // Auto-append .dot if missing
        const fileName = name.endsWith('.dot') ? name : `${name}.dot`;
        const graphName = sanitizeGraphId(fileName);

        const escapeDot = (value: string) => value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')
        const uiAttrLines = [
            uiDefaults.llm_model ? `ui_default_llm_model="${escapeDot(uiDefaults.llm_model)}"` : '',
            uiDefaults.llm_provider ? `ui_default_llm_provider="${escapeDot(uiDefaults.llm_provider)}"` : '',
            uiDefaults.reasoning_effort ? `ui_default_reasoning_effort="${escapeDot(uiDefaults.reasoning_effort)}"` : '',
        ].filter(Boolean)
        const graphAttrBlock = uiAttrLines.length ? `  graph [${uiAttrLines.join(', ')}];\n` : ''

        const initialContent = `digraph ${graphName} {\n${graphAttrBlock}  start [shape=Mdiamond, label="Start"];\n  end [shape=Msquare, label="End"];\n  start -> end;\n}`;

        const saved = await saveFlowContent(fileName, initialContent)
        if (!saved) return

        await loadFlows();
        setActiveFlow(fileName);
    }

    const handleDeleteFlow = async (e: React.MouseEvent, fileName: string) => {
        e.stopPropagation();
        if (!window.confirm(`Are you sure you want to delete ${fileName}?`)) return;

        await deleteFlowValidated(fileName);

        if (activeFlow === fileName) {
            setActiveFlow(null);
        }
        if (executionFlow === fileName) {
            setExecutionFlow(null)
        }
        await loadFlows();
    };

    const updateNodeProperty = (nodeId: string, key: string, value: string | boolean) => {
        if (!activeFlow) return;

        let newNodes: Node[] = [];
        setNodes(nds => {
            newNodes = nds.map(node => {
                if (node.id === nodeId) {
                    return { ...node, data: { ...node.data, [key]: value } };
                }
                return node;
            });
            return newNodes;
        });

        if (newNodes.length > 0) {
            scheduleSave({ nodes: newNodes, edges: getEdges() });
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
    const isTrue = (value: unknown) => value === true || value === 'true';
    const handlerType = getHandlerType(
        (selectedNode?.data?.shape as string) || '',
        (selectedNode?.data?.type as string) || ''
    )
    const selectedNodeReadsContextRaw = typeof selectedNode?.data?.['spark.reads_context'] === 'string'
        ? selectedNode.data['spark.reads_context']
        : ''
    const selectedNodeWritesContextRaw = typeof selectedNode?.data?.['spark.writes_context'] === 'string'
        ? selectedNode.data['spark.writes_context']
        : ''
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
    const activeTab = activeInspectorScope === 'edge' ? 'edge' : activeInspectorScope === 'node' ? 'edit' : 'flows'
    const inspectorTitle = activeInspectorScope === 'edge' ? 'Edge' : activeInspectorScope === 'node' ? 'Node' : activeInspectorScope === 'graph' ? 'Graph' : 'Flows'

    useEffect(() => {
        const parsedReads = parseContextKeyList(selectedNodeReadsContextRaw)
        setReadsContextDraft(parsedReads.keys.join('\n'))
        setReadsContextError(parsedReads.error)
    }, [selectedNodeId, selectedNodeReadsContextRaw])

    useEffect(() => {
        const parsedWrites = parseContextKeyList(selectedNodeWritesContextRaw)
        setWritesContextDraft(parsedWrites.keys.join('\n'))
        setWritesContextError(parsedWrites.error)
    }, [selectedNodeId, selectedNodeWritesContextRaw])

    const handleEdgePropertyChange = (key: string, value: string | boolean) => {
        if (!selectedEdgeId || !activeFlow) return;

        let newEdges: Edge[] = [];
        setEdges((eds) => {
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
            scheduleSave({ nodes: getNodes(), edges: newEdges });
        }
    };

    const updateSelectedNodeAttrs = (transform: (attrs: Record<string, unknown>) => Record<string, unknown>) => {
        if (!activeFlow || !selectedNodeId) {
            return
        }
        let newNodes: Node[] = []
        setNodes((currentNodes) => {
            newNodes = currentNodes.map((node) => {
                if (node.id !== selectedNodeId) {
                    return node
                }
                const nextData = transform({ ...((node.data || {}) as Record<string, unknown>) })
                return {
                    ...node,
                    data: nextData,
                }
            })
            return newNodes
        })
        if (newNodes.length > 0) {
            scheduleSave({ nodes: newNodes, edges: getEdges() })
        }
    }

    const handleNodeExtensionValueChange = (key: string, value: string) => {
        updateSelectedNodeAttrs((attrs) => ({
            ...attrs,
            [key]: value,
        }))
    }

    const handleReadsContextChange = (value: string) => {
        setReadsContextDraft(value)
        const parsed = parseContextKeyDraft(value)
        setReadsContextError(parsed.error)
        if (parsed.error) {
            return
        }
        handlePropertyChange('spark.reads_context', serializeContextKeyList(parsed.keys))
    }

    const handleWritesContextChange = (value: string) => {
        setWritesContextDraft(value)
        const parsed = parseContextKeyDraft(value)
        setWritesContextError(parsed.error)
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
        setEdges((currentEdges) => {
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
            scheduleSave({ nodes: getNodes(), edges: newEdges })
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
                className="space-y-1 rounded-md border border-border/80 bg-muted/20 px-3 py-2"
            >
                {diagnosticsForField.map((diag, index) => {
                    const severityClassName = diag.severity === 'error'
                        ? 'text-destructive'
                        : diag.severity === 'warning'
                            ? 'text-amber-800'
                            : 'text-sky-700'
                    return (
                        <p key={`${scope}-${field}-${diag.rule_id}-${index}`} className={`text-[11px] ${severityClassName}`}>
                            {diag.message}
                        </p>
                    )
                })}
            </div>
        )
    }

    return (
        <nav
            data-testid="inspector-panel"
            data-inspector-active-scope={activeInspectorScope}
            data-responsive-layout={isNarrowViewport ? 'stacked' : 'split'}
            className={`bg-background flex flex-col shrink-0 overflow-hidden z-40 ${
                isNarrowViewport ? 'w-full max-h-[46vh] border-b' : 'w-72 border-r'
            }`}
        >
            <div className="px-4 pb-2 pt-4">
                <div className="flex items-center gap-3 text-xs font-semibold uppercase tracking-[0.2em] text-foreground">
                    <span>{inspectorTitle}</span>
                    <span className="h-2 w-2 rounded-full bg-muted-foreground/40" />
                </div>
            </div>

            {activeTab === 'flows' && (
                <div className="flex-1 flex flex-col overflow-hidden">
                    <div className="px-5 py-2 flex items-center justify-between">
                        <h2 className="font-semibold text-sm tracking-tight">Saved Flows</h2>
                        <button onClick={createNewFlow} className="h-8 px-2 text-muted-foreground hover:text-foreground" title="New Flow">
                            <FilePlus className="w-4 h-4" />
                        </button>
                    </div>
                    <div className="flex-1 overflow-y-auto px-3 pb-4 space-y-4">
                        <div className="space-y-1">
                            {flows.map(f => (
                                <div key={f} className="relative group">
                                    <button
                                        onClick={() => {
                                            setActiveFlow(f)
                                        }}
                                        className={`w-full text-left px-3 py-2 pr-8 rounded-md text-sm transition-colors ${activeFlow === f
                                            ? 'bg-secondary text-secondary-foreground font-medium'
                                            : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                                            }`}
                                    >
                                        <span className="flex items-center gap-2">{f}</span>
                                    </button>
                                    <button
                                        onClick={(e) => handleDeleteFlow(e, f)}
                                        className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-muted-foreground opacity-0 group-hover:opacity-100 hover:text-destructive transition-all"
                                        title="Delete Flow"
                                    >
                                        <Trash2 className="w-3.5 h-3.5" />
                                    </button>
                                </div>
                            ))}
                        </div>
                        {activeInspectorScope === 'graph' && (
                            <GraphSettings inline />
                        )}
                    </div>
                </div>
            )}

            {activeTab === 'edit' && (
                <div className="flex-1 overflow-y-auto px-5 pb-5 pt-3">
                    <InspectorScaffold
                        scopeLabel="Node"
                        title="Configuration"
                        description="Use the same inspect-edit flow as graph and edge inspectors."
                        entityLabel="Node ID"
                        entityValue={selectedNodeId || undefined}
                    >
                        {!selectedNodeId ? (
                            <InspectorEmptyState message="Select a component on the canvas to inspect and edit its properties." />
                        ) : (
                            <div data-testid="node-structured-form" className="space-y-4">
                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Label</label>
                                    <input
                                        value={(selectedNode?.data?.label as string) || ''}
                                        onChange={(e) => handlePropertyChange('label', e.target.value)}
                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    />
                                </div>

                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Shape / Type</label>
                                    <select
                                        value={(selectedNode?.data?.shape as string) || 'box'}
                                        onChange={(e) => handlePropertyChange('shape', e.target.value)}
                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    >
                                        <option value="box">Codergen (Task)</option>
                                        <option value="hexagon">Wait for Human</option>
                                        <option value="diamond">Condition</option>
                                        <option value="component">Parallel (Fan Out)</option>
                                        <option value="tripleoctagon">Parallel (Fan In)</option>
                                        <option value="parallelogram">Tool</option>
                                        <option value="house">Manager Loop</option>
                                        <option value="Mdiamond">Start Node</option>
                                        <option value="Msquare">End Node</option>
                                    </select>
                                </div>

                                {visibility.showPrompt && (
                                    <div className="space-y-1.5 flex flex-col h-48">
                                        <label className="text-sm font-medium">Prompt Instruction</label>
                                        <textarea
                                            value={(selectedNode?.data?.prompt as string) || ''}
                                            onChange={(e) => handlePropertyChange('prompt', e.target.value)}
                                            className="flex flex-1 w-full rounded-md border border-input bg-background px-3 py-2 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring resize-none"
                                            placeholder="Enter system prompt instructions..."
                                        />
                                        {renderFieldDiagnostics('node', 'prompt', nodeFieldDiagnostics, 'node-field-diagnostics-prompt')}
                                    </div>
                                )}
                                {(selectedNode?.data?.shape as string) !== 'Mdiamond' && (selectedNode?.data?.shape as string) !== 'Msquare' && (
                                    <div className="space-y-3">
                                        <ContextKeyListEditor
                                            testId="node-reads-context-editor"
                                            title="Reads Context"
                                            description="Declare the `context.*` keys this node expects to consume from launch state or earlier stages."
                                            value={readsContextDraft}
                                            error={readsContextError}
                                            onChange={handleReadsContextChange}
                                        />
                                        <ContextKeyListEditor
                                            testId="node-writes-context-editor"
                                            title="Writes Context"
                                            description="Declare the `context.*` keys this node is expected to produce for later stages."
                                            value={writesContextDraft}
                                            error={writesContextError}
                                            onChange={handleWritesContextChange}
                                        />
                                    </div>
                                )}
                                {visibility.showToolCommand && (
                                    <div className="space-y-1.5">
                                        <label className="text-sm font-medium">Tool Command</label>
                                        <input
                                            value={(selectedNode?.data?.['tool.command'] as string) || ''}
                                            onChange={(e) => handlePropertyChange('tool.command', e.target.value)}
                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            placeholder="e.g. pytest -q"
                                        />
                                    </div>
                                )}
                                {visibility.showParallelOptions && (
                                    <>
                                        <div className="space-y-1.5">
                                            <label className="text-sm font-medium">Join Policy</label>
                                            <select
                                                value={(selectedNode?.data?.join_policy as string) || 'wait_all'}
                                                onChange={(e) => handlePropertyChange('join_policy', e.target.value)}
                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            >
                                                <option value="wait_all">Wait All</option>
                                                <option value="first_success">First Success</option>
                                                <option value="k_of_n">K of N</option>
                                                <option value="quorum">Quorum</option>
                                            </select>
                                        </div>
                                        <div className="space-y-1.5">
                                            <label className="text-sm font-medium">Error Policy</label>
                                            <select
                                                value={(selectedNode?.data?.error_policy as string) || 'continue'}
                                                onChange={(e) => handlePropertyChange('error_policy', e.target.value)}
                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            >
                                                <option value="continue">Continue</option>
                                                <option value="fail_fast">Fail Fast</option>
                                                <option value="ignore">Ignore</option>
                                            </select>
                                        </div>
                                        <div className="space-y-1.5">
                                            <label className="text-sm font-medium">Max Parallel</label>
                                            <input
                                                value={(selectedNode?.data?.max_parallel as number | string | undefined) ?? 4}
                                                onChange={(e) => handlePropertyChange('max_parallel', e.target.value)}
                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            />
                                        </div>
                                    </>
                                )}
                                {visibility.showManagerOptions && (
                                    <>
                                        <div className="space-y-1.5">
                                            <label className="text-sm font-medium">Manager Poll Interval</label>
                                            <input
                                                value={(selectedNode?.data?.['manager.poll_interval'] as string) || ''}
                                                onChange={(e) => handlePropertyChange('manager.poll_interval', e.target.value)}
                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder="25ms"
                                            />
                                        </div>
                                        <div className="space-y-1.5">
                                            <label className="text-sm font-medium">Manager Max Cycles</label>
                                            <input
                                                value={(selectedNode?.data?.['manager.max_cycles'] as number | string | undefined) ?? ''}
                                                onChange={(e) => handlePropertyChange('manager.max_cycles', e.target.value)}
                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder="3"
                                            />
                                        </div>
                                        <div className="space-y-1.5">
                                            <label className="text-sm font-medium">Manager Stop Condition</label>
                                            <input
                                                value={(selectedNode?.data?.['manager.stop_condition'] as string) || ''}
                                                onChange={(e) => handlePropertyChange('manager.stop_condition', e.target.value)}
                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder='child.status == "success"'
                                            />
                                        </div>
                                        <div className="space-y-1.5">
                                            <label className="text-sm font-medium">Manager Actions</label>
                                            <input
                                                value={(selectedNode?.data?.['manager.actions'] as string) || ''}
                                                onChange={(e) => handlePropertyChange('manager.actions', e.target.value)}
                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder="observe,steer"
                                            />
                                        </div>
                                        <div
                                            data-testid="manager-child-linkage"
                                            className="space-y-2 rounded-md border border-border/80 bg-muted/20 px-3 py-2"
                                        >
                                            <div>
                                                <p className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                                                    Child Pipeline Linkage
                                                </p>
                                                <p className="mt-1 text-[11px] text-muted-foreground">
                                                    Manager loops use <code>stack.child_dotfile</code> and <code>stack.child_workdir</code> from graph attributes.
                                                </p>
                                            </div>
                                            <div className="space-y-1 text-[11px] text-foreground">
                                                <p><span className="font-mono">stack.child_dotfile</span>: {graphAttrs['stack.child_dotfile'] || '(unset)'}</p>
                                                <p><span className="font-mono">stack.child_workdir</span>: {graphAttrs['stack.child_workdir'] || '(unset)'}</p>
                                            </div>
                                            <button
                                                type="button"
                                                data-testid="manager-open-child-settings"
                                                onClick={openGraphChildSettings}
                                                className="rounded border border-border bg-background px-2 py-1 text-[11px] font-medium text-foreground hover:bg-muted"
                                            >
                                                Open Graph Child Settings
                                            </button>
                                        </div>
                                    </>
                                )}
                                {visibility.showHumanDefaultChoice && (
                                    <div className="space-y-1.5">
                                        <label className="text-sm font-medium">Human Default Choice</label>
                                        <input
                                            value={(selectedNode?.data?.['human.default_choice'] as string) || ''}
                                            onChange={(e) => handlePropertyChange('human.default_choice', e.target.value)}
                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            placeholder="target node id"
                                        />
                                        <p
                                            data-testid="human-default-choice-timeout-guidance"
                                            className="text-xs text-muted-foreground"
                                        >
                                            Used when this gate times out without an explicit answer.
                                        </p>
                                    </div>
                                )}
                                {visibility.showTypeOverride && (
                                    <div className="space-y-1.5">
                                        <label className="text-sm font-medium">Handler Type</label>
                                        <input
                                            value={(selectedNode?.data?.type as string) || ''}
                                            onChange={(e) => handlePropertyChange('type', e.target.value)}
                                            list="node-handler-type-options"
                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            placeholder="optional override"
                                        />
                                        <datalist id="node-handler-type-options">
                                            <option value="start">start</option>
                                            <option value="exit">exit</option>
                                            <option value="codergen">codergen</option>
                                            <option value="wait.human">wait.human</option>
                                            <option value="conditional">conditional</option>
                                            <option value="parallel">parallel</option>
                                            <option value="parallel.fan_in">parallel.fan_in</option>
                                            <option value="tool">tool</option>
                                            <option value="stack.manager_loop">stack.manager_loop</option>
                                        </datalist>
                                        {renderFieldDiagnostics('node', 'type', nodeFieldDiagnostics, 'node-field-diagnostics-type')}
                                    </div>
                                )}
                                {visibility.showAdvanced && (
                                    <button
                                        onClick={() => setShowAdvanced((prev) => !prev)}
                                        className="w-full rounded-md border border-border bg-background px-3 py-2 text-xs font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                                    >
                                        {showAdvanced ? 'Hide Advanced' : 'Show Advanced'}
                                    </button>
                                )}
                                {visibility.showAdvanced && showAdvanced && (
                                    <div className="space-y-4">
                                        {visibility.showGeneralAdvanced && (
                                            <>
                                                <div className="grid grid-cols-2 gap-3">
                                                    <div className="space-y-1.5">
                                                        <label className="text-sm font-medium">Max Retries</label>
                                                        <input
                                                            value={(selectedNode?.data?.max_retries as number | string | undefined) ?? ''}
                                                            onChange={(e) => handlePropertyChange('max_retries', e.target.value)}
                                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        />
                                                    </div>
                                                    <div className="space-y-1.5">
                                                        <label className="text-sm font-medium">Timeout</label>
                                                        <input
                                                            value={(selectedNode?.data?.timeout as string) || ''}
                                                            onChange={(e) => handlePropertyChange('timeout', e.target.value)}
                                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                            placeholder="900s"
                                                        />
                                                    </div>
                                                </div>
                                                <div className="flex items-center gap-2">
                                                    <input
                                                        id={`goal-gate-${selectedNodeId}`}
                                                        type="checkbox"
                                                        checked={isTrue(selectedNode?.data?.goal_gate)}
                                                        onChange={(e) => handlePropertyChange('goal_gate', e.target.checked)}
                                                        className="h-4 w-4 rounded border border-input"
                                                    />
                                                    <label htmlFor={`goal-gate-${selectedNodeId}`} className="text-sm font-medium">
                                                        Goal Gate
                                                    </label>
                                                </div>
                                                {renderFieldDiagnostics('node', 'goal_gate', nodeFieldDiagnostics, 'node-field-diagnostics-goal_gate')}
                                                <div className="space-y-1.5">
                                                    <label className="text-sm font-medium">Retry Target</label>
                                                    <input
                                                        value={(selectedNode?.data?.retry_target as string) || ''}
                                                        onChange={(e) => handlePropertyChange('retry_target', e.target.value)}
                                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    />
                                                    {renderFieldDiagnostics('node', 'retry_target', nodeFieldDiagnostics, 'node-field-diagnostics-retry_target')}
                                                </div>
                                                <div className="space-y-1.5">
                                                    <label className="text-sm font-medium">Fallback Retry Target</label>
                                                    <input
                                                        value={(selectedNode?.data?.fallback_retry_target as string) || ''}
                                                        onChange={(e) => handlePropertyChange('fallback_retry_target', e.target.value)}
                                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    />
                                                    {renderFieldDiagnostics('node', 'fallback_retry_target', nodeFieldDiagnostics, 'node-field-diagnostics-fallback_retry_target')}
                                                </div>
                                                {visibility.showToolCommand && (
                                                    <>
                                                        <div className="space-y-1.5">
                                                            <label className="text-sm font-medium">Pre Hook Override</label>
                                                            <input
                                                                data-testid="node-attr-input-tool.hooks.pre"
                                                                value={(selectedNode?.data?.['tool.hooks.pre'] as string) || ''}
                                                                onChange={(e) => handlePropertyChange('tool.hooks.pre', e.target.value)}
                                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                                placeholder="e.g. ./hooks/pre.sh"
                                                            />
                                                            {selectedNodeToolHookPreWarning && (
                                                                <p data-testid="node-attr-warning-tool.hooks.pre" className="text-xs text-amber-800">
                                                                    {selectedNodeToolHookPreWarning}
                                                                </p>
                                                            )}
                                                        </div>
                                                        <div className="space-y-1.5">
                                                            <label className="text-sm font-medium">Post Hook Override</label>
                                                            <input
                                                                data-testid="node-attr-input-tool.hooks.post"
                                                                value={(selectedNode?.data?.['tool.hooks.post'] as string) || ''}
                                                                onChange={(e) => handlePropertyChange('tool.hooks.post', e.target.value)}
                                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                                placeholder="e.g. ./hooks/post.sh"
                                                            />
                                                            {selectedNodeToolHookPostWarning && (
                                                                <p data-testid="node-attr-warning-tool.hooks.post" className="text-xs text-amber-800">
                                                                    {selectedNodeToolHookPostWarning}
                                                                </p>
                                                            )}
                                                        </div>
                                                        <div className="space-y-1.5">
                                                            <label className="text-sm font-medium">Artifact Paths</label>
                                                            <input
                                                                data-testid="node-attr-input-tool.artifacts.paths"
                                                                value={(selectedNode?.data?.['tool.artifacts.paths'] as string) || ''}
                                                                onChange={(e) => handlePropertyChange('tool.artifacts.paths', e.target.value)}
                                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                                placeholder="e.g. dist/**,reports/*.json"
                                                            />
                                                        </div>
                                                        <div className="space-y-1.5">
                                                            <label className="text-sm font-medium">Stdout Artifact</label>
                                                            <input
                                                                data-testid="node-attr-input-tool.artifacts.stdout"
                                                                value={(selectedNode?.data?.['tool.artifacts.stdout'] as string) || ''}
                                                                onChange={(e) => handlePropertyChange('tool.artifacts.stdout', e.target.value)}
                                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                                placeholder="e.g. stdout.txt"
                                                            />
                                                        </div>
                                                        <div className="space-y-1.5">
                                                            <label className="text-sm font-medium">Stderr Artifact</label>
                                                            <input
                                                                data-testid="node-attr-input-tool.artifacts.stderr"
                                                                value={(selectedNode?.data?.['tool.artifacts.stderr'] as string) || ''}
                                                                onChange={(e) => handlePropertyChange('tool.artifacts.stderr', e.target.value)}
                                                                className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                                placeholder="e.g. stderr.txt"
                                                            />
                                                        </div>
                                                    </>
                                                )}
                                                <div className="grid grid-cols-2 gap-3">
                                                    <div className="space-y-1.5">
                                                        <label className="text-sm font-medium">Fidelity</label>
                                                        <input
                                                            value={(selectedNode?.data?.fidelity as string) || ''}
                                                            onChange={(e) => handlePropertyChange('fidelity', e.target.value)}
                                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                            placeholder="full"
                                                        />
                                                        {renderFieldDiagnostics('node', 'fidelity', nodeFieldDiagnostics, 'node-field-diagnostics-fidelity')}
                                                    </div>
                                                    <div className="space-y-1.5">
                                                        <label className="text-sm font-medium">Thread ID</label>
                                                        <input
                                                            value={(selectedNode?.data?.thread_id as string) || ''}
                                                            onChange={(e) => handlePropertyChange('thread_id', e.target.value)}
                                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        />
                                                    </div>
                                                </div>
                                                <div className="space-y-1.5">
                                                    <label className="text-sm font-medium">Class</label>
                                                    <input
                                                        value={(selectedNode?.data?.class as string) || ''}
                                                        onChange={(e) => handlePropertyChange('class', e.target.value)}
                                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    />
                                                </div>
                                            </>
                                        )}
                                        {visibility.showLlmSettings && (
                                            <>
                                                <div className="grid grid-cols-2 gap-3">
                                                    <div className="space-y-1.5">
                                                        <label className="text-sm font-medium">LLM Model</label>
                                                        <input
                                                            value={(selectedNode?.data?.llm_model as string) || ''}
                                                            onChange={(e) => handlePropertyChange('llm_model', e.target.value)}
                                                            list="llm-model-options-panel"
                                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        />
                                                        <datalist id="llm-model-options-panel">
                                                            {getModelSuggestions((selectedNode?.data?.llm_provider as string) || '').map((model) => (
                                                                <option key={model} value={model} />
                                                            ))}
                                                        </datalist>
                                                    </div>
                                                    <div className="space-y-1.5">
                                                        <label className="text-sm font-medium">LLM Provider</label>
                                                        <input
                                                            value={(selectedNode?.data?.llm_provider as string) || ''}
                                                            onChange={(e) => handlePropertyChange('llm_provider', e.target.value)}
                                                            list="llm-provider-options-panel"
                                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        />
                                                        <datalist id="llm-provider-options-panel">
                                                            {LLM_PROVIDER_OPTIONS.map((provider) => (
                                                                <option key={provider} value={provider} />
                                                            ))}
                                                        </datalist>
                                                    </div>
                                                </div>
                                                <div className="space-y-1.5">
                                                    <label className="text-sm font-medium">Reasoning Effort</label>
                                                    <input
                                                        value={(selectedNode?.data?.reasoning_effort as string) || ''}
                                                        onChange={(e) => handlePropertyChange('reasoning_effort', e.target.value)}
                                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        placeholder="high"
                                                    />
                                                </div>
                                            </>
                                        )}
                                        {visibility.showGeneralAdvanced && (
                                            <div className="flex items-center gap-4">
                                                <label className="flex items-center gap-2 text-sm font-medium">
                                                    <input
                                                        type="checkbox"
                                                        checked={isTrue(selectedNode?.data?.auto_status)}
                                                        onChange={(e) => handlePropertyChange('auto_status', e.target.checked)}
                                                        className="h-4 w-4 rounded border border-input"
                                                    />
                                                    Auto Status
                                                </label>
                                                <label className="flex items-center gap-2 text-sm font-medium">
                                                    <input
                                                        type="checkbox"
                                                        checked={isTrue(selectedNode?.data?.allow_partial)}
                                                        onChange={(e) => handlePropertyChange('allow_partial', e.target.checked)}
                                                        className="h-4 w-4 rounded border border-input"
                                                    />
                                                    Allow Partial
                                                </label>
                                            </div>
                                        )}
                                    </div>
                                )}
                                <AdvancedKeyValueEditor
                                    testIdPrefix="node"
                                    entries={selectedNodeExtensionEntries}
                                    onValueChange={handleNodeExtensionValueChange}
                                    onRemove={handleNodeExtensionRemove}
                                    onAdd={handleNodeExtensionAdd}
                                    reservedKeys={CORE_NODE_ATTR_KEYS}
                                />
                            </div>
                        )}
                    </InspectorScaffold>
                </div>
            )}

            {activeTab === 'edge' && (
                <div className="flex-1 overflow-y-auto px-5 pb-5 pt-3">
                    <InspectorScaffold
                        scopeLabel="Edge"
                        title="Properties"
                        description="Use the same inspect-edit flow as graph and node inspectors."
                        entityLabel="Edge"
                        entityValue={selectedEdge ? `${selectedEdge.source} -> ${selectedEdge.target}` : undefined}
                    >
                        {!selectedEdge ? (
                            <InspectorEmptyState message="Select an edge on the canvas to inspect and edit its properties." />
                        ) : (
                            <div data-testid="edge-structured-form" className="space-y-4">
                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Label</label>
                                    <input
                                        value={(selectedEdge.data?.label as string) || ''}
                                        onChange={(e) => handleEdgePropertyChange('label', e.target.value)}
                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="e.g. Approve"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Condition</label>
                                    <input
                                        value={(selectedEdge.data?.condition as string) || ''}
                                        onChange={(e) => handleEdgePropertyChange('condition', e.target.value)}
                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder='e.g. outcome = "success"'
                                    />
                                    <div data-testid="edge-condition-syntax-hints" className="space-y-1 rounded-md border border-border/80 bg-muted/20 px-3 py-2 text-[11px] text-muted-foreground">
                                        <p>Use && to join clauses.</p>
                                        <p>{'Supported keys: outcome, preferred_label, context.<path>'}</p>
                                        <p>Operators: = or !=</p>
                                    </div>
                                    {renderFieldDiagnostics('edge', 'condition', edgeFieldDiagnostics, 'edge-field-diagnostics-condition')}
                                    <div
                                        data-testid="edge-condition-preview-feedback"
                                        className={`rounded-md border px-3 py-2 text-[11px] ${
                                            conditionPreviewHasError
                                                ? 'border-destructive/40 bg-destructive/10 text-destructive'
                                                : conditionPreviewHasWarning
                                                    ? 'border-amber-500/40 bg-amber-500/10 text-amber-800'
                                                    : 'border-emerald-500/40 bg-emerald-500/10 text-emerald-700'
                                        }`}
                                    >
                                        {selectedEdgeConditionDiagnostics.length > 0 ? (
                                            <ul className="space-y-1">
                                                {selectedEdgeConditionDiagnostics.map((diag, index) => (
                                                    <li key={`${diag.rule_id}-${diag.message}-${index}`}>{diag.message}</li>
                                                ))}
                                            </ul>
                                        ) : (
                                            <p>Condition syntax looks valid in preview.</p>
                                        )}
                                    </div>
                                </div>
                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Weight</label>
                                    <input
                                        value={(selectedEdge.data?.weight as number | string | undefined) ?? ''}
                                        onChange={(e) => handleEdgePropertyChange('weight', e.target.value)}
                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="0"
                                    />
                                </div>
                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Fidelity</label>
                                    <input
                                        value={(selectedEdge.data?.fidelity as string) || ''}
                                        onChange={(e) => handleEdgePropertyChange('fidelity', e.target.value)}
                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="full | truncate | compact | summary:low"
                                    />
                                    {renderFieldDiagnostics('edge', 'fidelity', edgeFieldDiagnostics, 'edge-field-diagnostics-fidelity')}
                                </div>
                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Thread ID</label>
                                    <input
                                        value={(selectedEdge.data?.thread_id as string) || ''}
                                        onChange={(e) => handleEdgePropertyChange('thread_id', e.target.value)}
                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    />
                                </div>
                                <div className="flex items-center gap-2">
                                    <input
                                        id="edge-loop-restart"
                                        type="checkbox"
                                        checked={Boolean(selectedEdge.data?.loop_restart)}
                                        onChange={(e) => handleEdgePropertyChange('loop_restart', e.target.checked)}
                                        className="h-4 w-4 rounded border border-input"
                                    />
                                    <label htmlFor="edge-loop-restart" className="text-sm font-medium">
                                        Loop Restart
                                    </label>
                                </div>
                                <AdvancedKeyValueEditor
                                    testIdPrefix="edge"
                                    entries={selectedEdgeExtensionEntries}
                                    onValueChange={handleEdgeExtensionValueChange}
                                    onRemove={handleEdgeExtensionRemove}
                                    onAdd={handleEdgeExtensionAdd}
                                    reservedKeys={CORE_EDGE_ATTR_KEYS}
                                />
                            </div>
                        )}
                    </InspectorScaffold>
                </div>
            )}
        </nav>
    )
}
