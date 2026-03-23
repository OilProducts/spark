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

import { useStore, type GraphAttrs } from '@/store';
import { ValidationPanel } from './ValidationPanel';
import { clearDotSerializationContext, generateDot, setDotSerializationContext } from '@/lib/dotUtils';
import { recordFlowLoadDebug, summarizeDiagnosticsForFlowLoadDebug } from '@/lib/flowLoadDebug';
import {
    fetchFlowPayloadValidated,
    fetchPreviewValidated,
    type PreviewResponsePayload,
} from '@/lib/attractorClient';
import {
    EXPECT_SEMANTIC_EQUIVALENCE_OPTIONS,
    primeFlowSaveBaseline,
    saveFlowContent,
    saveFlowContentExpectingSemanticEquivalence,
} from '@/lib/flowPersistence';
import { CANVAS_INTERACTION_BUDGET_MS } from '@/lib/performanceBudgets';
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler';
import {
    buildHydratedFlowGraph,
    edgeTypes,
    EDGE_CLASS,
    EDGE_INTERACTION_WIDTH,
    EDGE_TYPE,
    layoutWithElk,
    nodeTypes,
    normalizeLegacyDot,
    nowMs,
} from './flowCanvasShared';

const DEFAULT_PREVIEW_DEBOUNCE_MS = 300;
const MEDIUM_GRAPH_PREVIEW_DEBOUNCE_MS = 600;
const MEDIUM_GRAPH_NODE_THRESHOLD = 25;

type PreviewResponse = PreviewResponsePayload

type EditorMode = 'structured' | 'raw'
type FlowGraphSnapshot = {
    nodes: Node[]
    edges: Edge[]
}

type PreviewDebugContext = {
    source: 'flow-load-source-dot' | 'structured-sync-preview' | 'raw-dot-handoff'
    loadId: number | null
    nodeCount?: number
    edgeCount?: number
    graphAttrCount?: number
    debounceMs?: number
}

