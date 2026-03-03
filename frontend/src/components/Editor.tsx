import { useCallback, useEffect, useRef, useState } from 'react';
import {
    ReactFlow,
    MiniMap,
    Controls,
    Background,
    useNodesState,
    useEdgesState,
    addEdge,
    applyNodeChanges,
    applyEdgeChanges,
} from '@xyflow/react';
import type { Connection, Edge, EdgeChange, Node, NodeChange, OnSelectionChangeParams } from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import ELK from 'elkjs/lib/elk.bundled.js';

import { useStore, type DiagnosticEntry } from '@/store';
import { TaskNode } from './TaskNode';
import { ValidationEdge } from './ValidationEdge';
import { ValidationPanel } from './ValidationPanel';
import { ExecutionControls } from './ExecutionControls';
import { generateDot } from '@/lib/dotUtils';
import { saveFlowContent } from '@/lib/flowPersistence';

const nodeTypes = {
    customTask: TaskNode,
};
const edgeTypes = {
    validation: ValidationEdge,
};
const EDGE_TYPE: Edge['type'] = 'validation';
const EDGE_CLASS = 'flow-edge';
const EDGE_INTERACTION_WIDTH = 16;
const LIVE_PREVIEW_DEBOUNCE_MS = 300;
const elk = new ELK();

const DEFAULT_NODE_WIDTH = 220;
const DEFAULT_NODE_HEIGHT = 110;
const ELK_OPTIONS = {
    'elk.algorithm': 'layered',
    'elk.direction': 'DOWN',
    'elk.layered.spacing.nodeNodeBetweenLayers': '30',
    'elk.spacing.nodeNode': '20',
    'elk.layered.cycleBreaking.strategy': 'DEPTH_FIRST',
};

interface PreviewNode {
    id: string
    label?: string
    shape?: string
    prompt?: string
    tool_command?: string
    'tool_hooks.pre'?: string
    'tool_hooks.post'?: string
    join_policy?: string
    error_policy?: string
    max_parallel?: number | string
    type?: string
    max_retries?: number | string
    goal_gate?: boolean | string
    retry_target?: string
    fallback_retry_target?: string
    fidelity?: string
    thread_id?: string
    class?: string
    timeout?: string
    llm_model?: string
    llm_provider?: string
    reasoning_effort?: string
    auto_status?: boolean | string
    allow_partial?: boolean | string
    'manager.poll_interval'?: string
    'manager.max_cycles'?: number | string
    'manager.stop_condition'?: string
    'manager.actions'?: string
    'human.default_choice'?: string
}

interface PreviewEdge {
    from: string
    to: string
    label?: string
    condition?: string
    weight?: number | string
    fidelity?: string
    thread_id?: string
    loop_restart?: boolean | string
}

interface PreviewResponse {
    status?: string
    graph?: {
        nodes: PreviewNode[]
        edges: PreviewEdge[]
        graph_attrs?: {
            goal?: string
            label?: string
            model_stylesheet?: string
            default_max_retry?: number | string
            retry_target?: string
            fallback_retry_target?: string
            default_fidelity?: string
            'stack.child_dotfile'?: string
            'stack.child_workdir'?: string
            'tool_hooks.pre'?: string
            'tool_hooks.post'?: string
            ui_default_llm_model?: string
            ui_default_llm_provider?: string
            ui_default_reasoning_effort?: string
        }
    }
    diagnostics?: DiagnosticEntry[]
    errors?: DiagnosticEntry[]
}

type EditorMode = 'structured' | 'raw'
type SaveFlowOptions = {
    expectSemanticEquivalence?: boolean
}

function normalizeLegacyDot(content: string): string {
    return content.replace(/\blabel=label=/g, 'label=');
}

async function layoutWithElk(nodes: Node[], edges: Edge[]): Promise<Node[]> {
    const graph = {
        id: 'root',
        layoutOptions: ELK_OPTIONS,
        children: nodes.map((node) => ({
            id: node.id,
            width: node.width ?? DEFAULT_NODE_WIDTH,
            height: node.height ?? DEFAULT_NODE_HEIGHT,
        })),
        edges: edges.map((edge) => ({
            id: edge.id,
            sources: [edge.source],
            targets: [edge.target],
        })),
    };

    const layout = await elk.layout(graph);
    const layoutMap = new Map((layout.children ?? []).map((child) => [child.id, child]));

    return nodes.map((node) => {
        const layoutNode = layoutMap.get(node.id);
        if (!layoutNode) return node;
        return {
            ...node,
            position: {
                x: layoutNode.x ?? node.position.x,
                y: layoutNode.y ?? node.position.y,
            },
        };
    });
}

