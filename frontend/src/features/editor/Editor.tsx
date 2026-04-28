import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent } from 'react';
import {
    ReactFlow,
    MiniMap,
    Controls,
    Background,
    MarkerType,
    useNodesState,
    useEdgesState,
    applyNodeChanges,
    applyEdgeChanges,
} from '@xyflow/react';
import type { Connection, Edge, EdgeChange, Node, NodeChange, OnSelectionChangeParams } from '@xyflow/react';
import '@xyflow/react/dist/style.css';

import { useStore, type GraphAttrs } from '@/store';
import { ValidationPanel } from './components/ValidationPanel';
import { clearDotSerializationContext, generateDot, setDotSerializationContext } from '@/lib/dotUtils';
import { recordFlowLoadDebug, summarizeDiagnosticsForFlowLoadDebug } from '@/lib/flowLoadDebug';
import {
    EXPECT_SEMANTIC_EQUIVALENCE_OPTIONS,
    primeFlowSaveBaseline,
    saveFlowContentExpectingSemanticEquivalence,
} from '@/lib/flowPersistence';
import { CANVAS_INTERACTION_BUDGET_MS } from '@/lib/performanceBudgets';
import { useFlowSaveScheduler } from '@/lib/useFlowSaveScheduler';
import {
    attachRenderRoutesToEdges,
    buildEdgeLayoutAssignments,
    buildFixedNodeRouterRequest,
    buildSavedFlowLayout,
    buildEdgeLayoutKeyMap,
    computeFlowTopologyStamp,
    edgeLayoutAssignmentsDiffer,
    readNodeRect,
    type LaidOutFlowGraph,
} from '@/lib/flowLayout';
import {
    clearSavedFlowLayout,
    loadSavedFlowLayout,
    saveSavedFlowLayout,
    type FlowCanvasKind,
    type SavedFlowLayoutV1,
} from '@/lib/flowLayoutPersistence';
import { routeFixedNodeGraphInWorker } from '@/lib/flowLayoutRouterClient';
import { routeIntersectsRect, type NodeRect, type RouteSide } from '@/lib/edgeRouting';
import {
    buildHydratedFlowGraph,
    ChildFlowExpansionToggle,
    edgeTypes,
    EDGE_CLASS,
    EDGE_INTERACTION_WIDTH,
    EDGE_TYPE,
    filterAuthoredEdges,
    filterAuthoredNodes,
    layoutWithElk,
    nodeTypes,
    normalizeLegacyDot,
    nowMs,
} from '@/features/workflow-canvas';
import { getReactFlowNodeTypeForShape, getShapeNodeStyle } from '@/lib/workflowNodeShape';
import { Button } from '@/components/ui/button';
import { Textarea } from '@/components/ui/textarea';
import { isAbortError } from '@/lib/api/shared';
import {
    loadEditorFlowPayload,
    loadEditorPreview,
    type EditorPreviewResponse,
} from './services/editorPreview';
import { useRegisterEditorGraphBridge } from './EditorGraphBridgeContext';

const DEFAULT_PREVIEW_DEBOUNCE_MS = 300;
const MEDIUM_GRAPH_PREVIEW_DEBOUNCE_MS = 600;
const MEDIUM_GRAPH_NODE_THRESHOLD = 25;
const LIVE_ROUTE_THROTTLE_MS = 80;
const EDITOR_LAYOUT_CANVAS_KIND: FlowCanvasKind = 'editor-parent-only';

type PreviewResponse = EditorPreviewResponse

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

type PreparedHydratedPreview = {
    defaults: NonNullable<ReturnType<typeof buildHydratedFlowGraph>>['defaults']
    subgraphs: NonNullable<ReturnType<typeof buildHydratedFlowGraph>>['subgraphs']
    graphAttrs: NonNullable<ReturnType<typeof buildHydratedFlowGraph>>['graphAttrs']
    nodes: Node[]
    edges: Edge[]
    serializedNodes: Node[]
    layoutDurationMs: number
    layout: SavedFlowLayoutV1
    edgeIdToLayoutKey: Map<string, string>
}

export function shouldPersistNodeChanges(changes: NodeChange<Node>[]): boolean {
    return changes.some((change) => {
        return change.type !== 'select' && change.type !== 'position' && change.type !== 'dimensions'
    })
}

function readHandleSide(handleId: string | null | undefined): RouteSide | null {
    if (!handleId) {
        return null
    }
    if (handleId.endsWith('top')) {
        return 'top'
    }
    if (handleId.endsWith('right')) {
        return 'right'
    }
    if (handleId.endsWith('bottom')) {
        return 'bottom'
    }
    if (handleId.endsWith('left')) {
        return 'left'
    }
    return null
}

function buildEdgeIdForConnection(params: Connection | Edge, currentEdges: Edge[]): string {
    if ('id' in params && typeof params.id === 'string' && params.id.trim().length > 0) {
        return params.id
    }
    return `e-${params.source}-${params.target}-${currentEdges.length}`
}

