import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { useCallback, useEffect, useMemo, useRef, useState, type MouseEvent as ReactMouseEvent, type SetStateAction } from 'react';
import {
    ReactFlow,
    Background,
    MarkerType,
    useNodesState,
    useEdgesState,
    useReactFlow,
    applyNodeChanges,
    applyEdgeChanges,
} from '@xyflow/react';
import type { Connection, Edge, EdgeChange, Node, NodeChange, OnSelectionChangeParams } from '@xyflow/react';
import '@xyflow/react/dist/style.css';

import { captureShownWorkspace, useStore, type EditorMode, type FlowDefinitionMetadata } from '@/store';
import { getActiveEnvironmentId } from '@/lib/activeEnvironment';
import { LaunchPanel } from '@/features/launch';
import { defaultProjectChoice, launchWorkingDirectory } from '@/features/projects/model/projectChoices';
import { markProjectUsed } from '@/features/projects/hooks/usePersistProjectState';
import { SheetEmpty } from '@/components/app/view-layout';
import { GitBranch } from 'lucide-react';
import { ValidationPanel } from './components/ValidationPanel';
import {
    buildCanonicalFlowModelFromEditorState,
    buildFlowDefinitionFromCanonicalFlowModel,
    type CanonicalFlowDefinition,
} from '@/lib/canonicalFlowModel';
import type { FlowFile } from '@/lib/attractorClient';
import { recordFlowLoadDebug, summarizeDiagnosticsForFlowLoadDebug } from '@/lib/flowLoadDebug';
import {
    closeFlowOpening,
    primeFlowSaveBaseline,
    saveFlowContent,
    sendPendingFlowSaves,
    waitForFlowSaves,
} from '@/lib/flowPersistence';
import { requestNavigation } from '@/state/workspaceSlice';
import { useDialogController } from '@/components/app/dialog-controller';
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
    CanvasMiniMap,
    edgeTypes,
    EDGE_CLASS,
    EDGE_INTERACTION_WIDTH,
    EDGE_TYPE,
    filterAuthoredEdges,
    filterAuthoredNodes,
    layoutWithElk,
    nodeTypes,
    nowMs,
    useReadableFraming,
} from '@/features/workflow-canvas';
import { getReactFlowNodeTypeForShape, getShapeNodeStyle } from '@/lib/workflowNodeShape';
import { isAbortError } from '@/lib/abortError';
import { isPerformanceDebugEnabled } from '@/lib/performanceDebug';
import { flowLoadFailed, flowLoadProblems, flowProblems, isInvalidFlowError, isMissingFlowError, type FlowProblem } from './model/flowLoadFailure';
import {
    loadEditorFlowFiles,
    loadEditorFlowPayload,
    loadEditorPreview,
    type EditorPreviewResponse,
} from './services/editorPreview';
import { projectLabel } from '@/features/projects/model/projectChoices';
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button';
import { useRegisterEditorGraphBridge } from './EditorGraphBridgeContext';
import { EditorCanvasToolbar } from './components/EditorCanvasToolbar';
import { selectFlowSettingsOpen } from './GraphSettings';
import { EditorSaveStatus } from './components/EditorSaveStatus';
import { flowFileChanges, FlowFilesView, hasFlowFileChanges, ProblemList, type FlowFileDrafts, type FlowFilesError } from './components/FlowFilesView';
import { useSettingsNavigationProtection } from '@/features/settings/hooks/useSettingsNavigationProtection';

const DEFAULT_PREVIEW_DEBOUNCE_MS = 300;
const MEDIUM_GRAPH_PREVIEW_DEBOUNCE_MS = 600;
const MEDIUM_GRAPH_NODE_THRESHOLD = 25;
// The view controls live in the toolbar, and the minimap shows only for a flow too big to show
// whole, or when asked for, so nothing covers a canvas that shows all there is.
// The sides leave room for self-loops, which reach past their node's box.
const FRAME_PADDING = { top: 24, right: 48, bottom: 24, left: 48 } as const;
const MINIMAP_SIZE = { width: 160, height: 108 };
const LIVE_ROUTE_THROTTLE_MS = 80;
const EDITOR_LAYOUT_CANVAS_KIND: FlowCanvasKind = 'editor-parent-only';

type PreviewResponse = EditorPreviewResponse

type FlowGraphSnapshot = {
    nodes: Node[]
    edges: Edge[]
}

type PreviewDebugContext = {
    source: 'flow-load' | 'structured-sync-preview'
    loadId: number | null
    nodeCount?: number
    edgeCount?: number
    flowMetadataCount?: number
    debounceMs?: number
}

type PreparedHydratedPreview = {
    flow: NonNullable<ReturnType<typeof buildHydratedFlowGraph>>['flow']
    flowMetadata: NonNullable<ReturnType<typeof buildHydratedFlowGraph>>['flowMetadata']
    nodes: Node[]
    edges: Edge[]
    serializedNodes: Node[]
    layoutDurationMs: number
    layout: SavedFlowLayoutV1
    edgeIdToLayoutKey: Map<string, string>
}