export function Editor() {
    const { activeFlow, viewMode, selectedNodeId, selectedEdgeId, setSelectedNodeId, setSelectedEdgeId } = useStore();
    const activeProjectPath = useStore((state) => state.activeProjectPath);
    const nodeStatuses = useStore((state) => state.nodeStatuses);
    const graphAttrs = useStore((state) => state.graphAttrs);
    const uiDefaults = useStore((state) => state.uiDefaults);
    const saveErrorMessage = useStore((state) => state.saveErrorMessage);
    const setGraphAttrs = useStore((state) => state.setGraphAttrs);
    const setDiagnostics = useStore((state) => state.setDiagnostics);
    const clearDiagnostics = useStore((state) => state.clearDiagnostics);
    const suppressPreview = useStore((state) => state.suppressPreview);
    const [nodes, setNodes] = useNodesState<Node>([]);
    const [edges, setEdges] = useEdgesState<Edge>([]);
    const hydratedRef = useRef(false);
    const previewTimer = useRef<number | null>(null);
    const saveTimer = useRef<number | null>(null);
    const pendingSaveRef = useRef<{ nodes: Node[]; edges: Edge[]; options?: SaveFlowOptions } | null>(null);
    const rawDotEntryDraftRef = useRef<string>('');
    const [isDragging, setIsDragging] = useState(false);
    const [editorMode, setEditorMode] = useState<EditorMode>('structured');
    const [rawDotDraft, setRawDotDraft] = useState('');
    const [rawHandoffError, setRawHandoffError] = useState<string | null>(null);

    const enforceSingleSelectedNode = useCallback((nextNodes: Node[], selectedNodeId: string) => {
        setEdges((currentEdges) =>
            currentEdges.map((edge) => (edge.selected ? { ...edge, selected: false } : edge))
        );
        setSelectedNodeId(selectedNodeId);
        setSelectedEdgeId(null);
        return nextNodes.map((node) => {
            const shouldSelect = node.id === selectedNodeId;
            return node.selected === shouldSelect ? node : { ...node, selected: shouldSelect };
        });
    }, [setEdges, setSelectedEdgeId, setSelectedNodeId]);

    const enforceSingleSelectedEdge = useCallback((nextEdges: Edge[], selectedEdgeId: string) => {
        setNodes((currentNodes) =>
            currentNodes.map((node) => (node.selected ? { ...node, selected: false } : node))
        );
        setSelectedEdgeId(selectedEdgeId);
        setSelectedNodeId(null);
        return nextEdges.map((edge) => {
            const shouldSelect = edge.id === selectedEdgeId;
            return edge.selected === shouldSelect ? edge : { ...edge, selected: shouldSelect };
        });
    }, [setNodes, setSelectedEdgeId, setSelectedNodeId]);

    const saveFlow = useCallback((nextNodes: Node[], nextEdges: Edge[], options?: SaveFlowOptions) => {
        if (!activeProjectPath || !activeFlow) return;
        const dot = generateDot(activeFlow, nextNodes, nextEdges, graphAttrs);
        if (options) {
            void saveFlowContent(activeFlow, dot, options);
            return;
        }
        void saveFlowContent(activeFlow, dot);
    }, [activeProjectPath, activeFlow, graphAttrs]);

    const scheduleSave = useCallback((nextNodes: Node[], nextEdges: Edge[], options?: SaveFlowOptions) => {
        if (!activeProjectPath || !activeFlow) return;
        pendingSaveRef.current = { nodes: nextNodes, edges: nextEdges, options };
        if (saveTimer.current) {
            window.clearTimeout(saveTimer.current);
        }
        saveTimer.current = window.setTimeout(() => {
            pendingSaveRef.current = null;
            saveFlow(nextNodes, nextEdges, options);
        }, 250);
    }, [activeProjectPath, activeFlow, saveFlow]);

    const flushPendingSave = useCallback(() => {
        if (!activeProjectPath || !activeFlow || !pendingSaveRef.current) return;
        if (saveTimer.current) {
            window.clearTimeout(saveTimer.current);
            saveTimer.current = null;
        }
        const pending = pendingSaveRef.current;
        pendingSaveRef.current = null;
        saveFlow(pending.nodes, pending.edges, pending.options);
    }, [activeProjectPath, activeFlow, saveFlow]);

    const hydrateFromPreview = useCallback(async (preview: PreviewResponse) => {
        if (!preview.graph) return false;

        if (preview.graph.graph_attrs) {
            const nextGraphAttrs = { ...preview.graph.graph_attrs }
            const shouldSeed = (value?: string | null) =>
                value === undefined || value === null || value === ''
            if (shouldSeed(nextGraphAttrs.ui_default_llm_model) && uiDefaults.llm_model) {
                nextGraphAttrs.ui_default_llm_model = uiDefaults.llm_model
            }
            if (shouldSeed(nextGraphAttrs.ui_default_llm_provider) && uiDefaults.llm_provider) {
                nextGraphAttrs.ui_default_llm_provider = uiDefaults.llm_provider
            }
            if (shouldSeed(nextGraphAttrs.ui_default_reasoning_effort) && uiDefaults.reasoning_effort) {
                nextGraphAttrs.ui_default_reasoning_effort = uiDefaults.reasoning_effort
            }
            setGraphAttrs(nextGraphAttrs)
        }

        const rfNodes: Node[] = preview.graph.nodes.map((n, i: number) => ({
            id: n.id,
            type: 'customTask',
            position: { x: 250, y: i * 150 },
            data: {
                label: n.label,
                shape: n.shape ?? 'box',
                prompt: n.prompt ?? '',
                tool_command: n.tool_command ?? '',
                'tool_hooks.pre': n['tool_hooks.pre'] ?? '',
                'tool_hooks.post': n['tool_hooks.post'] ?? '',
                join_policy: n.join_policy ?? 'wait_all',
                error_policy: n.error_policy ?? 'continue',
                max_parallel: n.max_parallel ?? 4,
                type: n.type ?? '',
                max_retries: n.max_retries ?? '',
                goal_gate: n.goal_gate ?? false,
                retry_target: n.retry_target ?? '',
                fallback_retry_target: n.fallback_retry_target ?? '',
                fidelity: n.fidelity ?? '',
                thread_id: n.thread_id ?? '',
                class: n.class ?? '',
                timeout: n.timeout ?? '',
                llm_model: n.llm_model ?? '',
                llm_provider: n.llm_provider ?? '',
                reasoning_effort: n.reasoning_effort ?? '',
                auto_status: n.auto_status ?? false,
                allow_partial: n.allow_partial ?? false,
                'manager.poll_interval': n['manager.poll_interval'] ?? '',
                'manager.max_cycles': n['manager.max_cycles'] ?? '',
                'manager.stop_condition': n['manager.stop_condition'] ?? '',
                'manager.actions': n['manager.actions'] ?? '',
                'human.default_choice': n['human.default_choice'] ?? '',
                status: 'idle'
            },
        }));

        const rfEdges: Edge[] = preview.graph.edges.map((e, i: number) => ({
            id: `e-${e.from}-${e.to}-${i}`,
            source: e.from,
            target: e.to,
            type: EDGE_TYPE,
            className: EDGE_CLASS,
            interactionWidth: EDGE_INTERACTION_WIDTH,
            label: e.label,
            data: {
                label: e.label ?? '',
                condition: e.condition ?? '',
                weight: e.weight ?? '',
                fidelity: e.fidelity ?? '',
                thread_id: e.thread_id ?? '',
                loop_restart: e.loop_restart ?? false,
            },
        }));

        try {
            const layoutNodes = await layoutWithElk(rfNodes, rfEdges);
            setNodes(layoutNodes);
        } catch (error) {
            console.error('ELK layout failed, using fallback positions.', error);
            setNodes(rfNodes);
        }
        setEdges(rfEdges);
        hydratedRef.current = true;
        return true;
    }, [
        setEdges,
        setGraphAttrs,
        setNodes,
        uiDefaults.llm_model,
        uiDefaults.llm_provider,
        uiDefaults.reasoning_effort,
    ]);

    const requestPreview = useCallback(async (dot: string): Promise<PreviewResponse> => {
        const response = await fetch('/preview', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ flow_content: dot }),
        });
        const preview = (await response.json()) as PreviewResponse;
        if (preview.diagnostics) {
            setDiagnostics(preview.diagnostics);
        } else {
            clearDiagnostics();
        }
        return preview;
    }, [setDiagnostics, clearDiagnostics]);

    // Auto-load and sync with Backend Preview
    useEffect(() => {
        hydratedRef.current = false;
        if (!activeFlow) {
            setNodes([]);
            setEdges([]);
            clearDiagnostics();
            setRawDotDraft('');
            setRawHandoffError(null);
            setEditorMode('structured');
            rawDotEntryDraftRef.current = '';
            return;
        }
        clearDiagnostics();
        setRawHandoffError(null);
        setEditorMode('structured');
        rawDotEntryDraftRef.current = '';

        fetch(`/api/flows/${activeFlow}`)
            .then((res) => res.json())
            .then((data) => {
                const normalizedContent = normalizeLegacyDot(data.content);
                setRawDotDraft(normalizedContent);
                if (activeProjectPath && normalizedContent !== data.content) {
                    void saveFlowContent(activeFlow, normalizedContent, { expectSemanticEquivalence: true });
                }
                return requestPreview(normalizedContent);
            })
            .then((preview) => hydrateFromPreview(preview))
            .catch(console.error);
    }, [
        activeFlow,
        activeProjectPath,
        clearDiagnostics,
        hydrateFromPreview,
        requestPreview,
        setEdges,
        setNodes,
    ]);

    useEffect(() => {
        if (
            !activeFlow
            || !hydratedRef.current
            || viewMode === 'execution'
            || suppressPreview
            || isDragging
            || editorMode === 'raw'
        ) return;
        const dot = generateDot(activeFlow, nodes, edges, graphAttrs);
        if (previewTimer.current) {
            window.clearTimeout(previewTimer.current);
        }
        previewTimer.current = window.setTimeout(() => {
            void requestPreview(dot).catch(console.error);
        }, LIVE_PREVIEW_DEBOUNCE_MS);

        return () => {
            if (previewTimer.current) {
                window.clearTimeout(previewTimer.current);
            }
        };
    }, [activeFlow, nodes, edges, graphAttrs, viewMode, requestPreview, suppressPreview, isDragging, editorMode]);

    useEffect(() => {
        const handleBeforeUnload = () => {
            flushPendingSave();
        };
        window.addEventListener('beforeunload', handleBeforeUnload);
        return () => {
            window.removeEventListener('beforeunload', handleBeforeUnload);
            flushPendingSave();
        };
    }, [flushPendingSave]);

    // Handle new connections via UI
    const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {
        setNodes((currentNodes) => {
            const updatedNodes = applyNodeChanges(changes, currentNodes);
            const latestSelectedNodeChange = [...changes].reverse().find(
                (change): change is NodeChange<Node> & { type: 'select'; id: string; selected: boolean } =>
                    change.type === 'select' && change.selected === true
            );
            const nextNodes = latestSelectedNodeChange
                ? enforceSingleSelectedNode(updatedNodes, latestSelectedNodeChange.id)
                : updatedNodes;
            const draggingNow = changes.some(
                (change) => change.type === 'position' && (change as { dragging?: boolean }).dragging
            );
            const draggingStopped = changes.some(
                (change) => change.type === 'position' && (change as { dragging?: boolean }).dragging === false
            );
            if (draggingNow) {
                setIsDragging(true);
            } else if (draggingStopped) {
                setIsDragging(false);
            }

            const shouldSave = changes.some((change) => {
                if (change.type === 'select') return false;
                if (change.type === 'position') {
                    return !(change as { dragging?: boolean }).dragging;
                }
                if (change.type === 'dimensions') {
                    return !(change as { resizing?: boolean }).resizing;
                }
                return true;
            });

            if (shouldSave) {
                const nonSelectChanges = changes.filter((change) => change.type !== 'select');
                const shouldExpectSemanticEquivalence = nonSelectChanges.length > 0
                    && nonSelectChanges.every(
                        (change) => change.type === 'position' || change.type === 'dimensions'
                    );
                if (shouldExpectSemanticEquivalence) {
                    scheduleSave(nextNodes, edges, { expectSemanticEquivalence: true });
                } else {
                    scheduleSave(nextNodes, edges);
                }
            }
            return nextNodes;
        });
    }, [setNodes, scheduleSave, edges, enforceSingleSelectedNode]);

    const onEdgesChange = useCallback((changes: EdgeChange<Edge>[]) => {
        setEdges((currentEdges) => {
            const updatedEdges = applyEdgeChanges(changes, currentEdges);
            const latestSelectedEdgeChange = [...changes].reverse().find(
                (change): change is EdgeChange<Edge> & { type: 'select'; id: string; selected: boolean } =>
                    change.type === 'select' && change.selected === true
            );
            const nextEdges = latestSelectedEdgeChange
                ? enforceSingleSelectedEdge(updatedEdges, latestSelectedEdgeChange.id)
                : updatedEdges;
            const shouldSave = changes.some((change) => change.type !== 'select');
            if (shouldSave) {
                scheduleSave(nodes, nextEdges);
            }
            return nextEdges;
        });
    }, [setEdges, scheduleSave, nodes, enforceSingleSelectedEdge]);

    const onConnect = useCallback(
        (params: Connection | Edge) => {
            setEdges((currentEdges) => {
                const newEdges = addEdge(
                    { ...params, type: EDGE_TYPE, interactionWidth: EDGE_INTERACTION_WIDTH },
                    currentEdges
                );
                scheduleSave(nodes, newEdges);
                return newEdges;
            });
        },
        [setEdges, scheduleSave, nodes],
    );

    const onAddNode = useCallback(() => {
        if (!activeProjectPath || !activeFlow) return;
        const defaultModel = graphAttrs.ui_default_llm_model || uiDefaults.llm_model || '';
        const defaultProvider = graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || '';
        const defaultReasoning = graphAttrs.ui_default_reasoning_effort || uiDefaults.reasoning_effort || '';
        const newNodeId = `node_${Math.floor(Math.random() * 10000)}`;
        const newNode: Node = {
            id: newNodeId,
            type: 'customTask',
            position: { x: Math.random() * 200 + 100, y: Math.random() * 200 + 100 },
            data: {
                label: 'New Node',
                shape: 'box',
                status: 'idle',
                llm_model: defaultModel,
                llm_provider: defaultProvider,
                reasoning_effort: defaultReasoning,
            }
        };

        setNodes(nds => {
            const newNodes = [...nds, newNode];
            scheduleSave(newNodes, edges);
            return newNodes;
        });
    }, [activeProjectPath, activeFlow, edges, graphAttrs, uiDefaults, setNodes, scheduleSave]);

    const enterRawDotMode = useCallback(() => {
        if (!activeProjectPath || !activeFlow) return;
        if (editorMode === 'raw') return;
        flushPendingSave();
        const dot = generateDot(activeFlow, nodes, edges, graphAttrs);
        rawDotEntryDraftRef.current = dot;
        setRawDotDraft(dot);
        setRawHandoffError(null);
        setEditorMode('raw');
    }, [activeProjectPath, activeFlow, editorMode, edges, flushPendingSave, graphAttrs, nodes]);

    const returnToStructuredMode = useCallback(async () => {
        if (!activeProjectPath || !activeFlow) return;
        const expectSemanticEquivalence = rawDotEntryDraftRef.current === rawDotDraft;
        const saveOptions = expectSemanticEquivalence ? { expectSemanticEquivalence: true } : undefined;
        const saved = await saveFlowContent(activeFlow, rawDotDraft, saveOptions);
        if (!saved) {
            setRawHandoffError(`Safe handoff requires valid DOT. ${saveErrorMessage || 'Fix parse or validation errors before switching modes.'}`);
            return;
        }

        try {
            const preview = await requestPreview(rawDotDraft);
            const hydrated = await hydrateFromPreview(preview);
            if (!hydrated) {
                setRawHandoffError('Safe handoff requires valid DOT. Preview response did not include a graph.');
                return;
            }
            setRawHandoffError(null);
            rawDotEntryDraftRef.current = '';
            setEditorMode('structured');
        } catch {
            setRawHandoffError('Safe handoff requires valid DOT. Failed to parse DOT preview for structured mode.');
        }
    }, [activeProjectPath, activeFlow, hydrateFromPreview, rawDotDraft, requestPreview, saveErrorMessage]);

    const onSelectionChange = useCallback(({ nodes, edges }: OnSelectionChangeParams) => {
        const selectedNode = nodes.find(n => n.selected);
        const selectedEdge = edges.find(e => e.selected);
        setSelectedNodeId(selectedNode ? selectedNode.id : null);
        setSelectedEdgeId(selectedEdge ? selectedEdge.id : null);
    }, [setSelectedEdgeId, setSelectedNodeId]);

    useEffect(() => {
        setNodes((currentNodes) => currentNodes.map((node) => {
            const nextStatus = nodeStatuses[node.id] || 'idle';
            if (node.data?.status === nextStatus) {
                return node;
            }
            return { ...node, data: { ...node.data, status: nextStatus } };
        }));
    }, [nodeStatuses, setNodes]);

    useEffect(() => {
        setNodes((currentNodes) =>
            currentNodes.map((node) => {
                const shouldSelect = !selectedEdgeId && node.id === selectedNodeId;
                return node.selected === shouldSelect ? node : { ...node, selected: shouldSelect };
            })
        );
        setEdges((currentEdges) =>
            currentEdges.map((edge) => {
                const shouldSelect = edge.id === selectedEdgeId;
                return edge.selected === shouldSelect ? edge : { ...edge, selected: shouldSelect };
            })
        );
    }, [selectedNodeId, selectedEdgeId, setNodes, setEdges]);

    return (
        <div className="flow-surface w-full h-full relative">
            {editorMode === 'raw' ? (
                <div className="h-full w-full p-4">
                    <div className="h-full rounded-lg border border-border bg-background/80 p-3">
                        <textarea
                            data-testid="raw-dot-editor"
                            value={rawDotDraft}
                            onChange={(event) => {
                                setRawDotDraft(event.target.value);
                                setRawHandoffError(null);
                            }}
                            className="h-full w-full resize-none rounded-md border border-input bg-background px-3 py-2 font-mono text-xs leading-5 shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            spellCheck={false}
                        />
                    </div>
                    {rawHandoffError ? (
                        <p data-testid="raw-dot-handoff-error" className="mt-2 text-xs font-medium text-destructive">
                            {rawHandoffError}
                        </p>
                    ) : null}
                </div>
            ) : (
                <ReactFlow
                    className="flow-canvas"
                    style={{ background: 'transparent' }}
                    nodes={nodes}
                    edges={edges}
                    onNodesChange={onNodesChange}
                    onEdgesChange={onEdgesChange}
                    onConnect={onConnect}
                    onSelectionChange={onSelectionChange}
                    nodeTypes={nodeTypes}
                    edgeTypes={edgeTypes}
                    defaultEdgeOptions={{
                        type: EDGE_TYPE,
                        className: EDGE_CLASS,
                        interactionWidth: EDGE_INTERACTION_WIDTH,
                    }}
                    elevateEdgesOnSelect
                    fitView
                    colorMode="light"
                    minZoom={0.1}
                    maxZoom={1.5}
                >
                    <Controls />
                    <MiniMap
                        nodeColor="hsl(var(--muted))"
                        maskColor="hsl(var(--background)/0.5)"
                        className="flow-minimap"
                    />
                    <Background gap={20} size={1} color="hsl(var(--border))" />
                </ReactFlow>
            )}

            {viewMode === 'editor' && activeFlow && (
                <div className="absolute left-4 top-4 z-10 flex gap-2">
                    <div data-testid="editor-mode-toggle" className="flex rounded-md border border-border bg-background/90 p-1 shadow-sm">
                        <button
                            onClick={() => {
                                if (editorMode === 'raw') {
                                    void returnToStructuredMode();
                                    return;
                                }
                                setEditorMode('structured');
                            }}
                            className={`px-3 py-1.5 rounded text-sm font-medium transition-colors ${
                                editorMode === 'structured'
                                    ? 'bg-primary text-primary-foreground'
                                    : 'text-muted-foreground hover:text-foreground'
                            }`}
                        >
                            Structured
                        </button>
                        <button
                            onClick={enterRawDotMode}
                            disabled={editorMode === 'raw'}
                            className={`px-3 py-1.5 rounded text-sm font-medium transition-colors ${
                                editorMode === 'raw'
                                    ? 'bg-primary text-primary-foreground'
                                    : 'text-muted-foreground hover:text-foreground'
                            }`}
                        >
                            Raw DOT
                        </button>
                    </div>
                    {editorMode === 'structured' && (
                        <button
                            onClick={onAddNode}
                            className="bg-primary text-primary-foreground shadow-sm px-3 py-1.5 rounded-md text-sm font-medium hover:bg-primary/90 transition-colors"
                        >
                            Add Node
                        </button>
                    )}
                </div>
            )}

            {activeFlow && editorMode === 'structured' && <ValidationPanel />}
            <ExecutionControls />
        </div>
    );
}
