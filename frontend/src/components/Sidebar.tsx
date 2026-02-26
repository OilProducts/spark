import { useStore } from "@/store"
import { FilePlus, Trash2 } from "lucide-react"
import { useEffect, useRef, useState } from "react"
import { useReactFlow, useStore as useReactFlowStore, type Edge, type Node } from "@xyflow/react"
import { generateDot, sanitizeGraphId } from "@/lib/dotUtils"
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from "@/lib/llmSuggestions"
import { getHandlerType, getNodeFieldVisibility } from "@/lib/nodeVisibility"

export function Sidebar() {
    const { viewMode, activeFlow, setActiveFlow, selectedNodeId, selectedEdgeId } = useStore()
    const humanGate = useStore((state) => state.humanGate)
    const graphAttrs = useStore((state) => state.graphAttrs)
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setSuppressPreview = useStore((state) => state.setSuppressPreview)
    const [flows, setFlows] = useState<string[]>([])
    const [showAdvanced, setShowAdvanced] = useState(false)
    const { getNodes, setNodes, getEdges, setEdges } = useReactFlow()
    const nodes = useReactFlowStore((state) => state.nodes)
    const edges = useReactFlowStore((state) => state.edges)
    const saveTimer = useRef<number | null>(null)
    const promptNodeRef = useRef<string | null>(null)
    const promptRef = useRef<HTMLTextAreaElement | null>(null)

    const loadFlows = () => {
        fetch('/api/flows')
            .then(res => res.json())
            .then(data => setFlows(data))
            .catch(console.error)
    }

    useEffect(() => {
        loadFlows()
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

        await fetch('/api/flows', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name: fileName, content: initialContent })
        });

        await loadFlows();
        setActiveFlow(fileName);
    }

    const handleDeleteFlow = async (e: React.MouseEvent, fileName: string) => {
        e.stopPropagation();
        if (!window.confirm(`Are you sure you want to delete ${fileName}?`)) return;

        await fetch(`/api/flows/${fileName}`, {
            method: 'DELETE'
        });

        if (activeFlow === fileName) {
            setActiveFlow(null);
            setNodes([]);
            setEdges([]);
        }
        await loadFlows();
    };

    const scheduleSave = (nextNodes: Node[], nextEdges: Edge[]) => {
        if (!activeFlow) return

        if (saveTimer.current) {
            window.clearTimeout(saveTimer.current)
        }

        saveTimer.current = window.setTimeout(() => {
            const dot = generateDot(activeFlow, nextNodes, nextEdges, graphAttrs)
            fetch('/api/flows', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ name: activeFlow, content: dot })
            }).catch(console.error)
        }, 600)
    }

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
            scheduleSave(newNodes, getEdges());
        }
    }

    const handlePropertyChange = (key: string, value: string | boolean) => {
        if (!selectedNodeId) return;
        updateNodeProperty(selectedNodeId, key, value)
    }

    const selectedNode = nodes.find(n => n.id === selectedNodeId);
    const selectedEdge = edges.find(e => e.id === selectedEdgeId);
    const isTrue = (value: unknown) => value === true || value === 'true';
    const handlerType = getHandlerType(
        (selectedNode?.data?.shape as string) || '',
        (selectedNode?.data?.type as string) || ''
    )
    const visibility = getNodeFieldVisibility(handlerType)
    const autoTab = selectedEdgeId ? 'edge' : selectedNodeId ? 'edit' : 'flows';
    const activeTab = viewMode !== 'editor' ? 'flows' : autoTab;

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
            scheduleSave(getNodes(), newEdges);
        }
    };

    const commitPrompt = () => {
        const nodeId = promptNodeRef.current ?? selectedNodeId
        if (!nodeId) return
        const value = promptRef.current?.value ?? ''
        updateNodeProperty(nodeId, 'prompt', value)
    }

    return (
        <nav className="w-72 border-r bg-background flex flex-col shrink-0 overflow-hidden z-40">
            <div className="px-4 pb-2 pt-4">
                <div className="flex items-center gap-3 text-xs font-semibold uppercase tracking-[0.2em] text-foreground">
                    <span>{activeTab === 'flows' ? 'Flows' : activeTab === 'edit' ? 'Node' : 'Edge'}</span>
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
                    <div className="flex-1 overflow-y-auto px-3 space-y-1">
                        {flows.map(f => (
                            <div key={f} className="relative group">
                                <button
                                    onClick={() => setActiveFlow(f)}
                                    className={`w-full text-left px-3 py-2 pr-8 rounded-md text-sm transition-colors ${activeFlow === f
                                        ? 'bg-secondary text-secondary-foreground font-medium'
                                        : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                                        }`}
                                >
                                    <span className="flex items-center gap-2">
                                        {humanGate?.flowName === f && (
                                            <span
                                                className="h-2 w-2 rounded-full bg-amber-500"
                                                title="Needs human input"
                                            />
                                        )}
                                        {f}
                                    </span>
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
                </div>
            )}

            {activeTab === 'edit' && (
                <div className="flex-1 flex flex-col overflow-hidden">
                    <div className="px-5 py-2">
                        <h2 className="font-semibold text-sm tracking-tight">Configuration</h2>
                    </div>

                    {!selectedNodeId ? (
                        <div className="flex-1 flex flex-col items-center justify-center text-center px-6 text-muted-foreground p-4">
                            <p className="text-sm">Select a component on the canvas to inspect and edit its properties.</p>
                        </div>
                    ) : (
                        <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-5">
                            <div className="space-y-1.5">
                                <label className="text-sm font-medium">Node ID</label>
                                <input readOnly value={selectedNodeId} className="flex h-9 w-full rounded-md border border-input bg-muted/50 px-3 py-1 text-xs font-mono shadow-sm cursor-not-allowed" />
                            </div>
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
                                    <option value="Mdiamond">Start Node</option>
                                    <option value="Msquare">End Node</option>
                                </select>
                            </div>

                            {visibility.showPrompt && (
                                <div className="space-y-1.5 flex flex-col h-48">
                                    <label className="text-sm font-medium">Prompt Instruction</label>
                                    <textarea
                                        key={selectedNodeId ?? 'prompt'}
                                        ref={promptRef}
                                        defaultValue={(selectedNode?.data?.prompt as string) || ''}
                                        onFocus={() => {
                                            setSuppressPreview(true)
                                            promptNodeRef.current = selectedNodeId
                                        }}
                                        onBlur={() => {
                                            setSuppressPreview(false)
                                            commitPrompt()
                                        }}
                                        className="flex flex-1 w-full rounded-md border border-input bg-background px-3 py-2 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring resize-none"
                                        placeholder="Enter system prompt instructions..."
                                    />
                                </div>
                            )}
                            {visibility.showToolCommand && (
                                <div className="space-y-1.5">
                                    <label className="text-sm font-medium">Tool Command</label>
                                    <input
                                        value={(selectedNode?.data?.tool_command as string) || ''}
                                        onChange={(e) => handlePropertyChange('tool_command', e.target.value)}
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
                                    <div className="space-y-1.5">
                                        <label className="text-sm font-medium">Handler Type</label>
                                        <input
                                            value={(selectedNode?.data?.type as string) || ''}
                                            onChange={(e) => handlePropertyChange('type', e.target.value)}
                                            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            placeholder="optional override"
                                        />
                                    </div>
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
                                            <div className="space-y-1.5">
                                                <label className="text-sm font-medium">Retry Target</label>
                                                <input
                                                    value={(selectedNode?.data?.retry_target as string) || ''}
                                                    onChange={(e) => handlePropertyChange('retry_target', e.target.value)}
                                                    className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                            </div>
                                            <div className="space-y-1.5">
                                                <label className="text-sm font-medium">Fallback Retry Target</label>
                                                <input
                                                    value={(selectedNode?.data?.fallback_retry_target as string) || ''}
                                                    onChange={(e) => handlePropertyChange('fallback_retry_target', e.target.value)}
                                                    className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                            </div>
                                            <div className="grid grid-cols-2 gap-3">
                                                <div className="space-y-1.5">
                                                    <label className="text-sm font-medium">Fidelity</label>
                                                    <input
                                                        value={(selectedNode?.data?.fidelity as string) || ''}
                                                        onChange={(e) => handlePropertyChange('fidelity', e.target.value)}
                                                        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        placeholder="full"
                                                    />
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
                        </div>
                    )}
                </div>
            )}

            {activeTab === 'edge' && (
                <div className="flex-1 flex flex-col overflow-hidden">
                    <div className="px-5 py-2">
                        <h2 className="font-semibold text-sm tracking-tight">Edge Properties</h2>
                    </div>

                    {!selectedEdge ? (
                        <div className="flex-1 flex flex-col items-center justify-center text-center px-6 text-muted-foreground p-4">
                            <p className="text-sm">Select an edge on the canvas to inspect and edit its properties.</p>
                        </div>
                    ) : (
                        <div className="flex-1 overflow-y-auto px-5 pb-5 space-y-5">
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
                        </div>
                    )}
                </div>
            )}
        </nav>
    )
}
