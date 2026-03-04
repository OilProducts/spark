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

import { useStore, type DiagnosticEntry, type GraphAttrs } from '@/store';
import { TaskNode } from './TaskNode';
import { ValidationEdge } from './ValidationEdge';
import { ValidationPanel } from './ValidationPanel';
import { ExecutionControls } from './ExecutionControls';
import { clearDotSerializationContext, generateDot, setDotSerializationContext } from '@/lib/dotUtils';
import { buildCanonicalFlowModelFromPreviewGraph } from '@/lib/canonicalFlowModel';
import {
    fetchFlowPayloadValidated,
    fetchPreviewValidated,
    type PreviewResponsePayload,
} from '@/lib/apiClient';
import {
    EXPECT_SEMANTIC_EQUIVALENCE_OPTIONS,
    saveFlowContent,
    saveFlowContentExpectingSemanticEquivalence,
} from '@/lib/flowPersistence';

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

type PreviewResponse = PreviewResponsePayload

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
    const rawHandoffInFlightRef = useRef(false);
    const [isDragging, setIsDragging] = useState(false);
    const [editorMode, setEditorMode] = useState<EditorMode>('structured');
    const [rawDotDraft, setRawDotDraft] = useState('');
    const [rawHandoffError, setRawHandoffError] = useState<string | null>(null);
    const [isRawHandoffInFlight, setIsRawHandoffInFlight] = useState(false);

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

    const hydrateFromPreview = useCallback(async (preview: PreviewResponse, sourceDot?: string) => {
        if (!preview.graph) return false;
        const canonicalModel = buildCanonicalFlowModelFromPreviewGraph(
            activeFlow ?? 'flow',
            preview.graph,
            sourceDot !== undefined ? { rawDot: sourceDot } : undefined,
        )
        setDotSerializationContext({
            defaults: canonicalModel.defaults,
            subgraphs: canonicalModel.subgraphs,
        })

        if (preview.graph.graph_attrs) {
            const nextGraphAttrs: GraphAttrs = { ...canonicalModel.graphAttrs }
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

        const rfNodes: Node[] = canonicalModel.nodes.map((n, i: number) => ({
            id: n.id,
            type: 'customTask',
            position: { x: 250, y: i * 150 },
            data: {
                ...n.attrs,
                label: typeof n.attrs.label === 'string' ? n.attrs.label : n.id,
                shape: typeof n.attrs.shape === 'string' ? n.attrs.shape : 'box',
                prompt: typeof n.attrs.prompt === 'string' ? n.attrs.prompt : '',
                tool_command: typeof n.attrs.tool_command === 'string' ? n.attrs.tool_command : '',
                'tool_hooks.pre': typeof n.attrs['tool_hooks.pre'] === 'string' ? n.attrs['tool_hooks.pre'] : '',
                'tool_hooks.post': typeof n.attrs['tool_hooks.post'] === 'string' ? n.attrs['tool_hooks.post'] : '',
                join_policy: typeof n.attrs.join_policy === 'string' ? n.attrs.join_policy : 'wait_all',
                error_policy: typeof n.attrs.error_policy === 'string' ? n.attrs.error_policy : 'continue',
                max_parallel: typeof n.attrs.max_parallel === 'number' || typeof n.attrs.max_parallel === 'string'
                    ? n.attrs.max_parallel
                    : 4,
                type: typeof n.attrs.type === 'string' ? n.attrs.type : '',
                max_retries: typeof n.attrs.max_retries === 'number' || typeof n.attrs.max_retries === 'string'
                    ? n.attrs.max_retries
                    : '',
                goal_gate: n.attrs.goal_gate === true || n.attrs.goal_gate === 'true',
                retry_target: typeof n.attrs.retry_target === 'string' ? n.attrs.retry_target : '',
                fallback_retry_target: typeof n.attrs.fallback_retry_target === 'string' ? n.attrs.fallback_retry_target : '',
                fidelity: typeof n.attrs.fidelity === 'string' ? n.attrs.fidelity : '',
                thread_id: typeof n.attrs.thread_id === 'string' ? n.attrs.thread_id : '',
                class: typeof n.attrs.class === 'string' ? n.attrs.class : '',
                timeout: typeof n.attrs.timeout === 'string' ? n.attrs.timeout : '',
                llm_model: typeof n.attrs.llm_model === 'string' ? n.attrs.llm_model : '',
                llm_provider: typeof n.attrs.llm_provider === 'string' ? n.attrs.llm_provider : '',
                reasoning_effort: typeof n.attrs.reasoning_effort === 'string' ? n.attrs.reasoning_effort : '',
                auto_status: n.attrs.auto_status === true || n.attrs.auto_status === 'true',
                allow_partial: n.attrs.allow_partial === true || n.attrs.allow_partial === 'true',
                'manager.poll_interval': typeof n.attrs['manager.poll_interval'] === 'string'
                    ? n.attrs['manager.poll_interval']
                    : '',
                'manager.max_cycles': typeof n.attrs['manager.max_cycles'] === 'number'
                    || typeof n.attrs['manager.max_cycles'] === 'string'
                    ? n.attrs['manager.max_cycles']
                    : '',
                'manager.stop_condition': typeof n.attrs['manager.stop_condition'] === 'string'
                    ? n.attrs['manager.stop_condition']
                    : '',
                'manager.actions': typeof n.attrs['manager.actions'] === 'string' ? n.attrs['manager.actions'] : '',
                'human.default_choice': typeof n.attrs['human.default_choice'] === 'string'
                    ? n.attrs['human.default_choice']
                    : '',
                status: 'idle'
            },
        }));

        const rfEdges: Edge[] = canonicalModel.edges.map((e, i: number) => ({
            id: `e-${e.source}-${e.target}-${i}`,
            source: e.source,
            target: e.target,
            type: EDGE_TYPE,
            className: EDGE_CLASS,
            interactionWidth: EDGE_INTERACTION_WIDTH,
            label: typeof e.attrs.label === 'string' ? e.attrs.label : undefined,
            data: {
                ...e.attrs,
                label: typeof e.attrs.label === 'string' ? e.attrs.label : '',
                condition: typeof e.attrs.condition === 'string' ? e.attrs.condition : '',
                weight: typeof e.attrs.weight === 'number' || typeof e.attrs.weight === 'string' ? e.attrs.weight : '',
                fidelity: typeof e.attrs.fidelity === 'string' ? e.attrs.fidelity : '',
                thread_id: typeof e.attrs.thread_id === 'string' ? e.attrs.thread_id : '',
                loop_restart: e.attrs.loop_restart === true || e.attrs.loop_restart === 'true',
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
        activeFlow,
        setEdges,
        setGraphAttrs,
        setNodes,
        uiDefaults.llm_model,
        uiDefaults.llm_provider,
        uiDefaults.reasoning_effort,
    ]);

    const requestPreview = useCallback(async (dot: string): Promise<PreviewResponse> => {
        const preview = await fetchPreviewValidated(dot)
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
            clearDotSerializationContext();
            setNodes([]);
            setEdges([]);
            clearDiagnostics();
            setRawDotDraft('');
            setRawHandoffError(null);
            rawHandoffInFlightRef.current = false;
            setIsRawHandoffInFlight(false);
            setEditorMode('structured');
            rawDotEntryDraftRef.current = '';
            return;
        }
        clearDotSerializationContext();
        clearDiagnostics();
        setRawHandoffError(null);
        rawHandoffInFlightRef.current = false;
        setIsRawHandoffInFlight(false);
        setEditorMode('structured');
        rawDotEntryDraftRef.current = '';

        fetchFlowPayloadValidated(activeFlow)
            .then((data) => {
                const normalizedContent = normalizeLegacyDot(data.content);
                setRawDotDraft(normalizedContent);
                if (activeProjectPath && normalizedContent !== data.content) {
                    void saveFlowContentExpectingSemanticEquivalence(activeFlow, normalizedContent);
                }
                return requestPreview(normalizedContent).then((preview) => ({
                    normalizedContent,
                    preview,
                }));
            })
            .then(({ normalizedContent, preview }) => hydrateFromPreview(preview, normalizedContent))
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
                    scheduleSave(nextNodes, edges, EXPECT_SEMANTIC_EQUIVALENCE_OPTIONS);
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
        if (rawHandoffInFlightRef.current) {
            return;
        }
        rawHandoffInFlightRef.current = true;
        setIsRawHandoffInFlight(true);
        try {
            const expectSemanticEquivalence = rawDotEntryDraftRef.current === rawDotDraft;
            const save = expectSemanticEquivalence ? saveFlowContentExpectingSemanticEquivalence : saveFlowContent;
            const saved = await save(activeFlow, rawDotDraft);
            if (!saved) {
                setRawHandoffError(`Safe handoff requires valid DOT. ${saveErrorMessage || 'Fix parse or validation errors before switching modes.'}`);
                return;
            }

            try {
                const preview = await requestPreview(rawDotDraft);
                if (preview.status === 'validation_error' || (preview.errors?.length ?? 0) > 0) {
                    setRawHandoffError(
                        'Raw DOT edit conflicts with structured mode assumptions. Resolve validation errors before switching modes.',
                    );
                    return;
                }
                const hydrated = await hydrateFromPreview(preview, rawDotDraft);
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
        } finally {
            rawHandoffInFlightRef.current = false;
            setIsRawHandoffInFlight(false);
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
                            disabled={editorMode === 'raw' && isRawHandoffInFlight}
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