// A removal saves from the canvas's onDelete, which has the step and its routes together.
export function shouldPersistNodeChanges(changes: NodeChange<Node>[]): boolean {
    return changes.some((change) => {
        return change.type !== 'select' && change.type !== 'position' && change.type !== 'dimensions' && change.type !== 'remove'
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

/** An edit Cmd/Ctrl+Z can take back on the canvas. */
type CanvasEdit =
    | { kind: 'delete'; nodes: Node[]; edges: Edge[] }
    | { kind: 'add-step'; nodeId: string }
    /** A route added, or given again by its ID, such as with a new target; `previous` is what it replaced. */
    | { kind: 'route'; edgeId: string; previous: Edge | null }
    /**
     * An inspector field changed; `nodes` and `edges` are the steps and routes as they were, and
     * `flowMetadata` the flow settings, when the edit was to them.
     */
    | { kind: 'fields'; key: string | null; nodes: Node[]; edges: Edge[]; flowMetadata?: FlowDefinitionMetadata }

/** What the Undo button says it takes back: a field edit names the step it goes back on, now. */
function canvasEditUndoLabel(edit: CanvasEdit, nodes: Node[]): string {
    if (edit.kind === 'delete') return 'Undo delete'
    if (edit.kind === 'add-step') return 'Undo add step'
    if (edit.kind === 'route') return edit.previous ? 'Undo route change' : 'Undo add route'
    if (edit.flowMetadata) return 'Undo flow settings edit'
    if (edit.nodes.length === 1) {
        const node = nodes.find((candidate) => candidate.id === edit.nodes[0].id) ?? edit.nodes[0]
        return `Undo edit to ${String(node.data?.label || node.id)}`
    }
    if (edit.nodes.length > 1) return `Undo edit to ${edit.nodes.length} steps`
    return edit.edges.length > 0 ? 'Undo route edit' : 'Undo edit'
}

/** The graph with an edit taken back, as far as it still fits what is there now. */
function undoCanvasEdit(edit: CanvasEdit, { nodes, edges }: { nodes: Node[]; edges: Edge[] }): { nodes: Node[]; edges: Edge[] } {
    const nodeIds = new Set(nodes.map((node) => node.id));
    if (edit.kind === 'fields') {
        // Only the fields go back: a step stays where it was since moved.
        const previousNodes = new Map(edit.nodes.map((node) => [node.id, node]));
        const previousEdges = new Map(edit.edges.map((edge) => [edge.id, edge]));
        return {
            nodes: nodes.map((node) => {
                const previous = previousNodes.get(node.id);
                return previous ? { ...node, type: previous.type, style: previous.style, data: previous.data } : node;
            }),
            edges: edges.map((edge) => {
                const previous = previousEdges.get(edge.id);
                return previous ? { ...edge, data: previous.data, label: previous.label } : edge;
            }),
        };
    }
    if (edit.kind === 'add-step') {
        return {
            nodes: nodes.filter((node) => node.id !== edit.nodeId),
            edges: edges.filter((edge) => edge.source !== edit.nodeId && edge.target !== edit.nodeId),
        };
    }
    if (edit.kind === 'route') {
        const { previous } = edit;
        if (!previous) return { nodes, edges: edges.filter((edge) => edge.id !== edit.edgeId) };
        const restored = { ...previous, selected: false };
        if (edges.some((edge) => edge.id === edit.edgeId)) {
            return { nodes, edges: edges.map((edge) => (edge.id === edit.edgeId ? restored : edge)) };
        }
        return { nodes, edges: nodeIds.has(previous.source) && nodeIds.has(previous.target) ? [...edges, restored] : edges };
    }
    // Only what still fits: an ID taken meanwhile stays, and a route goes back only between its
    // own steps, never onto another step that now holds a deleted one's ID.
    const deletedIds = new Set(edit.nodes.map((node) => node.id));
    const restored = edit.nodes.filter((node) => !nodeIds.has(node.id));
    const restoredIds = new Set(restored.map((node) => node.id));
    const isOwnStep = (id: string) => restoredIds.has(id) || (!deletedIds.has(id) && nodeIds.has(id));
    const edgeIds = new Set(edges.map((edge) => edge.id));
    return {
        nodes: [...nodes, ...restored.map((node) => ({ ...node, selected: false }))],
        edges: [...edges, ...edit.edges.filter((edge) => (
            !edgeIds.has(edge.id) && isOwnStep(edge.source) && isOwnStep(edge.target)
        )).map((edge) => ({ ...edge, selected: false }))],
    };
}

// Padding around the steps a new step is shown with, and the smallest zoom that still reads. Above
// them is more: room for the new step's badges below the save status in the canvas's top corner.
const NEW_STEP_VIEW_PADDING = 48
const NEW_STEP_VIEW_TOP = 80
const NEW_STEP_MIN_ZOOM = 0.6

/**
 * The viewport after Add step: the whole flow with its new step when it fits at a zoom that still
 * reads, never zoomed in past `zoom`; otherwise as much of it as fits, with the new step, which sits
 * right of the flow level with its top, in view at the right.
 */
export function viewportShowingNewStep(step: NodeRect, others: NodeRect[], canvas: { width: number; height: number }, zoom: number) {
    const rects = [...others, step];
    const left = Math.min(...rects.map((rect) => rect.x));
    const top = Math.min(...rects.map((rect) => rect.y));
    const width = Math.max(...rects.map((rect) => rect.x + rect.width)) - left;
    const height = Math.max(...rects.map((rect) => rect.y + rect.height)) - top;
    const room = { width: Math.max(1, canvas.width - 2 * NEW_STEP_VIEW_PADDING), height: Math.max(1, canvas.height - NEW_STEP_VIEW_TOP - NEW_STEP_VIEW_PADDING) };
    const fit = Math.min(room.width / width, room.height / height);
    const nextZoom = Math.min(zoom, Math.max(fit, Math.min(zoom, NEW_STEP_MIN_ZOOM)));
    const centeredX = (canvas.width - width * nextZoom) / 2 - left * nextZoom;
    const centeredY = NEW_STEP_VIEW_TOP + (room.height - height * nextZoom) / 2 - top * nextZoom;
    return {
        // Too wide: the new step's right edge at the canvas's, the flow running off to the left.
        x: Math.min(centeredX, canvas.width - NEW_STEP_VIEW_PADDING - (step.x + step.width) * nextZoom),
        // Too tall: the flow's top at the canvas's, where the new step is.
        y: Math.max(centeredY, NEW_STEP_VIEW_TOP - top * nextZoom),
        zoom: nextZoom,
    };
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

/** A fresh graph's nodes or edges, marked with the selection the store holds, as the canvas shows it. */
function withStoreSelection<T extends Node | Edge>(items: T[], kind: 'node' | 'edge'): T[] {
    const { selectedNodeId, selectedEdgeId } = useStore.getState();
    const selectedId = kind === 'edge' ? selectedEdgeId : selectedEdgeId ? null : selectedNodeId;
    return items.map((item) => (item.id === selectedId) === Boolean(item.selected) ? item : { ...item, selected: item.id === selectedId });
}

function buildAuthoredFlow(
    flowName: string,
    nodes: Node[],
    edges: Edge[],
    flowMetadata: FlowDefinitionMetadata,
    flow: CanonicalFlowDefinition | null,
) {
    return buildFlowDefinitionFromCanonicalFlowModel(flowName, buildCanonicalFlowModelFromEditorState(flowName, {
        nodes: filterAuthoredNodes(nodes),
        edges: filterAuthoredEdges(edges),
        flowMetadata,
        flow,
    }))
}


export function Editor({ isActive = true, title = null, flowList }: { isActive?: boolean; title?: string | null; flowList?: { failed: boolean; retry: () => void } }) {
    const selectedNodeId = useStore((state) => state.selectedNodeId);
    const selectedEdgeId = useStore((state) => state.selectedEdgeId);
    const setSelectedNodeId = useStore((state) => state.setSelectedNodeId);
    const setSelectedEdgeId = useStore((state) => state.setSelectedEdgeId);
    const pendingEditorNodeSelection = useStore((state) => state.pendingEditorNodeSelection);
    const setPendingEditorNodeSelection = useStore((state) => state.setPendingEditorNodeSelection);
    const editorMode = useStore((state) => state.editorMode);
    const clientPreferencesLoaded = useStore((state) => state.clientPreferencesLoaded);
    const initialModeFlowRef = useRef<string | null>(null);
    const setEditorMode = useStore((state) => state.setEditorMode);
    // A run defaults to the project of the chat you came from, else the last used.
    const runDefaultProjectPath = useStore((state) => defaultProjectChoice(state.projectRegistry, state.chatOriginProjectPath));
    const projectRegistry = useStore((state) => state.projectRegistry);
    const setActiveFlow = useStore((state) => state.setActiveFlow);
    const activeFlow = useStore((state) => state.activeFlow);
    const flowMetadata = useStore((state) => state.flowMetadata);
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
    const replaceFlowMetadata = useStore((state) => state.replaceFlowMetadata);
    const setDiagnostics = useStore((state) => state.setDiagnostics);
    const clearDiagnostics = useStore((state) => state.clearDiagnostics);
    const suppressPreview = useStore((state) => state.suppressPreview);
    const expandChildFlows = useStore((state) => (
        state.activeFlow ? (state.editorExpandChildFlowsByFlow[state.activeFlow] ?? state.preferredExpandChildFlows) : false
    ));
    const setEditorExpandChildFlows = useStore((state) => state.setEditorExpandChildFlows);
    const flowSettingsShown = useStore((state) => !selectedNodeId && !selectedEdgeId && selectFlowSettingsOpen(state));
    const setEditorGraphSettingsPanelOpen = useStore((state) => state.setEditorGraphSettingsPanelOpen);
    const hasValidationErrors = useStore((state) => state.hasValidationErrors);
    const saveState = useStore((state) => state.saveState);
    const workingDir = useStore((state) => state.workingDir);
    const activeProjectPath = useStore((state) => state.activeProjectPath);
    const model = useStore((state) => state.model);
    const setViewMode = useStore((state) => state.setViewMode);
    const { setViewport, getZoom } = useReactFlow();
    const [nodes, setNodes] = useNodesState<Node>([]);
    const [edges, setEdges] = useEdgesState<Edge>([]);
    // Layouts belong to the environment this editor mounted in; one that finishes after a switch is dropped.
    const [environmentId] = useState(getActiveEnvironmentId);
    const hydratedRef = useRef(false);
    const hydratedFlowNameRef = useRef<string | null>(null);
    // A flow belongs to a project: the same name in another project is another flow.
    const hydratedProjectPathRef = useRef<string | null>(null);
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
    const expandChildFlowsRef = useRef(expandChildFlows);
    const canonicalFlowRef = useRef<NonNullable<ReturnType<typeof buildHydratedFlowGraph>>['flow']>(null);
    const canvasGraphRef = useRef({
        nodes: [] as Node[],
        edges: [] as Edge[],
        flowMetadata: {} as FlowDefinitionMetadata,
    });
    const [isDragging, setIsDragging] = useState(false);
    const [flowFiles, setFlowFiles] = useState<FlowFile[]>([]);
    // The file view's unsaved edits by file name, kept here so a mode switch cannot drop them.
    const [fileDrafts, setFileDrafts] = useState<FlowFileDrafts>({});
    const fileDraftsRef = useRef(fileDrafts);
    useEffect(() => { fileDraftsRef.current = fileDrafts; }, [fileDrafts]);
    const [fileSaveError, setFileSaveError] = useState<FlowFilesError | null>(null);
    // The file a broken flow's problem link opens the file view on.
    const [filesFocus, setFilesFocus] = useState<string | null>(null);
    // A flow that couldn't load, and why: a link to a renamed or deleted flow says so instead of
    // opening an empty canvas that could still be run. The inspector hides for it too.
    const flowLoadFailure = useStore((state) => state.flowLoadFailure);
    const setFlowLoadFailure = useStore((state) => state.setFlowLoadFailure);
    // Bumped to reload the active flow, e.g. after the file view saves.
    const [flowReloadToken, setFlowReloadToken] = useState(0);
    const reloadRequestedRef = useRef(false);
    const [runProjectPath, setRunProjectPath] = useState<string | null>(null);
    const isRunPanelOpen = runProjectPath !== null;
    const [isHydrated, setIsHydrated] = useState(false);
    const [lastLayoutMs, setLastLayoutMs] = useState(0);
    const [lastPreviewMs, setLastPreviewMs] = useState(0);

    const nodeCount = nodes.length;
    const isMediumGraph = nodeCount >= MEDIUM_GRAPH_NODE_THRESHOLD;
    const previewDebounceMs = isMediumGraph ? MEDIUM_GRAPH_PREVIEW_DEBOUNCE_MS : DEFAULT_PREVIEW_DEBOUNCE_MS;
    const onlyRenderVisibleElements = isMediumGraph;
    const performanceProfile = isMediumGraph ? 'medium' : 'default';
    const showPerformanceDebug = isPerformanceDebugEnabled();
    const activeOptimizations = [
        ...(onlyRenderVisibleElements ? ['visible-only'] : []),
        ...(previewDebounceMs > DEFAULT_PREVIEW_DEBOUNCE_MS ? ['debounced-preview'] : []),
    ];
    const optimizationLabel = activeOptimizations.length ? activeOptimizations.join(', ') : 'none';
    const flowName = activeFlow;
    const canvasPaneRef = useRef<HTMLDivElement>(null);
    const frame = useReadableFraming(canvasPaneRef);
    // A flow that opens too big to show whole shows where the rest of it is, until the toggle says otherwise.
    const [minimapChoice, setMinimapChoice] = useState<boolean | null>(null);
    const [graphOverflows, setGraphOverflows] = useState(false);
    const showMinimap = minimapChoice ?? graphOverflows;
    // A view switch saves the view it leaves first; the toolbar says so while it does.
    const [switchingEditorMode, setSwitchingEditorMode] = useState(false);
    // A newly opened flow, or a switch between its parent and expanded views, opens framed;
    // edits and reloads of the same graph keep the viewport where the user left it.
    const framedGraphRef = useRef<string | null>(null);
    const frameNewGraph = useCallback((graphNodes: Node[]) => {
        const graphKey = `${flowName}:${expandChildFlowsRef.current}`;
        if (framedGraphRef.current === graphKey) return;
        framedGraphRef.current = graphKey;
        setGraphOverflows(frame(graphNodes, { padding: FRAME_PADDING }) === false);
    }, [frame, flowName]);
    const isExpandedReadOnlyPreview = editorMode === 'structured' && expandChildFlows;
    // The file view edits the files; its canvas only selects nodes.
    const isCanvasReadOnly = isExpandedReadOnlyPreview || editorMode === 'files';
    const connectRef = useRef<(params: Connection | Edge) => void>(() => {});
    const recordFieldEditRef = useRef<(key: string | null, previous: { nodes?: Node[]; edges?: Edge[]; flowMetadata?: FlowDefinitionMetadata }) => void>(() => {});
    // The inspector's and canvas nodes' edits apply to the graph at once, so the caller saves what
    // it set: React may run a state updater only at its next render. A second edit in the same event
    // builds on the first; React applies each to its own latest state, keeping a pending resize.
    const editorGraphBridge = useMemo(() => ({
        getNodes: () => canvasGraphRef.current.nodes,
        setNodes: (update: SetStateAction<Node[]>) => {
            const next = typeof update === 'function' ? update(canvasGraphRef.current.nodes) : update;
            canvasGraphRef.current = { ...canvasGraphRef.current, nodes: next };
            setNodes(update);
        },
        getEdges: () => canvasGraphRef.current.edges,
        setEdges: (update: SetStateAction<Edge[]>) => {
            const next = typeof update === 'function' ? update(canvasGraphRef.current.edges) : update;
            canvasGraphRef.current = { ...canvasGraphRef.current, edges: next };
            setEdges(update);
        },
        getFlow: () => canonicalFlowRef.current,
        connect: (params: Connection | Edge) => connectRef.current(params),
        recordFieldEdit: (key: string | null, previous: { nodes?: Node[]; edges?: Edge[]; flowMetadata?: FlowDefinitionMetadata }) => recordFieldEditRef.current(key, previous),
    }), [setEdges, setNodes]);

    useRegisterEditorGraphBridge(editorGraphBridge);

    useEffect(() => {
        setRunProjectPath(null);
        setFilesFocus(null);
    }, [flowName]);

    const runDisabledReason = hasValidationErrors
            ? 'Fix validation errors before running.'
            : saveState === 'saving'
                ? 'Waiting for the flow to finish saving.'
                : saveState === 'error' || saveState === 'conflict'
                    ? 'Resolve the flow save error before running.'
                    : undefined;

    const fileChangesPending = hasFlowFileChanges(flowFileChanges(flowFiles, fileDrafts));
    // Edits that would be lost on opening another flow: file drafts, and canvas edits a save refused.
    const hasUnsavedEdits = fileChangesPending || saveState === 'error' || saveState === 'conflict';
    // File edits a save refused, since put back or discarded, leave nothing unsaved and no error.
    useEffect(() => {
        if (editorMode !== 'files' || fileChangesPending) return;
        setFileSaveError(null);
        const { saveState, resetSaveState } = useStore.getState();
        if (saveState === 'error' || saveState === 'conflict') resetSaveState();
    }, [editorMode, fileChangesPending]);
    useSettingsNavigationProtection(hasUnsavedEdits, false, 'flow edits');
    // Leaving first sends edits still waiting to save and waits for this environment's saves in
    // flight: a save refused after the click still asks before its edits are dropped, and never
    // lands on the next flow. Another environment's saves only hold up closing the browser.
    const { confirm } = useDialogController();
    useEffect(() => {
        const waitForSaves = (event: Event) => {
            if (event.defaultPrevented || !sendPendingFlowSaves(environmentId)) return;
            event.preventDefault();
            const { proceed } = (event as CustomEvent<{ proceed: () => void }>).detail;
            const shown = captureShownWorkspace();
            void waitForFlowSaves().then(async () => {
                // A switch away ended the navigation: the save's result stays with its own workspace,
                // and nothing asks or moves in the one shown now.
                if (!shown()) return;
                const { saveState, saveErrorMessage, saveErrorProblems } = useStore.getState();
                if (saveState !== 'error' && saveState !== 'conflict') {
                    // Saved: anything else that guards leaving (file drafts) still gets its say.
                    requestNavigation(proceed);
                } else if (await confirm({
                    title: 'Discard unsaved flow edits?',
                    description: `${saveErrorMessage ? `They weren’t saved: ${(saveErrorProblems?.map((problem) => problem.message) ?? flowProblems(saveErrorMessage)).join(' ')} ` : ''}Your unsaved flow edits will be lost.`,
                    confirmLabel: 'Discard and leave',
                    cancelLabel: 'Keep editing',
                    confirmVariant: 'destructive',
                })) {
                    proceed();
                }
            });
        };
        window.addEventListener('spark:before-navigation', waitForSaves, { capture: true });
        return () => window.removeEventListener('spark:before-navigation', waitForSaves, { capture: true });
    }, [confirm, environmentId]);

    useEffect(() => {
        expandChildFlowsRef.current = expandChildFlows;
    }, [expandChildFlows]);

    useEffect(() => {
        canvasGraphRef.current = {
            nodes,
            edges,
            flowMetadata,
        }
    }, [edges, flowMetadata, nodes]);

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

    const { clearPendingSave, flushPendingSave, saveNow, scheduleSave } = useFlowSaveScheduler<FlowGraphSnapshot>({
        flowName,
        debounceMs: 250,
        buildContent: (snapshot, currentFlowName) => ({
            flow: buildAuthoredFlow(
                currentFlowName,
                snapshot?.nodes || [],
                snapshot?.edges || [],
                // The latest: Undo sets flow settings back just before it saves.
                useStore.getState().flowMetadata,
                canonicalFlowRef.current,
            ),
        }),
    })
    // Leaving the flow ends this opening of it, after the save of edits waiting above: a save refused
    // from then on is reported by the flow's notice, never as the save state of whatever is shown here.
    useEffect(() => () => {
        if (flowName) closeFlowOpening(flowName, activeProjectPath, environmentId)
    }, [activeProjectPath, environmentId, flowName])

    const persistLayoutState = useCallback((layout?: SavedFlowLayoutV1 | null, userControlled = true) => {
        if (!flowName || expandChildFlowsRef.current || getActiveEnvironmentId() !== environmentId) {
            return
        }
        const nextLayout = layout ?? layoutStateRef.current
        if (!nextLayout) {
            return
        }
        saveSavedFlowLayout(
            null,
            flowName,
            EDITOR_LAYOUT_CANVAS_KIND,
            nextLayout,
            userControlled,
        )
    }, [environmentId, flowName])

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
        debugContext?: { loadId: number | null; source: PreviewDebugContext['source'] },
        options?: {
            expandChildren?: boolean
            forceFreshLayout?: boolean
        },
    ): Promise<PreparedHydratedPreview | null> => {
        if (!preview.graph) {
            recordFlowLoadDebug('hydrate:skipped', flowName, {
                loadId: debugContext?.loadId ?? null,
                source: debugContext?.source ?? 'flow-load',
                reason: 'preview graph missing',
            });
            return null;
        }
        const hydratedGraph = buildHydratedFlowGraph(
            flowName ?? 'flow',
            preview,
            resolvedUiDefaults,
            options,
        )
        if (!hydratedGraph) {
            return null
        }
        recordFlowLoadDebug('hydrate:start', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'flow-load',
            nodeCount: hydratedGraph.nodes.length,
            edgeCount: hydratedGraph.edges.length,
            flowMetadataCount: Object.keys(hydratedGraph.flowMetadata).length,
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
                : await loadSavedFlowLayout(null, flowName, EDITOR_LAYOUT_CANVAS_KIND);
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
            flow: hydratedGraph.flow,
            flowMetadata: hydratedGraph.flowMetadata,
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
        flowName,
        resolvedUiDefaults,
    ]);

    const requestPreview = useCallback(async (
        flow: CanonicalFlowDefinition,
        debugContext?: PreviewDebugContext,
        signal?: AbortSignal,
        options?: { expandChildren?: boolean },
    ): Promise<{ preview: PreviewResponse; elapsedMs: number }> => {
        recordFlowLoadDebug('preview:request', flowName, {
            loadId: debugContext?.loadId ?? null,
            source: debugContext?.source ?? 'structured-sync-preview',
            nodeCount: debugContext?.nodeCount ?? null,
            edgeCount: debugContext?.edgeCount ?? null,
            flowMetadataCount: debugContext?.flowMetadataCount ?? null,
            debounceMs: debugContext?.debounceMs ?? null,
        });
        const previewStart = nowMs();
        const preview = await loadEditorPreview(
            flow,
            signal ? { signal } : undefined,
            {
                flowName,
                projectPath: activeProjectPath,
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
    }, [activeProjectPath, flowName]);

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
            setNodes([]);
            setEdges([]);
            setSelectedNodeId(null);
            setSelectedEdgeId(null);
            replaceFlowMetadata({});
            canonicalFlowRef.current = null;
            clearDiagnostics();
            setFlowFiles([]);
            setFileDrafts({});
            setFileSaveError(null);
            useStore.getState().resetSaveState();
            setEditorMode('structured');
            hydratedFlowNameRef.current = null;
            setLastLayoutMs(0);
            setLastPreviewMs(0);
            return;
        }

        if (
            hydratedRef.current
            && hydratedFlowNameRef.current === flowName
            && hydratedProjectPathRef.current === activeProjectPath
        ) {
            return;
        }
        // A reload of the hydrated flow keeps the canvas, selection and mode until fresh data lands.
        const isReload = reloadRequestedRef.current
            && hydratedFlowNameRef.current === flowName
            && hydratedProjectPathRef.current === activeProjectPath;
        reloadRequestedRef.current = false;
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
        // The flow opens anew as it starts loading: an earlier opening's save not sent yet never goes over
        // the version this load reads.
        primeFlowSaveBaseline(flowName, null);
        if (!isReload) {
            setNodes([]);
            setEdges([]);
            setSelectedNodeId(null);
            setSelectedEdgeId(null);
            replaceFlowMetadata({});
            canonicalFlowRef.current = null;
            clearDiagnostics();
            recordFlowLoadDebug('diagnostics:clear', flowName, {
                loadId,
                reason: 'flow-load reset',
            });
            setFlowFiles([]);
            setFileDrafts({});
            setFileSaveError(null);
            // The last flow's save outcome is not this one's.
            useStore.getState().resetSaveState();
            setEditorMode('structured');
            setLastLayoutMs(0);
            setLastPreviewMs(0);
        }

        const startScopedLoad = async () => {
            try {
                const data = await loadEditorFlowPayload(flowName, activeProjectPath, { signal: loadAbort.signal });
                if (!isCurrentLoad()) {
                    return;
                }

                recordFlowLoadDebug('flow-load:payload', flowName, {
                    loadId,
                    fileCount: data.files.length,
                });

                const { preview, elapsedMs } = await requestPreview(
                    data.flow,
                    {
                        loadId,
                        source: 'flow-load',
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
                    source: 'flow-load',
                    ...summarizeDiagnosticsForFlowLoadDebug(preview.diagnostics),
                });
                setFlowFiles(data.files);

                const hydrated = await hydrateFromPreview(preview, {
                    loadId,
                    source: 'flow-load',
                }, {
                    expandChildren: expandChildFlowsRef.current,
                });
                if (!isCurrentLoad() || !hydrated) {
                    return;
                }

                canonicalFlowRef.current = hydrated.flow;
                replaceFlowMetadata(hydrated.flowMetadata);
                setLastLayoutMs(hydrated.layoutDurationMs);
                layoutStateRef.current = hydrated.layout;
                edgeIdToLayoutKeyRef.current = hydrated.edgeIdToLayoutKey;
                setNodes(withStoreSelection(hydrated.nodes, 'node'));
                setEdges(withStoreSelection(hydrated.edges, 'edge'));
                frameNewGraph(hydrated.nodes);
                if (!expandChildFlowsRef.current) {
                    persistLayoutState(hydrated.layout, false);
                }
                primeFlowSaveBaseline(flowName, {
                    flow: buildAuthoredFlow(flowName, hydrated.serializedNodes, hydrated.edges, hydrated.flowMetadata, hydrated.flow),
                });
                hydratedRef.current = true;
                setIsHydrated(true);
                useStore.getState().setFlowLoadFailure(null);
                hydratedFlowNameRef.current = flowName;
                hydratedProjectPathRef.current = activeProjectPath;
                recordFlowLoadDebug('hydrate:complete', flowName, {
                    loadId,
                    source: 'flow-load',
                    nodeCount: hydrated.nodes.length,
                    edgeCount: hydrated.edges.length,
                    layoutMs: hydrated.layoutDurationMs,
                });
            } catch (error) {
                if (loadAbort.signal.aborted || isAbortError(error)) {
                    return;
                }
                // A missing or invalid flow is explained on the page; anything else is unexpected.
                if (!isMissingFlowError(error) && !isInvalidFlowError(error)) console.error(error);
                if (isCurrentLoad()) {
                    useStore.getState().setFlowLoadFailure({
                        flow: flowName,
                        project: activeProjectPath,
                        missing: isMissingFlowError(error),
                        problems: flowLoadProblems(error),
                    });
                    // An invalid flow's files still read: they open in the file view to be fixed, in an
                    // opening of their own.
                    if (isInvalidFlowError(error)) {
                        loadEditorFlowFiles(flowName, activeProjectPath, { signal: loadAbort.signal })
                            .then((files) => {
                                if (!isCurrentLoad()) return;
                                setFlowFiles(files);
                                primeFlowSaveBaseline(flowName, null);
                            })
                            .catch(() => undefined);
                    }
                }
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
        activeProjectPath,
        flowReloadToken,
        clearDiagnostics,
        frameNewGraph,
        hydrateFromPreview,
        requestPreview,
        replaceFlowMetadata,
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
            || editorMode !== 'structured'
        ) return;
        const flow = buildAuthoredFlow(flowName, nodes, edges, flowMetadata, canonicalFlowRef.current);
        if (previewTimer.current) {
            window.clearTimeout(previewTimer.current);
        }
        recordFlowLoadDebug('preview:schedule', flowName, {
            loadId: activeFlowLoadIdRef.current,
            source: 'structured-sync-preview',
            debounceMs: previewDebounceMs,
            nodeCount: nodes.length,
            edgeCount: edges.length,
            flowMetadataCount: Object.keys(flowMetadata).length,
        });
        // A preview answered after the editor moved on (an edit, another flow, a switch away) is not its own.
        let current = true;
        previewTimer.current = window.setTimeout(() => {
            void requestPreview(flow, {
                loadId: activeFlowLoadIdRef.current,
                source: 'structured-sync-preview',
                nodeCount: nodes.length,
                edgeCount: edges.length,
                flowMetadataCount: Object.keys(flowMetadata).length,
                debounceMs: previewDebounceMs,
            })
                .then(({ preview, elapsedMs }) => {
                    if (!current) return;
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
            current = false;
            if (previewTimer.current) {
                window.clearTimeout(previewTimer.current);
            }
        };
    }, [
        clearDiagnostics,
        flowName,
        flowMetadata,
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
            // Clicking empty canvas or pressing Escape deselects the step: the inspector goes back to Flow settings.
            if (!latestSelectedNodeChange && changes.some((change) => change.type === 'select' && change.id === useStore.getState().selectedNodeId)) {
                setSelectedNodeId(null);
            }
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
                scheduleSave({ nodes: nextNodes, edges });
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
    }, [edges, enforceSingleSelectedNode, expandChildFlows, scheduleEdgeRouting, scheduleSave, setNodes, setSelectedNodeId]);

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
            if (!latestSelectedEdgeChange && changes.some((change) => change.type === 'select' && change.id === useStore.getState().selectedEdgeId)) {
                setSelectedEdgeId(null);
            }
            if (changes.some((change) => change.type !== 'select')) {
                // A removal saves from onDelete; see shouldPersistNodeChanges.
                if (changes.some((change) => change.type !== 'select' && change.type !== 'remove')) {
                    scheduleSave({ nodes, edges: nextEdges });
                }
                scheduleEdgeRouting(nodes, nextEdges, {
                    persistAfter: true,
                })
            }
            return nextEdges;
        });
    }, [enforceSingleSelectedEdge, expandChildFlows, nodes, scheduleEdgeRouting, scheduleSave, setEdges, setSelectedEdgeId]);

    // The flow's edits, latest last, for Cmd/Ctrl+Z to take back: a deletion it puts back, a step it
    // removes, a route it removes or moves back, an inspector or flow settings field it sets back. It
    // belongs to the open flow: the same name in another project is another flow, so opening either
    // forgets it. Text fields keep their own undo for their typing.
    const editsRef = useRef<CanvasEdit[]>([]);
    // The edit Undo takes back next, which names it.
    const [lastEdit, setLastEdit] = useState<CanvasEdit | null>(null);
    const pushEdit = useCallback((edit: CanvasEdit) => {
        editsRef.current.push(edit);
        setLastEdit(edit);
    }, []);
    // What Undo just took back, said a moment below the canvas; the steps it restored glow, as Go to step's does.
    const [undone, setUndone] = useState<{ text: string; nodeIds: string[] } | null>(null);
    useEffect(() => {
        editsRef.current = [];
        setLastEdit(null);
        setUndone(null);
    }, [flowName, activeProjectPath]);
    useEffect(() => {
        if (!undone) return;
        Array.from(canvasPaneRef.current?.querySelectorAll('.react-flow__node') ?? [])
            .filter((element) => undone.nodeIds.includes(element.getAttribute('data-id') ?? ''))
            .forEach((element) => element.animate?.(
                [{ filter: 'drop-shadow(0 0 10px hsl(var(--primary)))' }, { filter: 'none' }],
                { duration: 1600, easing: 'ease-out' },
            ));
        const timer = window.setTimeout(() => setUndone(null), 2500);
        return () => window.clearTimeout(timer);
    }, [undone]);
    // Typing in one field is one edit: the field as it was before the first key.
    useEffect(() => {
        recordFieldEditRef.current = (key, previous) => {
            const last = editsRef.current.at(-1);
            if (key !== null && last?.kind === 'fields' && last.key === key) return;
            pushEdit({ kind: 'fields', key, nodes: previous.nodes ?? [], edges: previous.edges ?? [], flowMetadata: previous.flowMetadata });
        };
    }, [pushEdit]);

    // React Flow reports a deletion's nodes and edges in separate changes, each handler seeing the
    // other list from before it; here both come together, so the save drops a step with its routes.
    const onDelete = useCallback(({ nodes: deletedNodes, edges: deletedEdges }: { nodes: Node[]; edges: Edge[] }) => {
        pushEdit({ kind: 'delete', nodes: deletedNodes, edges: deletedEdges });
        // Apart: a step's ID can be spelled like a route's generated one.
        const nodeIds = new Set(deletedNodes.map((node) => node.id));
        const edgeIds = new Set(deletedEdges.map((edge) => edge.id));
        scheduleSave({
            nodes: nodes.filter((node) => !nodeIds.has(node.id)),
            edges: edges.filter((edge) => !edgeIds.has(edge.id)),
        });
    }, [edges, nodes, pushEdit, scheduleSave]);

    /** Takes back the latest canvas edit; false when there is none. */
    const undoLastEdit = useCallback(() => {
        const edit = editsRef.current.pop();
        if (!edit) return false;
        setLastEdit(editsRef.current.at(-1) ?? null);
        const { nodes: nextNodes, edges: nextEdges } = undoCanvasEdit(edit, canvasGraphRef.current);
        // A step or route taken back no longer shows in the inspector.
        const { selectedNodeId, selectedEdgeId } = useStore.getState();
        if (selectedNodeId && !nextNodes.some((node) => node.id === selectedNodeId)) setSelectedNodeId(null);
        if (selectedEdgeId && !nextEdges.some((edge) => edge.id === selectedEdgeId)) setSelectedEdgeId(null);
        if (edit.kind === 'fields' && edit.flowMetadata) {
            // Saved below with the graph; a launch input list drafted since shows what came back.
            const { flowMetadata: current, replaceFlowMetadata } = useStore.getState();
            replaceFlowMetadata(edit.flowMetadata);
            if (flowName && current.inputs !== edit.flowMetadata.inputs) {
                useStore.setState((state) => ({
                    editorLaunchInputDraftsByFlow: Object.fromEntries(Object.entries(state.editorLaunchInputDraftsByFlow).filter(([flow]) => flow !== flowName)),
                    editorLaunchInputDraftErrorByFlow: Object.fromEntries(Object.entries(state.editorLaunchInputDraftErrorByFlow).filter(([flow]) => flow !== flowName)),
                }));
            }
        }
        if (edit.kind === 'fields' && edit.nodes.length > 0) {
            // The inspector's drafts of those steps' fields start again from what came back.
            const restoredIds = new Set(edit.nodes.map((node) => node.id));
            useStore.setState((state) => ({
                editorNodeInspectorSessionsByNodeId: Object.fromEntries(Object.entries(state.editorNodeInspectorSessionsByNodeId)
                    .filter(([nodeId]) => !restoredIds.has(nodeId))),
            }));
        }
        setNodes(nextNodes);
        setEdges(nextEdges);
        setUndone({
            text: canvasEditUndoLabel(edit, nextNodes).replace(/^Undo\b/, 'Undid'),
            nodeIds: edit.kind === 'fields' || edit.kind === 'delete' ? edit.nodes.map((node) => node.id) : [],
        });
        scheduleSave({ nodes: nextNodes, edges: nextEdges });
        scheduleEdgeRouting(nextNodes, nextEdges, { persistAfter: true });
        return true;
    }, [flowName, scheduleEdgeRouting, scheduleSave, setEdges, setNodes, setSelectedEdgeId, setSelectedNodeId]);

    useEffect(() => {
        if (!isActive || isCanvasReadOnly) return;
        const undoOnShortcut = (event: KeyboardEvent) => {
            if (event.key.toLowerCase() !== 'z' || !(event.metaKey || event.ctrlKey) || event.shiftKey || event.altKey) return;
            // Text fields keep their own undo.
            if (event.target instanceof Element && event.target.closest('input, textarea, select, [contenteditable="true"]')) return;
            if (undoLastEdit()) event.preventDefault();
        };
        document.addEventListener('keydown', undoOnShortcut);
        return () => document.removeEventListener('keydown', undoOnShortcut);
    }, [isActive, isCanvasReadOnly, undoLastEdit]);

    const onConnect = useCallback(
        (params: Connection | Edge) => {
            if (expandChildFlows) {
                return
            }
            // Cmd/Ctrl+Z removes a new route, or moves a route given again by its ID back where it went.
            const edgeId = buildEdgeIdForConnection(params, canvasGraphRef.current.edges)
            pushEdit({ kind: 'route', edgeId, previous: canvasGraphRef.current.edges.find((edge) => edge.id === edgeId) ?? null })
            setEdges((currentEdges) => {
                // A route is named by its decision: show the one it saves under, as the flow does.
                const isFork = nodes.find((node) => node.id === params.source)?.data?.kind === 'parallel'
                const taken = new Set(currentEdges.filter((edge) => edge.source === params.source).map((edge) => edge.data?.decision))
                let decision = 'next'
                for (let suffix = 2; taken.has(decision); suffix += 1) decision = `next_${suffix}`
                // A route saves only with a description; a new one starts with a plain one to edit.
                const target = nodes.find((node) => node.id === params.target)
                const description = `Continue to ${String(target?.data?.label || params.target)}.`
                const nextEdge: Edge = {
                    ...params,
                    ...(isFork ? {} : { data: { decision, description, ...('data' in params ? params.data : {}) } }),
                    id: edgeId,
                    type: EDGE_TYPE,
                    className: EDGE_CLASS,
                    interactionWidth: EDGE_INTERACTION_WIDTH,
                }
                // A route given again by its ID keeps its place, as the flow keeps its routes in order.
                const newEdges = currentEdges.some((edge) => edge.id === nextEdge.id)
                    ? currentEdges.map((edge) => (edge.id === nextEdge.id ? nextEdge : edge))
                    : [...currentEdges, nextEdge]
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
        [expandChildFlows, nodes, pushEdit, scheduleEdgeRouting, scheduleSave, setEdges],
    );
    // The inspector's route form adds a route through the same path as a drag.
    useEffect(() => { connectRef.current = onConnect; }, [onConnect]);

    const onAddNode = useCallback(() => {
        if (!flowName || expandChildFlows) return;
        // A deleted step that Cmd/Ctrl+Z can still put back keeps its ID.
        const ids = new Set([...nodes, ...editsRef.current.flatMap((edit) => (edit.kind === 'delete' ? edit.nodes : []))].map((node) => node.id));
        let count = 1;
        while (ids.has(`step_${count}`)) count += 1;
        const newNodeId = `step_${count}`;
        const shape = 'box'
        // Beside the graph, level with its top, so it never lands on another step.
        const rects = filterAuthoredNodes(nodes).map(readNodeRect);
        const position = rects.length > 0
            ? { x: Math.max(...rects.map((rect) => rect.x + rect.width)) + 80, y: Math.min(...rects.map((rect) => rect.y)) }
            : { x: 0, y: 0 };
        // No model of its own: the step follows the flow's default, and later changes to it.
        const newNode: Node = {
            id: newNodeId,
            type: getReactFlowNodeTypeForShape(shape),
            style: getShapeNodeStyle(shape),
            position,
            selected: true,
            data: {
                label: count === 1 ? 'New step' : `New step ${count}`,
                kind: 'agent_task',
                shape,
                status: 'idle',
            }
        };

        pushEdit({ kind: 'add-step', nodeId: newNodeId });
        setNodes(nds => {
            // Selected, so the inspector opens on it.
            const newNodes = [...enforceSingleSelectedNode(nds, newNodeId), newNode];
            scheduleSave({ nodes: newNodes, edges });
            scheduleEdgeRouting(newNodes, edges, {
                persistAfter: true,
            })
            return newNodes;
        });
        // In view with the steps it will connect to, and again once the inspector and Validation panel
        // that open for the new step have narrowed the canvas, so it never ends up off screen.
        const rect = readNodeRect(newNode);
        const canvas = canvasPaneRef.current;
        if (!canvas) return;
        const currentZoom = getZoom();
        const show = () => {
            void setViewport(viewportShowingNewStep(rect, rects, { width: canvas.clientWidth, height: canvas.clientHeight }, currentZoom), { duration: 200 });
        };
        show();
        if (typeof ResizeObserver === 'undefined') return;
        const observer = new ResizeObserver(show);
        observer.observe(canvas);
        window.setTimeout(() => observer.disconnect(), 1000);
    }, [edges, enforceSingleSelectedNode, expandChildFlows, flowName, getZoom, nodes, pushEdit, scheduleEdgeRouting, scheduleSave, setNodes, setViewport]);

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

        clearSavedFlowLayout(null, flowName, EDITOR_LAYOUT_CANVAS_KIND)
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
    }, [applyLaidOutGraph, edges, expandChildFlows, flowName, nodes])

    const reloadFlow = useCallback(() => {
        hydratedRef.current = false;
        reloadRequestedRef.current = true;
        setFlowReloadToken((token) => token + 1);
    }, []);

    // After a save conflict, the way out: drop the unsaved edits and load the flow as saved.
    const reloadSavedFlow = useCallback(async () => {
        if (!(await confirm({
            title: 'Reload the saved flow?',
            description: 'Your edits since the last save are replaced by the flow as it is saved now.',
            confirmLabel: 'Reload flow',
            cancelLabel: 'Keep editing',
        }))) return;
        clearPendingSave();
        useStore.getState().resetSaveState();
        reloadFlow();
    }, [clearPendingSave, confirm, reloadFlow]);

    /** Saves the file view's edited files, then reloads the flow so the canvas shows them. */
    const saveFileDrafts = useCallback(async () => {
        if (!flowName) return false;
        const changes = flowFileChanges(flowFiles, fileDrafts);
        if (!hasFlowFileChanges(changes)) return true;
        const saved = await saveFlowContent(flowName, {
            files: changes.files,
            ...(changes.deleted.length > 0 ? { deleted_files: changes.deleted } : {}),
        });
        const { saveErrorMessage, saveErrorProblems } = useStore.getState();
        setFileSaveError(saved ? null : {
            files: [...changes.files.map((file) => file.name), ...changes.deleted],
            problems: saveErrorProblems
                ?? flowProblems(saveErrorMessage || 'The files were not saved.').map((message) => ({ message, file: null })),
        });
        if (saved) reloadFlow();
        return saved;
    }, [fileDrafts, flowFiles, flowName, reloadFlow]);

    const selectEditorMode = useCallback(async (mode: EditorMode, fromPreference = false) => {
        // Before hydration the canvas is empty; saving it would overwrite the flow.
        if (!flowName || !isHydrated || editorMode === mode) return;
        if (fromPreference) {
            setEditorMode(mode);
            return;
        }
        // Each view leaves only once its edits are saved; a refused save keeps them, and its error, in view.
        // A switch of environment or project meanwhile ends it: what is shown is another flow.
        const shown = captureShownWorkspace();
        setSwitchingEditorMode(true);
        try {
            if (editorMode === 'files') {
                const savedDrafts = fileDrafts;
                if (!(await saveFileDrafts()) || !shown()) return;
                // Edits typed while the save ran aren't in it: stay in the files with them unsaved.
                if (fileDraftsRef.current !== savedDrafts) return;
                setFileDrafts({});
                setFileSaveError(null);
            } else {
                // The files show the structured state as saved: save it (a no-op when unchanged, and it
                // covers edits still debounced elsewhere, such as graph settings), then read the files.
                // A refused save is reported by the save indicator.
                flushPendingSave();
                await waitForFlowSaves();
                if (!shown() || !(await saveNow({ nodes, edges })) || !shown()) return;
                reloadFlow();
            }
            setEditorMode(mode);
            useStore.setState({ preferredEditorMode: mode });
            completePreferenceInteraction({ editor_mode: mode });
        } finally {
            setSwitchingEditorMode(false);
        }
    }, [editorMode, edges, fileDrafts, flowName, flushPendingSave, isHydrated, nodes, reloadFlow, saveFileDrafts, saveNow, setEditorMode]);

    useEffect(() => {
        if (!isHydrated) { initialModeFlowRef.current = null; return; }
        if (!clientPreferencesLoaded || hydratedFlowNameRef.current !== flowName || initialModeFlowRef.current === flowName) return;
        initialModeFlowRef.current = flowName;
        if (useStore.getState().preferredEditorMode === 'files') void selectEditorMode('files', true);
    }, [clientPreferencesLoaded, selectEditorMode, flowName, isHydrated]);

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

    // Deep-link entry (e.g. "Open in Editor" from a run): the flow-load effect
    // clears selection mid-load, so a requested node selection waits here until
    // the target flow is hydrated, then applies once and is consumed.
    useEffect(() => {
        if (!pendingEditorNodeSelection || !isHydrated) {
            return;
        }
        if (pendingEditorNodeSelection.flowName !== flowName) {
            if (flowName) {
                // Hydrated into a different flow than the deep link targeted;
                // the request is stale.
                setPendingEditorNodeSelection(null);
            }
            return;
        }
        // Until this flow hydrates, the canvas still holds the last flow's nodes.
        if (hydratedFlowNameRef.current !== flowName) {
            return;
        }
        const { nodeId } = pendingEditorNodeSelection;
        if (nodeId && nodes.some((node) => node.id === nodeId)) {
            setSelectedEdgeId(null);
            setSelectedNodeId(nodeId);
            // The linked step is in view, even in a flow too big to show whole.
            frame(nodes, { padding: FRAME_PADDING, focusNodeId: nodeId });
        }
        setPendingEditorNodeSelection(null);
    }, [
        frame,
        flowName,
        isHydrated,
        nodes,
        pendingEditorNodeSelection,
        setPendingEditorNodeSelection,
        setSelectedEdgeId,
        setSelectedNodeId,
    ]);

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
        const flow = buildAuthoredFlow(
            flowName,
            authoredGraph.nodes,
            authoredGraph.edges,
            authoredGraph.flowMetadata,
            canonicalFlowRef.current,
        )

        void requestPreview(
            flow,
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
                canonicalFlowRef.current = hydrated.flow
                replaceFlowMetadata(hydrated.flowMetadata)
                setLastLayoutMs(hydrated.layoutDurationMs)
                layoutStateRef.current = hydrated.layout
                edgeIdToLayoutKeyRef.current = hydrated.edgeIdToLayoutKey
                // The fresh graph replaces the canvas's nodes; what is selected stays marked on it.
                setNodes(withStoreSelection(hydrated.nodes, 'node'))
                setEdges(withStoreSelection(hydrated.edges, 'edge'))
                frameNewGraph(hydrated.nodes)
                if (!expandChildFlowsRef.current) {
                    persistLayoutState(hydrated.layout, false)
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
        frameNewGraph,
        hydrateFromPreview,
        isHydrated,
        isActive,
        persistLayoutState,
        replaceFlowMetadata,
        requestPreview,
        suppressPreview,
        setEdges,
        setNodes,
    ]);

    const loadFailure = flowLoadFailed(flowLoadFailure, flowName, activeProjectPath) ? flowLoadFailure : null;
    const flowOpen = Boolean(flowName) && !loadFailure;
    // An invalid flow opens no canvas, but its files do.
    const canRepairFiles = Boolean(loadFailure && !loadFailure.missing && flowFiles.length > 0);
    const loadProblems: FlowProblem[] | null = loadFailure && !loadFailure.missing ? loadFailure.problems ?? null : null;
    // A problem's file, while the flow's files have it.
    const flowFileNamed = (file: string | null) => (file && flowFiles.some((flowFile) => flowFile.name === file) ? file : null);
    const linkedLoadProblems = loadProblems?.map((problem) => ({ ...problem, file: flowFileNamed(problem.file) })) ?? null;
    const openFileToFix = (file: string | null) => {
        setFilesFocus(file);
        setEditorMode('files');
    };
    const showFiles = (flowOpen || canRepairFiles) && editorMode === 'files';
    return (
        <div className="flow-surface flex h-full w-full flex-col">
            {flowOpen && flowName && (
                <EditorCanvasToolbar
                    title={title}
                    mode={editorMode}
                    modeDisabled={!isHydrated || switchingEditorMode}
                    switchingMode={switchingEditorMode}
                    childFlowsExpanded={expandChildFlows}
                    hasChildFlows={nodes.some((node) => node.data?.kind === 'subflow')}
                    runDisabledReason={runDisabledReason}
                    onSelectStructured={() => void selectEditorMode('structured')}
                    onSelectFiles={() => void selectEditorMode('files')}
                    onSetChildFlowsExpanded={(expanded) => setEditorExpandChildFlows(flowName, expanded)}
                    onArrange={() => {
                        void onAutoArrange()
                    }}
                    onReset={() => {
                        void onResetSavedLayout()
                    }}
                    onAddNode={onAddNode}
                    undo={isCanvasReadOnly ? undefined : { label: lastEdit ? canvasEditUndoLabel(lastEdit, nodes) : null, onUndo: undoLastEdit }}
                    runDefaultProjectPath={runDefaultProjectPath}
                    minimap={isExpandedReadOnlyPreview ? undefined : { shown: showMinimap, onToggle: () => setMinimapChoice(!showMinimap) }}
                    // Opening them leaves the selected step, so the inspector shows them.
                    flowSettings={{
                        shown: flowSettingsShown,
                        onToggle: () => {
                            if (!flowSettingsShown) {
                                setSelectedNodeId(null);
                                setSelectedEdgeId(null);
                            }
                            setEditorGraphSettingsPanelOpen(flowName, !flowSettingsShown);
                        },
                    }}
                    onRun={(projectPath) => {
                        flushPendingSave()
                        setRunProjectPath(projectPath)
                    }}
                />
            )}
            {flowOpen && editorMode === 'structured' && <EditorSaveStatus onReload={() => void reloadSavedFlow()} />}
            <div className="flex min-h-0 flex-1 flex-col md:flex-row">
            {/* ponytail: phones get the file view alone in Files mode; its list selects files. A flow
                that won't open has no canvas, so its files take the whole width and say what to fix. */}
            <div className={`relative flex min-h-0 min-w-0 flex-1 flex-col ${showFiles ? (canRepairFiles ? 'hidden' : 'hidden md:flex') : ''}`}>
            {/* The canvas, above the Validation panel rather than under it, so no problem hides the graph. */}
            <div ref={canvasPaneRef} className="relative min-h-0 flex-1">
                <div role="status" data-testid="editor-undone" className="pointer-events-none absolute bottom-3 left-1/2 z-10 -translate-x-1/2">
                    {undone ? (
                        <span key={undone.text + undone.nodeIds.join()} className="block whitespace-nowrap rounded-md border bg-background px-2.5 py-1 text-xs font-medium text-foreground shadow-sm animate-in fade-in">
                            {undone.text}
                        </span>
                    ) : null}
                </div>
                {
                    loadFailure ? (
                        <SheetEmpty data-testid="editor-flow-load-failed" icon={GitBranch} title={loadFailure.missing ? 'This flow isn’t here' : 'This flow couldn’t be opened'}>
                            <p>
                                {loadFailure.missing
                                    ? <>{projectLabel(projectRegistry, activeProjectPath)} has no flow named “{loadFailure.flow}”. It may have been renamed or deleted. Pick a flow from the list.</>
                                    : canRepairFiles
                                        ? <>“{loadFailure.flow}” isn’t a valid flow{loadProblems ? <>: {loadProblems.length === 1 ? 'it has a problem' : `it has ${loadProblems.length} problems`}</> : null}. Fix its files, then save to open it.</>
                                        : <>Spark couldn’t load “{loadFailure.flow}”. Try again, or pick another flow from the list.</>}
                            </p>
                            {loadProblems ? (
                                <InlineError data-testid="editor-flow-load-reason" role={undefined} className="mt-3 text-left">
                                    <ProblemList problems={linkedLoadProblems ?? []} onOpenFile={canRepairFiles ? openFileToFix : undefined} />
                                </InlineError>
                            ) : null}
                            <div className="mt-3 flex justify-center gap-2">
                                {canRepairFiles && editorMode !== 'files' ? (
                                    <Button type="button" size="sm" data-testid="editor-flow-edit-files" onClick={() => openFileToFix(null)}>Edit files</Button>
                                ) : null}
                                {loadFailure.missing ? null : (
                                    <Button type="button" size="sm" variant="outline" onClick={() => { setFlowLoadFailure(null); reloadFlow(); }}>Try again</Button>
                                )}
                                <Button type="button" size="sm" variant="ghost" onClick={() => setActiveFlow(null)}>Close</Button>
                            </div>
                        </SheetEmpty>
                    ) : flowName ? (
                        <ReactFlow
                            id="flow-editor"
                            className="flow-canvas"
                            style={{ background: 'transparent' }}
                            nodes={nodes}
                            edges={edges}
                            onNodesChange={onNodesChange}
                            onEdgesChange={onEdgesChange}
                            onDelete={onDelete}
                            onConnect={onConnect}
                            onNodeClick={onNodeClick}
                            onEdgeClick={onEdgeClick}
                            onSelectionChange={onSelectionChange}
                            nodeTypes={nodeTypes}
                            edgeTypes={edgeTypes}
                            nodesDraggable={!isCanvasReadOnly}
                            nodesConnectable={!isCanvasReadOnly}
                            deleteKeyCode={isCanvasReadOnly ? null : undefined}
                            elementsSelectable={!isExpandedReadOnlyPreview}
                            defaultEdgeOptions={{
                                type: EDGE_TYPE,
                                className: EDGE_CLASS,
                                interactionWidth: EDGE_INTERACTION_WIDTH,
                                markerEnd: {
                                    type: MarkerType.ArrowClosed,
                                },
                            }}
                            // Not raised when selected: a raised route would draw over the route labels, which
                            // mask the routes beneath them; a selected route reads by its colour.
                            colorMode="light"
                            onlyRenderVisibleElements={onlyRenderVisibleElements}
                            minZoom={0.1}
                            maxZoom={1.5}
                        >
                            {/* The expanded preview's nodes never reach the minimap, so it would show an empty box. */}
                            {showMinimap && !isExpandedReadOnlyPreview ? (
                                <CanvasMiniMap style={MINIMAP_SIZE} />
                            ) : null}
                            <Background gap={20} size={1} color="hsl(var(--border))" />
                        </ReactFlow>
                    ) : flowList?.failed ? (
                        <SheetEmpty data-testid="editor-flow-list-failed" icon={GitBranch} title="The flows couldn’t load">
                            <p>Spark couldn’t list this project’s flows, so there’s nothing to pick yet.</p>
                            <div className="mt-3 flex justify-center">
                                <Button type="button" size="sm" variant="outline" onClick={flowList.retry}>Try again</Button>
                            </div>
                        </SheetEmpty>
                    ) : (
                        <SheetEmpty data-testid="editor-no-flow-state" icon={GitBranch} title="No flow open">
                            Pick a flow from the list to edit it or run it.
                        </SheetEmpty>
                    )
                }

                {flowOpen && (
                    <div className="absolute left-4 top-4 z-10 flex max-w-[calc(100%-2rem)] flex-wrap gap-2">
                        {showPerformanceDebug ? (
                            <>
                                <div
                                    data-testid="canvas-interaction-performance-budget"
                                    data-budget-ms={CANVAS_INTERACTION_BUDGET_MS}
                                    className="inline-flex items-center rounded-md border border-border/70 bg-background px-3 py-1.5 text-xs text-muted-foreground"
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
                                    className="inline-flex items-center rounded-md border border-border/70 bg-background px-3 py-1.5 text-xs text-muted-foreground"
                                >
                                    Canvas profile: {performanceProfile} ({nodeCount} steps). Preview debounce: {previewDebounceMs}ms.
                                    {' '}Optimizations: {optimizationLabel}.
                                </div>
                            </>
                        ) : null}
                        {isExpandedReadOnlyPreview ? (
                            <div className="inline-flex items-center rounded-md border border-border/70 bg-background px-3 py-1.5 text-xs text-muted-foreground">
                                Read-only preview. Switch to Parent to edit.
                            </div>
                        ) : null}
                    </div>
                )}

            </div>

                {flowOpen && editorMode === 'structured' && <ValidationPanel />}

                {flowOpen && flowName && editorMode === 'structured' && isRunPanelOpen ? (
                    <div
                        data-testid="editor-run-panel"
                        className="absolute bottom-4 right-3 top-2 z-20 flex w-[26rem] max-w-[calc(100%-2rem)] flex-col rounded-lg border border-border bg-background p-4 shadow-lg"
                    >
                        <LaunchPanel
                            target={{
                                flowName,
                                // Runs launch the saved flow by name: persist the current
                                // content first (a no-op when already saved, a retry after
                                // a failed autosave), after any earlier save settles.
                                // A switch of environment or project meanwhile stops the launch: this flow isn't shown there.
                                prepareLaunch: async () => {
                                    const shown = captureShownWorkspace();
                                    await waitForFlowSaves();
                                    const saved = shown() && await saveNow({ nodes, edges });
                                    if (!shown()) {
                                        throw new Error('The environment or project changed before the run started.');
                                    }
                                    if (!saved) {
                                        throw new Error('Saving the flow failed; fix the save error before running.');
                                    }
                                },
                                previewSource: { kind: 'flow', flowName },
                            }}
                            key={runProjectPath}
                            projectPath={runProjectPath}
                            initialWorkingDirectory={launchWorkingDirectory(workingDir, activeProjectPath, runProjectPath)}
                            initialModel={model}
                            onLaunched={() => {
                                if (runProjectPath) void markProjectUsed(runProjectPath);
                                setRunProjectPath(null);
                                setViewMode('runs');
                            }}
                            onClose={() => setRunProjectPath(null)}
                        />
                    </div>
                ) : null}
            </div>
            {showFiles ? (
                <FlowFilesView
                    files={flowFiles}
                    drafts={fileDrafts}
                    error={fileSaveError}
                    loadProblems={canRepairFiles ? (loadProblems ?? []) : null}
                    initialFile={canRepairFiles ? filesFocus ?? linkedLoadProblems?.find((problem) => problem.file)?.file : null}
                    className={canRepairFiles ? 'md:w-full md:flex-1 md:border-l-0' : undefined}
                    selectedNodeId={selectedNodeId}
                    onDraftChange={(name, content) => setFileDrafts((current) => ({ ...current, [name]: content }))}
                    onSave={saveFileDrafts}
                />
            ) : null}
            </div>
        </div>
    );
}