export function Editor() {
    const selectedNodeId = useStore((state) => state.selectedNodeId);
    const selectedEdgeId = useStore((state) => state.selectedEdgeId);
    const setSelectedNodeId = useStore((state) => state.setSelectedNodeId);
    const setSelectedEdgeId = useStore((state) => state.setSelectedEdgeId);
    const activeFlow = useStore((state) => state.activeFlow);
    const graphAttrs = useStore((state) => state.graphAttrs);
    const uiDefaults = useStore((state) => state.uiDefaults);
    const replaceGraphAttrs = useStore((state) => state.replaceGraphAttrs);
    const setDiagnostics = useStore((state) => state.setDiagnostics);
    const clearDiagnostics = useStore((state) => state.clearDiagnostics);
    const suppressPreview = useStore((state) => state.suppressPreview);
    const [nodes, setNodes] = useNodesState<Node>([]);
    const [edges, setEdges] = useEdgesState<Edge>([]);
    const hydratedRef = useRef(false);
    const previewTimer = useRef<number | null>(null);
    const activeFlowLoadIdRef = useRef(0);
    const rawDotEntryDraftRef = useRef<string>('');
    const rawHandoffInFlightRef = useRef(false);
    const [isDragging, setIsDragging] = useState(false);
    const [editorMode, setEditorMode] = useState<EditorMode>('structured');
    const [rawDotDraft, setRawDotDraft] = useState('');
    const [rawHandoffError, setRawHandoffError] = useState<string | null>(null);
    const [isRawHandoffInFlight, setIsRawHandoffInFlight] = useState(false);
    const [lastLayoutMs, setLastLayoutMs] = useState(0);
    const [lastPreviewMs, setLastPreviewMs] = useState(0);

    const nodeCount = nodes.length;
    const isMediumGraph = nodeCount >= MEDIUM_GRAPH_NODE_THRESHOLD;
    const previewDebounceMs = isMediumGraph ? MEDIUM_GRAPH_PREVIEW_DEBOUNCE_MS : DEFAULT_PREVIEW_DEBOUNCE_MS;
    const onlyRenderVisibleElements = isMediumGraph;
    const performanceProfile = isMediumGraph ? 'medium' : 'default';
    const activeOptimizations = [
        ...(onlyRenderVisibleElements ? ['visible-only'] : []),
        ...(previewDebounceMs > DEFAULT_PREVIEW_DEBOUNCE_MS ? ['debounced-preview'] : []),
    ];
    const optimizationLabel = activeOptimizations.length ? activeOptimizations.join(', ') : 'none';
    const flowName = activeFlow;

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

    const { flushPendingSave, scheduleSave } = useFlowSaveScheduler<FlowGraphSnapshot>({
        flowName,
        debounceMs: 250,
        buildContent: (snapshot, currentFlowName) => generateDot(
            currentFlowName,
            snapshot?.nodes || [],
            snapshot?.edges || [],
            graphAttrs,
        ),
    })

    const hydrateFromPreview = useCallback(async (
        preview: PreviewResponse,
        sourceDot?: string,
        debugContext?: { loadId: number | null; source: PreviewDebugContext['source'] },
    ) => {
        if (!preview.graph) {
            recordFlowLoadDebug('hydrate:skipped', flowName, {
                loadId: debugContext?.loadId ?? null,
                source: debugContext?.source ?? 'flow-load-source-dot',
                reason: 'preview graph missing',
            });
            return false;
        }
        const hydratedGraph = buildHydratedFlowGraph(
            flowName ?? 'flow',
            preview,
            uiDefaults,
            sourceDot,
        )
        if (!hydratedGraph) {
            return false
        }
        recordFlowLoadDebug('hydrate:start', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load-source-dot',
            sourceDotLength: sourceDot?.length ?? null,
            nodeCount: hydratedGraph.nodes.length,
            edgeCount: hydratedGraph.edges.length,
            graphAttrCount: Object.keys(hydratedGraph.graphAttrs).length,
        })
        setDotSerializationContext({
            defaults: hydratedGraph.defaults,
            subgraphs: hydratedGraph.subgraphs,
        })
        replaceGraphAttrs(hydratedGraph.graphAttrs)

        const layoutStart = nowMs();
        let layoutDurationMs = 0;
        let serializedNodes = hydratedGraph.nodes;
        try {
            const layoutNodes = await layoutWithElk(hydratedGraph.nodes, hydratedGraph.edges);
            setNodes(layoutNodes);
            serializedNodes = layoutNodes;
        } catch (error) {
            console.error('ELK layout failed, using fallback positions.', error);
            setNodes(hydratedGraph.nodes);
            serializedNodes = hydratedGraph.nodes;
        } finally {
            layoutDurationMs = Math.max(0, nowMs() - layoutStart);
            setLastLayoutMs(layoutDurationMs);
        }
        setEdges(hydratedGraph.edges);
        primeFlowSaveBaseline(
            flowName ?? 'flow',
            generateDot(flowName ?? 'flow', serializedNodes, hydratedGraph.edges, hydratedGraph.graphAttrs),
        )
        hydratedRef.current = true;
        recordFlowLoadDebug('hydrate:complete', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load-source-dot',
            nodeCount: hydratedGraph.nodes.length,
            edgeCount: hydratedGraph.edges.length,
            layoutMs: layoutDurationMs,
        })
        return true;
    }, [
        flowName,
        setEdges,
        replaceGraphAttrs,
        setNodes,
        uiDefaults.llm_model,
        uiDefaults.llm_provider,
        uiDefaults.reasoning_effort,
    ]);

    const requestPreview = useCallback(async (
        dot: string,
        debugContext?: PreviewDebugContext,
    ): Promise<PreviewResponse> => {
        recordFlowLoadDebug('preview:request', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'structured-sync-preview',
            dotLength: dot.length,
            nodeCount: debugContext?.nodeCount ?? null,
            edgeCount: debugContext?.edgeCount ?? null,
            graphAttrCount: debugContext?.graphAttrCount ?? null,
            debounceMs: debugContext?.debounceMs ?? null,
        });
        const previewStart = nowMs();
        const preview = await fetchPreviewValidated(dot)
        const elapsed = Math.max(0, nowMs() - previewStart);
        setLastPreviewMs(elapsed);
        recordFlowLoadDebug('preview:response', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'structured-sync-preview',
            elapsedMs: elapsed,
            status: preview.status,
            hasGraph: Boolean(preview.graph),
            backendErrorCount: preview.errors?.length ?? 0,
            ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
        });
        if (preview.diagnostics) {
            setDiagnostics(preview.diagnostics);
        } else {
            clearDiagnostics();
        }
        recordFlowLoadDebug('diagnostics:apply', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'structured-sync-preview',
            ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
        });
        return preview;
    }, [clearDiagnostics, flowName, setDiagnostics]);

    // Auto-load and sync with Backend Preview
    useEffect(() => {
        hydratedRef.current = false;
        if (!flowName) {
            activeFlowLoadIdRef.current += 1;
            recordFlowLoadDebug('flow-load:cleared', null, {
                loadId: activeFlowLoadIdRef.current,
                reason: 'no active flow',
            });
            clearDotSerializationContext();
            setNodes([]);
            setEdges([]);
            replaceGraphAttrs({});
            clearDiagnostics();
            setRawDotDraft('');
            setRawHandoffError(null);
            rawHandoffInFlightRef.current = false;
            setIsRawHandoffInFlight(false);
            setEditorMode('structured');
            rawDotEntryDraftRef.current = '';
            setLastLayoutMs(0);
            setLastPreviewMs(0);
            return;
        }
        const loadId = activeFlowLoadIdRef.current + 1;
        activeFlowLoadIdRef.current = loadId;
        recordFlowLoadDebug('flow-load:start', flowName, {
            loadId,
            session: 'editor',
        });
        clearDotSerializationContext();
        replaceGraphAttrs({});
        clearDiagnostics();
        recordFlowLoadDebug('diagnostics:clear', flowName, {
            loadId,
            reason: 'flow-load reset',
        });
        setRawHandoffError(null);
        rawHandoffInFlightRef.current = false;
        setIsRawHandoffInFlight(false);
        setEditorMode('structured');
        rawDotEntryDraftRef.current = '';

        fetchFlowPayloadValidated(flowName)
            .then((data) => {
                const normalizedContent = normalizeLegacyDot(data.content);
                recordFlowLoadDebug('flow-load:payload', flowName, {
                    loadId,
                    originalLength: data.content.length,
                    normalizedLength: normalizedContent.length,
                    normalizedLegacySyntax: normalizedContent !== data.content,
                });
                setRawDotDraft(normalizedContent);
                return requestPreview(normalizedContent, {
                    loadId,
                    source: 'flow-load-source-dot',
                }).then((preview) => ({
                    normalizedContent,
                    preview,
                }));
            })
            .then(({ normalizedContent, preview }) => hydrateFromPreview(preview, normalizedContent, {
                loadId,
                source: 'flow-load-source-dot',
            }))
            .catch(console.error);
    }, [
        flowName,
        clearDiagnostics,
        hydrateFromPreview,
        requestPreview,
        replaceGraphAttrs,
        setEdges,
        setNodes,
    ]);

    useEffect(() => {
        if (
            !flowName
            || !hydratedRef.current
            || suppressPreview
            || isDragging
            || editorMode === 'raw'
        ) return;
        const dot = generateDot(flowName, nodes, edges, graphAttrs);
        if (previewTimer.current) {
            window.clearTimeout(previewTimer.current);
        }
        recordFlowLoadDebug('preview:schedule', flowName, {
            loadId: activeFlowLoadIdRef.current,
            source: 'structured-sync-preview',
            debounceMs: previewDebounceMs,
            nodeCount: nodes.length,
            edgeCount: edges.length,
            graphAttrCount: Object.keys(graphAttrs).length,
            dotLength: dot.length,
        });
        previewTimer.current = window.setTimeout(() => {
            void requestPreview(dot, {
                loadId: activeFlowLoadIdRef.current,
                source: 'structured-sync-preview',
                nodeCount: nodes.length,
                edgeCount: edges.length,
                graphAttrCount: Object.keys(graphAttrs).length,
                debounceMs: previewDebounceMs,
            }).catch(console.error);
        }, previewDebounceMs);

        return () => {
            if (previewTimer.current) {
                window.clearTimeout(previewTimer.current);
            }
        };
    }, [
        flowName,
        nodes,
        edges,
        graphAttrs,
        requestPreview,
        suppressPreview,
        isDragging,
        editorMode,
        previewDebounceMs,
    ]);

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
                    scheduleSave({ nodes: nextNodes, edges }, EXPECT_SEMANTIC_EQUIVALENCE_OPTIONS);
                } else {
                    scheduleSave({ nodes: nextNodes, edges });
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
                scheduleSave({ nodes, edges: nextEdges });
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
                scheduleSave({ nodes, edges: newEdges });
                return newEdges;
            });
        },
        [setEdges, scheduleSave, nodes],
    );

    const onAddNode = useCallback(() => {
        if (!flowName) return;
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
            scheduleSave({ nodes: newNodes, edges });
            return newNodes;
        });
    }, [flowName, edges, graphAttrs, uiDefaults, setNodes, scheduleSave]);

    const enterRawDotMode = useCallback(() => {
        if (!flowName) return;
        if (editorMode === 'raw') return;
        flushPendingSave();
        const dot = generateDot(flowName, nodes, edges, graphAttrs);
        rawDotEntryDraftRef.current = dot;
        setRawDotDraft(dot);
        setRawHandoffError(null);
        setEditorMode('raw');
    }, [flowName, editorMode, edges, flushPendingSave, graphAttrs, nodes]);

    const returnToStructuredMode = useCallback(async () => {
        if (!flowName) return;
        if (rawHandoffInFlightRef.current) {
            return;
        }
        rawHandoffInFlightRef.current = true;
        setIsRawHandoffInFlight(true);
        try {
            const expectSemanticEquivalence = rawDotEntryDraftRef.current === rawDotDraft;
            const save = expectSemanticEquivalence ? saveFlowContentExpectingSemanticEquivalence : saveFlowContent;
            const saved = await save(flowName, rawDotDraft);
            if (!saved) {
                const latestSaveErrorMessage = useStore.getState().saveErrorMessage;
                setRawHandoffError(
                    `Safe handoff requires valid DOT. ${latestSaveErrorMessage || 'Fix parse or validation errors before switching modes.'}`,
                );
                return;
            }

            try {
                const preview = await requestPreview(rawDotDraft, {
                    loadId: activeFlowLoadIdRef.current,
                    source: 'raw-dot-handoff',
                });
                if (preview.status === 'validation_error' || (preview.errors?.length ?? 0) > 0) {
                    setRawHandoffError(
                        'Raw DOT edit conflicts with structured mode assumptions. Resolve validation errors before switching modes.',
                    );
                    return;
                }
                const hydrated = await hydrateFromPreview(preview, rawDotDraft, {
                    loadId: activeFlowLoadIdRef.current,
                    source: 'raw-dot-handoff',
                });
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
    }, [flowName, hydrateFromPreview, rawDotDraft, requestPreview]);

    const onSelectionChange = useCallback(({ nodes, edges }: OnSelectionChangeParams) => {
        const selectedNode = nodes.find(n => n.selected);
        const selectedEdge = edges.find(e => e.selected);
        setSelectedNodeId(selectedNode ? selectedNode.id : null);
        setSelectedEdgeId(selectedEdge ? selectedEdge.id : null);
    }, [setSelectedEdgeId, setSelectedNodeId]);

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

    useEffect(() => {
        if (selectedNodeId && !nodes.some((node) => node.id === selectedNodeId)) {
            setSelectedNodeId(null);
        }
        if (selectedEdgeId && !edges.some((edge) => edge.id === selectedEdgeId)) {
            setSelectedEdgeId(null);
        }
    }, [edges, nodes, selectedEdgeId, selectedNodeId, setSelectedEdgeId, setSelectedNodeId]);

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
                flowName ? (
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
                        onlyRenderVisibleElements={onlyRenderVisibleElements}
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
                ) : (
                    <div
                        data-testid="editor-no-flow-state"
                        className="flex h-full items-center justify-center p-6"
                    >
                        <div className="max-w-md rounded-lg border border-dashed border-border bg-background/70 px-6 py-5 text-center shadow-sm">
                            <p className="text-sm font-medium text-foreground">Select a flow to begin authoring.</p>
                            <p className="mt-2 text-sm text-muted-foreground">
                                Flows are shared authoring assets. Choose one from the Flows panel.
                            </p>
                        </div>
                    </div>
                )
            )}

            {flowName && (
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
                    <div
                        data-testid="canvas-interaction-performance-budget"
                        data-budget-ms={CANVAS_INTERACTION_BUDGET_MS}
                        className="inline-flex items-center rounded-md border border-border/70 bg-background/90 px-3 py-1.5 text-xs text-muted-foreground shadow-sm"
                    >
                        Canvas interaction budget: {CANVAS_INTERACTION_BUDGET_MS}ms max per interaction frame.
                    </div>
                    <div
                        data-testid="canvas-performance-profile"
                        data-profile={performanceProfile}
                        data-node-count={nodeCount}
                        data-only-render-visible-elements={String(onlyRenderVisibleElements)}
                        data-preview-debounce-ms={previewDebounceMs}
                        data-optimizations={optimizationLabel}
                        data-preview-ms={Math.round(lastPreviewMs)}
                        data-layout-ms={Math.round(lastLayoutMs)}
                        className="inline-flex items-center rounded-md border border-border/70 bg-background/90 px-3 py-1.5 text-xs text-muted-foreground shadow-sm"
                    >
                        Canvas profile: {performanceProfile} ({nodeCount} nodes). Preview debounce: {previewDebounceMs}ms.
                        {' '}Optimizations: {optimizationLabel}.
                    </div>
                </div>
            )}

            {flowName && editorMode === 'structured' && <ValidationPanel />}
        </div>
    );
}