function buildDirtyRect(previousRect: NodeRect | null, nextRect: NodeRect | null): NodeRect | null {
    if (!previousRect && !nextRect) {
        return null
    }
    if (!previousRect) {
        return nextRect
    }
    if (!nextRect) {
        return previousRect
    }

    const left = Math.min(previousRect.x, nextRect.x)
    const top = Math.min(previousRect.y, nextRect.y)
    const right = Math.max(previousRect.x + previousRect.width, nextRect.x + nextRect.width)
    const bottom = Math.max(previousRect.y + previousRect.height, nextRect.y + nextRect.height)

    return {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

function buildAuthoredDot(
    flowName: string,
    nodes: Node[],
    edges: Edge[],
    graphAttrs: GraphAttrs,
) {
    return generateDot(
        flowName,
        filterAuthoredNodes(nodes),
        filterAuthoredEdges(edges),
        graphAttrs,
    )
}

export function Editor({ isActive = true }: { isActive?: boolean }) {
    const selectedNodeId = useStore((state) => state.selectedNodeId);
    const selectedEdgeId = useStore((state) => state.selectedEdgeId);
    const setSelectedNodeId = useStore((state) => state.setSelectedNodeId);
    const setSelectedEdgeId = useStore((state) => state.setSelectedEdgeId);
    const editorMode = useStore((state) => state.editorMode);
    const setEditorMode = useStore((state) => state.setEditorMode);
    const rawDotDraft = useStore((state) => state.rawDotDraft);
    const setRawDotDraft = useStore((state) => state.setRawDotDraft);
    const rawHandoffError = useStore((state) => state.rawHandoffError);
    const setRawHandoffError = useStore((state) => state.setRawHandoffError);
    const activeProjectPath = useStore((state) => state.activeProjectPath);
    const activeFlow = useStore((state) => state.activeFlow);
    const graphAttrs = useStore((state) => state.graphAttrs);
    const uiDefaults = useStore((state) => state.uiDefaults);
    const uiDefaultModel = uiDefaults.llm_model;
    const uiDefaultProvider = uiDefaults.llm_provider;
    const uiDefaultProfile = uiDefaults.llm_profile;
    const uiDefaultReasoningEffort = uiDefaults.reasoning_effort;
    const resolvedUiDefaults = useMemo(() => ({
        llm_model: uiDefaultModel,
        llm_provider: uiDefaultProvider,
        llm_profile: uiDefaultProfile,
        reasoning_effort: uiDefaultReasoningEffort,
    }), [uiDefaultModel, uiDefaultProvider, uiDefaultProfile, uiDefaultReasoningEffort]);
    const replaceGraphAttrs = useStore((state) => state.replaceGraphAttrs);
    const setDiagnostics = useStore((state) => state.setDiagnostics);
    const clearDiagnostics = useStore((state) => state.clearDiagnostics);
    const suppressPreview = useStore((state) => state.suppressPreview);
    const expandChildFlows = useStore((state) => (
        state.activeFlow ? (state.editorExpandChildFlowsByFlow[state.activeFlow] ?? false) : false
    ));
    const setEditorExpandChildFlows = useStore((state) => state.setEditorExpandChildFlows);
    const [nodes, setNodes] = useNodesState<Node>([]);
    const [edges, setEdges] = useEdgesState<Edge>([]);
    const hydratedRef = useRef(false);
    const hydratedFlowNameRef = useRef<string | null>(null);
    const previewTimer = useRef<number | null>(null);
    const activeFlowLoadIdRef = useRef(0);
    const liveRouteTimerRef = useRef<number | null>(null);
    const lastLiveRouteDispatchAtRef = useRef(0);
    const queuedRouteJobRef = useRef<{
        nodes: Node[]
        edges: Edge[]
        persistAfter: boolean
        dirtyNodeIds: Set<string>
        dirtyRects: NodeRect[]
    } | null>(null);
    const layoutStateRef = useRef<SavedFlowLayoutV1 | null>(null);
    const edgeIdToLayoutKeyRef = useRef<Map<string, string>>(new Map());
    const edgeSideIntentRef = useRef<Record<string, { sourceSide?: RouteSide; targetSide?: RouteSide }>>({});
    const routeRevisionRef = useRef(0);
    const rawDotEntryDraftRef = useRef<string>('');
    const rawHandoffInFlightRef = useRef(false);
    const expandChildFlowsRef = useRef(expandChildFlows);
    const canvasGraphRef = useRef({
        nodes: [] as Node[],
        edges: [] as Edge[],
        graphAttrs: {} as GraphAttrs,
    });
    const [isDragging, setIsDragging] = useState(false);
    const [isRawHandoffInFlight, setIsRawHandoffInFlight] = useState(false);
    const [isHydrated, setIsHydrated] = useState(false);
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
    const isExpandedReadOnlyPreview = editorMode === 'structured' && expandChildFlows;
    const editorGraphBridge = useMemo(() => ({
        getNodes: () => canvasGraphRef.current.nodes,
        setNodes,
        getEdges: () => canvasGraphRef.current.edges,
        setEdges,
    }), [setEdges, setNodes]);

    useRegisterEditorGraphBridge(editorGraphBridge);

    useEffect(() => {
        expandChildFlowsRef.current = expandChildFlows;
    }, [expandChildFlows]);

    useEffect(() => {
        canvasGraphRef.current = {
            nodes,
            edges,
            graphAttrs,
        }
    }, [edges, graphAttrs, nodes]);

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
            filterAuthoredNodes(snapshot?.nodes || []),
            filterAuthoredEdges(snapshot?.edges || []),
            graphAttrs,
        ),
    })

    const persistLayoutState = useCallback((layout?: SavedFlowLayoutV1 | null) => {
        if (!flowName || expandChildFlowsRef.current) {
            return
        }
        const nextLayout = layout ?? layoutStateRef.current
        if (!nextLayout) {
            return
        }
        saveSavedFlowLayout(
            activeProjectPath,
            flowName,
            EDITOR_LAYOUT_CANVAS_KIND,
            nextLayout,
        )
    }, [activeProjectPath, flowName])

    const runEdgeRouting = useCallback(async (
        nextNodes: Node[],
        nextEdges: Edge[],
        options?: {
            persistAfter?: boolean
            dirtyNodeIds?: Set<string>
            dirtyRects?: NodeRect[]
        },
    ) => {
        const currentFlowName = flowName
        if (!currentFlowName || expandChildFlowsRef.current) {
            return
        }

        const previousLayout = layoutStateRef.current
        const topologyStamp = computeFlowTopologyStamp(nextNodes, nextEdges)
        const { assignments, edgeIdToLayoutKey } = buildEdgeLayoutAssignments(
            nextNodes,
            nextEdges,
            previousLayout,
            edgeSideIntentRef.current,
        )
        const dirtyLayoutKeys = new Set<string>()

        if (!previousLayout || previousLayout.topologyStamp !== topologyStamp) {
            Object.keys(assignments).forEach((layoutKey) => dirtyLayoutKeys.add(layoutKey))
        } else {
            Object.values(assignments).forEach((assignment) => {
                if (edgeLayoutAssignmentsDiffer(previousLayout.edgeLayouts[assignment.layoutKey], assignment)) {
                    dirtyLayoutKeys.add(assignment.layoutKey)
                }
            })
        }

        if (options?.dirtyNodeIds && options.dirtyNodeIds.size > 0) {
            Object.values(assignments).forEach((assignment) => {
                if (
                    options.dirtyNodeIds?.has(assignment.source)
                    || options.dirtyNodeIds?.has(assignment.target)
                ) {
                    dirtyLayoutKeys.add(assignment.layoutKey)
                }
            })
        }

        if (options?.dirtyRects && previousLayout) {
            Object.entries(previousLayout.edgeLayouts).forEach(([layoutKey, savedEdgeLayout]) => {
                if (options.dirtyRects?.some((dirtyRect) => routeIntersectsRect(savedEdgeLayout.route, dirtyRect, 12))) {
                    dirtyLayoutKeys.add(layoutKey)
                }
            })
        }

        if (dirtyLayoutKeys.size > 0) {
            const affectedSourceGroups = new Set<string>()
            const affectedTargetGroups = new Set<string>()
            dirtyLayoutKeys.forEach((layoutKey) => {
                const assignment = assignments[layoutKey]
                if (!assignment) {
                    return
                }
                affectedSourceGroups.add(`${assignment.source}:${assignment.sourceSide}`)
                affectedTargetGroups.add(`${assignment.target}:${assignment.targetSide}`)
            })
            Object.values(assignments).forEach((assignment) => {
                if (
                    affectedSourceGroups.has(`${assignment.source}:${assignment.sourceSide}`)
                    || affectedTargetGroups.has(`${assignment.target}:${assignment.targetSide}`)
                ) {
                    dirtyLayoutKeys.add(assignment.layoutKey)
                }
            })
        }

        routeRevisionRef.current += 1
        const revision = routeRevisionRef.current

        const routedEdges = dirtyLayoutKeys.size > 0
            ? await routeFixedNodeGraphInWorker(
                buildFixedNodeRouterRequest(nextNodes, assignments, previousLayout, dirtyLayoutKeys),
            )
            : {}

        if (routeRevisionRef.current !== revision) {
            return
        }

        const nextLayout = buildSavedFlowLayout(
            nextNodes,
            assignments,
            routedEdges,
            topologyStamp,
            previousLayout,
        )
        layoutStateRef.current = nextLayout
        edgeIdToLayoutKeyRef.current = edgeIdToLayoutKey
        edgeSideIntentRef.current = Object.fromEntries(
            Object.entries(edgeSideIntentRef.current)
                .filter(([layoutKey]) => Boolean(assignments[layoutKey])),
        )
        setEdges((currentEdges) => attachRenderRoutesToEdges(currentEdges, edgeIdToLayoutKey, nextLayout))
        if (options?.persistAfter) {
            persistLayoutState(nextLayout)
        }
    }, [flowName, persistLayoutState, setEdges])

    const scheduleEdgeRouting = useCallback((
        nextNodes: Node[],
        nextEdges: Edge[],
        options?: {
            persistAfter?: boolean
            dirtyNodeIds?: Set<string>
            dirtyRects?: NodeRect[]
            throttleMs?: number
        },
    ) => {
        const dispatch = (job: NonNullable<typeof queuedRouteJobRef.current>) => {
            lastLiveRouteDispatchAtRef.current = Date.now()
            void runEdgeRouting(job.nodes, job.edges, {
                persistAfter: job.persistAfter,
                dirtyNodeIds: job.dirtyNodeIds,
                dirtyRects: job.dirtyRects,
            })
        }

        const nextJob = {
            nodes: nextNodes,
            edges: nextEdges,
            persistAfter: options?.persistAfter ?? false,
            dirtyNodeIds: options?.dirtyNodeIds ?? new Set<string>(),
            dirtyRects: options?.dirtyRects ?? [],
        }

        if (!options?.throttleMs || options.throttleMs <= 0) {
            if (liveRouteTimerRef.current) {
                window.clearTimeout(liveRouteTimerRef.current)
                liveRouteTimerRef.current = null
            }
            queuedRouteJobRef.current = null
            dispatch(nextJob)
            return
        }

        queuedRouteJobRef.current = nextJob
        const elapsedMs = Date.now() - lastLiveRouteDispatchAtRef.current
        const remainingMs = Math.max(0, options.throttleMs - elapsedMs)

        if (remainingMs === 0 && !liveRouteTimerRef.current) {
            const readyJob = queuedRouteJobRef.current
            queuedRouteJobRef.current = null
            if (readyJob) {
                dispatch(readyJob)
            }
            return
        }

        if (!liveRouteTimerRef.current) {
            liveRouteTimerRef.current = window.setTimeout(() => {
                liveRouteTimerRef.current = null
                const readyJob = queuedRouteJobRef.current
                queuedRouteJobRef.current = null
                if (readyJob) {
                    dispatch(readyJob)
                }
            }, remainingMs)
        }
    }, [runEdgeRouting])

    const hydrateFromPreview = useCallback(async (
        preview: PreviewResponse,
        sourceDot?: string,
        debugContext?: { loadId: number | null; source: PreviewDebugContext['source'] },
        options?: {
            expandChildren?: boolean
            forceFreshLayout?: boolean
        },
    ): Promise<PreparedHydratedPreview | null> => {
        if (!preview.graph) {
            recordFlowLoadDebug('hydrate:skipped', flowName, {
                loadId: debugContext?.loadId ?? null,
                source: debugContext?.source ?? 'flow-load-source-dot',
                reason: 'preview graph missing',
            });
            return null;
        }
        const hydratedGraph = buildHydratedFlowGraph(
            flowName ?? 'flow',
            preview,
            resolvedUiDefaults,
            sourceDot,
            options,
        )
        if (!hydratedGraph) {
            return null
        }
        recordFlowLoadDebug('hydrate:start', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load-source-dot',
            sourceDotLength: sourceDot?.length ?? null,
            nodeCount: hydratedGraph.nodes.length,
            edgeCount: hydratedGraph.edges.length,
            graphAttrCount: Object.keys(hydratedGraph.graphAttrs).length,
        })

        const layoutStart = nowMs();
        let layoutDurationMs = 0;
        let serializedNodes = hydratedGraph.nodes;
        let laidOutEdges = hydratedGraph.edges;
        let laidOutNodes = hydratedGraph.nodes;
        let nextLayout: SavedFlowLayoutV1 | null = null;
        let edgeIdToLayoutKey = new Map<string, string>();
        try {
            const savedLayout = options?.expandChildren || !flowName
                ? null
                : loadSavedFlowLayout(activeProjectPath, flowName, EDITOR_LAYOUT_CANVAS_KIND);
            const layoutGraph = await layoutWithElk(hydratedGraph.nodes, hydratedGraph.edges, {
                savedLayout,
                forceFreshLayout: options?.forceFreshLayout,
            });
            laidOutNodes = layoutGraph.nodes;
            laidOutEdges = layoutGraph.edges;
            serializedNodes = layoutGraph.nodes;
            nextLayout = layoutGraph.layout;
            edgeIdToLayoutKey = layoutGraph.edgeIdToLayoutKey;
        } catch (error) {
            console.error('ELK layout failed, using fallback positions.', error);
            serializedNodes = hydratedGraph.nodes;
        } finally {
            layoutDurationMs = Math.max(0, nowMs() - layoutStart);
        }
        return {
            defaults: hydratedGraph.defaults,
            subgraphs: hydratedGraph.subgraphs,
            graphAttrs: hydratedGraph.graphAttrs,
            nodes: laidOutNodes,
            edges: laidOutEdges,
            serializedNodes,
            layoutDurationMs,
            layout: nextLayout ?? {
                version: 1,
                topologyStamp: computeFlowTopologyStamp(laidOutNodes, laidOutEdges),
                nodePositions: Object.fromEntries(
                    laidOutNodes.map((node) => [node.id, { x: node.position.x, y: node.position.y }] as const),
                ),
                edgeLayouts: {},
            },
            edgeIdToLayoutKey,
        };
    }, [
        activeProjectPath,
        flowName,
        resolvedUiDefaults,
    ]);

    const requestPreview = useCallback(async (
        dot: string,
        debugContext?: PreviewDebugContext,
        signal?: AbortSignal,
        options?: { expandChildren?: boolean },
    ): Promise<{ preview: PreviewResponse; elapsedMs: number }> => {
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
        const preview = await loadEditorPreview(
            dot,
            signal ? { signal } : undefined,
            {
                flowName,
                expandChildren: options?.expandChildren ?? expandChildFlowsRef.current,
            },
        )
        const elapsedMs = Math.max(0, nowMs() - previewStart);
        recordFlowLoadDebug('preview:response', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'structured-sync-preview',
            elapsedMs,
            status: preview.status,
            hasGraph: Boolean(preview.graph),
            backendErrorCount: preview.errors?.length ?? 0,
            ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
        });
        return { preview, elapsedMs };
    }, [flowName]);

    // Auto-load and sync with Backend Preview
    useEffect(() => {
        if (!isActive) {
            return;
        }
        if (!flowName) {
            hydratedRef.current = false;
            setIsHydrated(false);
            activeFlowLoadIdRef.current += 1;
            routeRevisionRef.current += 1;
            layoutStateRef.current = null;
            edgeIdToLayoutKeyRef.current = new Map();
            edgeSideIntentRef.current = {};
            queuedRouteJobRef.current = null;
            if (liveRouteTimerRef.current) {
                window.clearTimeout(liveRouteTimerRef.current);
                liveRouteTimerRef.current = null;
            }
            recordFlowLoadDebug('flow-load:cleared', null, {
                loadId: activeFlowLoadIdRef.current,
                reason: 'no active flow',
            });
            clearDotSerializationContext();
            setNodes([]);
            setEdges([]);
            setSelectedNodeId(null);
            setSelectedEdgeId(null);
            replaceGraphAttrs({});
            clearDiagnostics();
            setRawDotDraft('');
            setRawHandoffError(null);
            rawHandoffInFlightRef.current = false;
            setIsRawHandoffInFlight(false);
            setEditorMode('structured');
            rawDotEntryDraftRef.current = '';
            hydratedFlowNameRef.current = null;
            setLastLayoutMs(0);
            setLastPreviewMs(0);
            return;
        }

        if (hydratedRef.current && hydratedFlowNameRef.current === flowName) {
            return;
        }
        hydratedRef.current = false;
        setIsHydrated(false);

        const loadId = activeFlowLoadIdRef.current + 1;
        activeFlowLoadIdRef.current = loadId;
        routeRevisionRef.current += 1;
        layoutStateRef.current = null;
        edgeIdToLayoutKeyRef.current = new Map();
        edgeSideIntentRef.current = {};
        queuedRouteJobRef.current = null;
        if (liveRouteTimerRef.current) {
            window.clearTimeout(liveRouteTimerRef.current);
            liveRouteTimerRef.current = null;
        }
        const loadAbort = new AbortController();
        let cancelled = false;
        const isCurrentLoad = () => !cancelled && activeFlowLoadIdRef.current === loadId;

        recordFlowLoadDebug('flow-load:start', flowName, {
            loadId,
            session: 'editor',
        });
        clearDotSerializationContext();
        setNodes([]);
        setEdges([]);
        setSelectedNodeId(null);
        setSelectedEdgeId(null);
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
        setRawDotDraft('');
        setLastLayoutMs(0);
        setLastPreviewMs(0);

        const startScopedLoad = async () => {
            try {
                const data = await loadEditorFlowPayload(flowName, { signal: loadAbort.signal });
                if (!isCurrentLoad()) {
                    return;
                }

                const normalizedContent = normalizeLegacyDot(data.content);
                recordFlowLoadDebug('flow-load:payload', flowName, {
                    loadId,
                    originalLength: data.content.length,
                    normalizedLength: normalizedContent.length,
                    normalizedLegacySyntax: normalizedContent !== data.content,
                });

                const { preview, elapsedMs } = await requestPreview(
                    normalizedContent,
                    {
                        loadId,
                        source: 'flow-load-source-dot',
                    },
                    loadAbort.signal,
                    {
                        expandChildren: expandChildFlowsRef.current,
                    },
                );
                if (!isCurrentLoad()) {
                    return;
                }

                setLastPreviewMs(elapsedMs);
                if (preview.diagnostics) {
                    setDiagnostics(preview.diagnostics);
                } else {
                    clearDiagnostics();
                }
                recordFlowLoadDebug('diagnostics:apply', flowName, {
                    loadId,
                    source: 'flow-load-source-dot',
                    ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
                });
                setRawDotDraft(normalizedContent);

                const hydrated = await hydrateFromPreview(preview, normalizedContent, {
                    loadId,
                    source: 'flow-load-source-dot',
                }, {
                    expandChildren: expandChildFlowsRef.current,
                });
                if (!isCurrentLoad() || !hydrated) {
                    return;
                }

                setDotSerializationContext({
                    defaults: hydrated.defaults,
                    subgraphs: hydrated.subgraphs,
                });
                replaceGraphAttrs(hydrated.graphAttrs);
                setLastLayoutMs(hydrated.layoutDurationMs);
                layoutStateRef.current = hydrated.layout;
                edgeIdToLayoutKeyRef.current = hydrated.edgeIdToLayoutKey;
                setNodes(hydrated.nodes);
                setEdges(hydrated.edges);
                if (!expandChildFlowsRef.current) {
                    persistLayoutState(hydrated.layout);
                }
                primeFlowSaveBaseline(
                    flowName,
                    buildAuthoredDot(flowName, hydrated.serializedNodes, hydrated.edges, hydrated.graphAttrs),
                );
                hydratedRef.current = true;
                setIsHydrated(true);
                hydratedFlowNameRef.current = flowName;
                recordFlowLoadDebug('hydrate:complete', flowName, {
                    loadId,
                    source: 'flow-load-source-dot',
                    nodeCount: hydrated.nodes.length,
                    edgeCount: hydrated.edges.length,
                    layoutMs: hydrated.layoutDurationMs,
                });
            } catch (error) {
                if (loadAbort.signal.aborted || isAbortError(error)) {
                    return;
                }
                console.error(error);
            }
        };

        void startScopedLoad();

        return () => {
            cancelled = true;
            loadAbort.abort();
        };
    }, [
        isActive,
        flowName,
        clearDiagnostics,
        hydrateFromPreview,
        requestPreview,
        replaceGraphAttrs,
        persistLayoutState,
        setEdges,
        setNodes,
        setSelectedEdgeId,
        setSelectedNodeId,
        setDiagnostics,
    ]);

    useEffect(() => {
        if (
            !flowName
            || !isHydrated
            || suppressPreview
            || isDragging
            || editorMode === 'raw'
        ) return;
        const dot = buildAuthoredDot(
            flowName,
            nodes,
            edges,
            graphAttrs,
        );
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
            })
                .then(({ preview, elapsedMs }) => {
                    setLastPreviewMs(elapsedMs);
                    if (preview.diagnostics) {
                        setDiagnostics(preview.diagnostics);
                    } else {
                        clearDiagnostics();
                    }
                    recordFlowLoadDebug('diagnostics:apply', flowName, {
                        loadId: activeFlowLoadIdRef.current,
                        source: 'structured-sync-preview',
                        ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
                    });
                })
                .catch(console.error);
        }, previewDebounceMs);

        return () => {
            if (previewTimer.current) {
                window.clearTimeout(previewTimer.current);
            }
        };
    }, [
        clearDiagnostics,
        flowName,
        graphAttrs,
        requestPreview,
        setDiagnostics,
        suppressPreview,
        isDragging,
        isHydrated,
        editorMode,
        edges,
        nodes,
        previewDebounceMs,
    ]);

    // Handle new connections via UI
    const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {
        if (expandChildFlows) {
            return
        }
        setNodes((currentNodes) => {
            const previousNodeRects = new Map(currentNodes.map((node) => [node.id, readNodeRect(node)]))
            const updatedNodes = applyNodeChanges(changes, currentNodes);
            const latestSelectedNodeChange = [...changes].reverse().find(
                (change): change is NodeChange<Node> & { type: 'select'; id: string; selected: boolean } =>
                    change.type === 'select' && change.selected === true
            );
            const nextNodes = latestSelectedNodeChange
                ? enforceSingleSelectedNode(updatedNodes, latestSelectedNodeChange.id)
                : updatedNodes;
            const dirtyNodeIds = new Set<string>()
            const dirtyRects: NodeRect[] = []
            changes.forEach((change) => {
                if (change.type !== 'position' && change.type !== 'dimensions') {
                    return
                }
                dirtyNodeIds.add(change.id)
                const nextNode = nextNodes.find((node) => node.id === change.id)
                const dirtyRect = buildDirtyRect(
                    previousNodeRects.get(change.id) ?? null,
                    nextNode ? readNodeRect(nextNode) : null,
                )
                if (dirtyRect) {
                    dirtyRects.push(dirtyRect)
                }
            })
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

            const shouldSave = shouldPersistNodeChanges(changes);
            const hasGeometryChange = changes.some(
                (change) => change.type === 'position' || change.type === 'dimensions',
            )

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

            if (hasGeometryChange) {
                scheduleEdgeRouting(nextNodes, edges, {
                    dirtyNodeIds,
                    dirtyRects,
                    persistAfter: draggingStopped || !draggingNow,
                    throttleMs: draggingNow ? LIVE_ROUTE_THROTTLE_MS : 0,
                })
            }
            return nextNodes;
        });
    }, [edges, enforceSingleSelectedNode, expandChildFlows, scheduleEdgeRouting, scheduleSave, setNodes]);

    const onEdgesChange = useCallback((changes: EdgeChange<Edge>[]) => {
        if (expandChildFlows) {
            return
        }
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
                scheduleEdgeRouting(nodes, nextEdges, {
                    persistAfter: true,
                })
            }
            return nextEdges;
        });
    }, [enforceSingleSelectedEdge, expandChildFlows, nodes, scheduleEdgeRouting, scheduleSave, setEdges]);

    const onConnect = useCallback(
        (params: Connection | Edge) => {
            if (expandChildFlows) {
                return
            }
            setEdges((currentEdges) => {
                const nextEdge: Edge = {
                    ...params,
                    id: buildEdgeIdForConnection(params, currentEdges),
                    type: EDGE_TYPE,
                    className: EDGE_CLASS,
                    interactionWidth: EDGE_INTERACTION_WIDTH,
                }
                const newEdges = [...currentEdges, nextEdge]
                const edgeIdToLayoutKey = buildEdgeLayoutKeyMap(newEdges)
                const layoutKey = edgeIdToLayoutKey.get(nextEdge.id)
                if (layoutKey) {
                    const sourceSide = readHandleSide(params.sourceHandle)
                    const targetSide = readHandleSide(params.targetHandle)
                    edgeSideIntentRef.current = {
                        ...edgeSideIntentRef.current,
                        [layoutKey]: {
                            ...(sourceSide ? { sourceSide } : {}),
                            ...(targetSide ? { targetSide } : {}),
                        },
                    }
                }
                scheduleSave({ nodes, edges: newEdges });
                scheduleEdgeRouting(nodes, newEdges, {
                    persistAfter: true,
                })
                return newEdges;
            });
        },
        [expandChildFlows, nodes, scheduleEdgeRouting, scheduleSave, setEdges],
    );

    const onAddNode = useCallback(() => {
        if (!flowName || expandChildFlows) return;
        const defaultModel = graphAttrs.ui_default_llm_model || uiDefaults.llm_model || '';
        const defaultProfile = graphAttrs.ui_default_llm_profile || uiDefaults.llm_profile || '';
        const defaultProvider = defaultProfile ? '' : (graphAttrs.ui_default_llm_provider || uiDefaults.llm_provider || '');
        const defaultReasoning = graphAttrs.ui_default_reasoning_effort || uiDefaults.reasoning_effort || '';
        const newNodeId = `node_${Math.floor(Math.random() * 10000)}`;
        const shape = 'box'
        const newNode: Node = {
            id: newNodeId,
            type: getReactFlowNodeTypeForShape(shape),
            style: getShapeNodeStyle(shape),
            position: { x: Math.random() * 200 + 100, y: Math.random() * 200 + 100 },
            data: {
                label: 'New Node',
                shape,
                status: 'idle',
                llm_model: defaultModel,
                llm_provider: defaultProvider,
                llm_profile: defaultProfile,
                reasoning_effort: defaultReasoning,
            }
        };

        setNodes(nds => {
            const newNodes = [...nds, newNode];
            scheduleSave({ nodes: newNodes, edges });
            scheduleEdgeRouting(newNodes, edges, {
                persistAfter: true,
            })
            return newNodes;
        });
    }, [edges, expandChildFlows, flowName, graphAttrs, scheduleEdgeRouting, scheduleSave, setNodes, uiDefaults]);

    const applyLaidOutGraph = useCallback((layoutGraph: LaidOutFlowGraph) => {
        layoutStateRef.current = layoutGraph.layout
        edgeIdToLayoutKeyRef.current = layoutGraph.edgeIdToLayoutKey
        setNodes(layoutGraph.nodes)
        setEdges(layoutGraph.edges)
        persistLayoutState(layoutGraph.layout)
    }, [persistLayoutState, setEdges, setNodes])

    const onAutoArrange = useCallback(async () => {
        if (!flowName || expandChildFlows) {
            return
        }

        routeRevisionRef.current += 1
        if (liveRouteTimerRef.current) {
            window.clearTimeout(liveRouteTimerRef.current)
            liveRouteTimerRef.current = null
        }
        queuedRouteJobRef.current = null
        edgeSideIntentRef.current = {}
        const layoutStart = nowMs()
        const layoutGraph = await layoutWithElk(nodes, edges, {
            forceFreshLayout: true,
        })
        setLastLayoutMs(Math.max(0, nowMs() - layoutStart))
        applyLaidOutGraph(layoutGraph)
    }, [applyLaidOutGraph, edges, expandChildFlows, flowName, nodes])

    const onResetSavedLayout = useCallback(async () => {
        if (!flowName || expandChildFlows) {
            return
        }

        clearSavedFlowLayout(activeProjectPath, flowName, EDITOR_LAYOUT_CANVAS_KIND)
        edgeSideIntentRef.current = {}
        routeRevisionRef.current += 1
        if (liveRouteTimerRef.current) {
            window.clearTimeout(liveRouteTimerRef.current)
            liveRouteTimerRef.current = null
        }
        queuedRouteJobRef.current = null
        const layoutStart = nowMs()
        const layoutGraph = await layoutWithElk(nodes, edges, {
            forceFreshLayout: true,
        })
        setLastLayoutMs(Math.max(0, nowMs() - layoutStart))
        applyLaidOutGraph(layoutGraph)
    }, [activeProjectPath, applyLaidOutGraph, edges, expandChildFlows, flowName, nodes])

    const enterRawDotMode = useCallback(() => {
        if (!flowName) return;
        if (editorMode === 'raw') return;
        flushPendingSave();
        const dot = buildAuthoredDot(flowName, nodes, edges, graphAttrs);
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
            const rawDotChanged = rawDotEntryDraftRef.current !== rawDotDraft;
            if (rawDotChanged) {
                const saved = await saveFlowContentExpectingSemanticEquivalence(flowName, rawDotDraft);
                if (!saved) {
                    const latestSaveErrorMessage = useStore.getState().saveErrorMessage;
                    setRawHandoffError(
                        `Safe handoff requires valid DOT. ${latestSaveErrorMessage || 'Fix parse or validation errors before switching modes.'}`,
                    );
                    return;
                }
            }

            try {
                const { preview, elapsedMs } = await requestPreview(rawDotDraft, {
                    loadId: activeFlowLoadIdRef.current,
                    source: 'raw-dot-handoff',
                }, undefined, {
                    expandChildren: expandChildFlowsRef.current,
                });
                setLastPreviewMs(elapsedMs);
                if (preview.diagnostics) {
                    setDiagnostics(preview.diagnostics);
                } else {
                    clearDiagnostics();
                }
                recordFlowLoadDebug('diagnostics:apply', flowName, {
                    loadId: activeFlowLoadIdRef.current,
                    source: 'raw-dot-handoff',
                    ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
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
                }, {
                    expandChildren: expandChildFlowsRef.current,
                });
                if (!hydrated) {
                    setRawHandoffError('Safe handoff requires valid DOT. Preview response did not include a graph.');
                    return;
                }
                setDotSerializationContext({
                    defaults: hydrated.defaults,
                    subgraphs: hydrated.subgraphs,
                });
                replaceGraphAttrs(hydrated.graphAttrs);
                setLastLayoutMs(hydrated.layoutDurationMs);
                layoutStateRef.current = hydrated.layout;
                edgeIdToLayoutKeyRef.current = hydrated.edgeIdToLayoutKey;
                setNodes(hydrated.nodes);
                setEdges(hydrated.edges);
                if (!expandChildFlowsRef.current) {
                    persistLayoutState(hydrated.layout);
                }
                primeFlowSaveBaseline(
                    flowName,
                    buildAuthoredDot(flowName, hydrated.serializedNodes, hydrated.edges, hydrated.graphAttrs),
                );
                hydratedRef.current = true;
                setIsHydrated(true);
                recordFlowLoadDebug('hydrate:complete', flowName, {
                    loadId: activeFlowLoadIdRef.current,
                    source: 'raw-dot-handoff',
                    nodeCount: hydrated.nodes.length,
                    edgeCount: hydrated.edges.length,
                    layoutMs: hydrated.layoutDurationMs,
                });
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
    }, [
        clearDiagnostics,
        flowName,
        hydrateFromPreview,
        persistLayoutState,
        rawDotDraft,
        replaceGraphAttrs,
        requestPreview,
        setDiagnostics,
        setEdges,
        setNodes,
    ]);

    const onSelectionChange = useCallback(({ nodes, edges }: OnSelectionChangeParams) => {
        const selectedNode = nodes.find(n => n.selected);
        const selectedEdge = edges.find(e => e.selected);
        if (selectedNode) {
            setSelectedNodeId(selectedNode.id);
            setSelectedEdgeId(null);
            return
        }
        if (selectedEdge) {
            setSelectedNodeId(null);
            setSelectedEdgeId(selectedEdge.id);
        }
    }, [setSelectedEdgeId, setSelectedNodeId]);

    const onNodeClick = useCallback((_event: ReactMouseEvent, node: Node) => {
        if (isExpandedReadOnlyPreview || node.selectable === false) {
            return
        }
        setNodes((currentNodes) => enforceSingleSelectedNode(currentNodes, node.id));
    }, [enforceSingleSelectedNode, isExpandedReadOnlyPreview, setNodes]);

    const onEdgeClick = useCallback((_event: ReactMouseEvent, edge: Edge) => {
        if (isExpandedReadOnlyPreview || edge.selectable === false) {
            return
        }
        setEdges((currentEdges) => enforceSingleSelectedEdge(currentEdges, edge.id));
    }, [enforceSingleSelectedEdge, isExpandedReadOnlyPreview, setEdges]);

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

    useEffect(() => {
        if (
            !isActive
            || !flowName
            || !isHydrated
            || editorMode !== 'structured'
            || (suppressPreview && !expandChildFlows)
        ) {
            return
        }

        const controller = new AbortController()
        const authoredGraph = canvasGraphRef.current
        const dot = buildAuthoredDot(
            flowName,
            authoredGraph.nodes,
            authoredGraph.edges,
            authoredGraph.graphAttrs,
        )

        void requestPreview(
            dot,
            {
                loadId: activeFlowLoadIdRef.current,
                source: 'structured-sync-preview',
            },
            controller.signal,
            {
                expandChildren: expandChildFlows,
            },
        )
            .then(async ({ preview, elapsedMs }) => {
                if (controller.signal.aborted) {
                    return
                }
                setLastPreviewMs(elapsedMs)
                const hydrated = await hydrateFromPreview(
                    preview,
                    dot,
                    {
                        loadId: activeFlowLoadIdRef.current,
                        source: 'structured-sync-preview',
                    },
                    {
                        expandChildren: expandChildFlows,
                    },
                )
                if (controller.signal.aborted || !hydrated) {
                    return
                }
                setDotSerializationContext({
                    defaults: hydrated.defaults,
                    subgraphs: hydrated.subgraphs,
                })
                replaceGraphAttrs(hydrated.graphAttrs)
                setLastLayoutMs(hydrated.layoutDurationMs)
                layoutStateRef.current = hydrated.layout
                edgeIdToLayoutKeyRef.current = hydrated.edgeIdToLayoutKey
                setNodes(hydrated.nodes)
                setEdges(hydrated.edges)
                if (!expandChildFlowsRef.current) {
                    persistLayoutState(hydrated.layout)
                }
            })
            .catch((error) => {
                if (controller.signal.aborted || isAbortError(error)) {
                    return
                }
                console.error(error)
            })

        return () => {
            controller.abort()
        }
    }, [
        expandChildFlows,
        flowName,
        hydrateFromPreview,
        isHydrated,
        isActive,
        persistLayoutState,
        replaceGraphAttrs,
        requestPreview,
        suppressPreview,
        setEdges,
        setNodes,
    ]);

    return (
        <div className="flow-surface w-full h-full relative">
            {editorMode === 'raw' ? (
                <div className="h-full w-full p-4">
                    <div className="h-full rounded-lg border border-border bg-background/80 p-3">
                        <Textarea
                            data-testid="raw-dot-editor"
                            value={rawDotDraft}
                            onChange={(event) => {
                                setRawDotDraft(event.target.value);
                                setRawHandoffError(null);
                            }}
                            className="h-full w-full resize-none font-mono text-xs leading-5"
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
                        onNodeClick={onNodeClick}
                        onEdgeClick={onEdgeClick}
                        onSelectionChange={onSelectionChange}
                        nodeTypes={nodeTypes}
                        edgeTypes={edgeTypes}
                        nodesDraggable={!isExpandedReadOnlyPreview}
                        nodesConnectable={!isExpandedReadOnlyPreview}
                        elementsSelectable={!isExpandedReadOnlyPreview}
                        defaultEdgeOptions={{
                            type: EDGE_TYPE,
                            className: EDGE_CLASS,
                            interactionWidth: EDGE_INTERACTION_WIDTH,
                            markerEnd: {
                                type: MarkerType.ArrowClosed,
                            },
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
                        <Button
                            onClick={() => {
                                if (editorMode === 'raw') {
                                    void returnToStructuredMode();
                                    return;
                                }
                                setEditorMode('structured');
                            }}
                            disabled={editorMode === 'raw' && isRawHandoffInFlight}
                            variant={editorMode === 'structured' ? 'default' : 'ghost'}
                            size="sm"
                            className={`px-3 ${
                                editorMode === 'structured'
                                    ? ''
                                    : 'text-muted-foreground hover:text-foreground'
                            }`}
                        >
                            Structured
                        </Button>
                        <Button
                            onClick={enterRawDotMode}
                            disabled={editorMode === 'raw'}
                            variant={editorMode === 'raw' ? 'default' : 'ghost'}
                            size="sm"
                            className={`px-3 ${
                                editorMode === 'raw'
                                    ? ''
                                    : 'text-muted-foreground hover:text-foreground'
                            }`}
                        >
                            Raw DOT
                        </Button>
                    </div>
                    {editorMode === 'structured' && (
                        <ChildFlowExpansionToggle
                            expanded={expandChildFlows}
                            onChange={(nextExpanded) => {
                                if (!flowName) {
                                    return
                                }
                                setEditorExpandChildFlows(flowName, nextExpanded)
                            }}
                            testId="editor-child-flow-toggle"
                        />
                    )}
                    {editorMode === 'structured' && !expandChildFlows && (
                        <Button
                            onClick={() => {
                                void onAutoArrange()
                            }}
                            className="shadow-sm"
                            variant="outline"
                        >
                            Auto Arrange
                        </Button>
                    )}
                    {editorMode === 'structured' && !expandChildFlows && (
                        <Button
                            onClick={() => {
                                void onResetSavedLayout()
                            }}
                            className="shadow-sm"
                            variant="outline"
                        >
                            Reset Saved Layout
                        </Button>
                    )}
                    {editorMode === 'structured' && !expandChildFlows && (
                        <Button
                            onClick={onAddNode}
                            className="shadow-sm"
                        >
                            Add Node
                        </Button>
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
                    {isExpandedReadOnlyPreview ? (
                        <div className="inline-flex items-center rounded-md border border-border/70 bg-background/90 px-3 py-1.5 text-xs text-muted-foreground shadow-sm">
                            Expanded child-flow mode is a read-only canvas preview. Switch to Parent Only to edit.
                        </div>
                    ) : null}
                </div>
            )}

            {flowName && editorMode === 'structured' && <ValidationPanel />}
        </div>
    );
}
