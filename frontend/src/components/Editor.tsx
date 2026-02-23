import { useCallback, useEffect, useRef } from 'react';
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

import { useStore, type DiagnosticEntry } from '@/store';
import { TaskNode } from './TaskNode';
import { GraphSettings } from './GraphSettings';
import { ValidationEdge } from './ValidationEdge';
import { ValidationPanel } from './ValidationPanel';
import { generateDot } from '@/lib/dotUtils';

const nodeTypes = {
    customTask: TaskNode,
};
const edgeTypes = {
    validation: ValidationEdge,
};
const EDGE_TYPE: Edge['type'] = 'validation';
const EDGE_CLASS = 'flow-edge';

interface PreviewNode {
    id: string
    label?: string
    shape?: string
    prompt?: string
    tool_command?: string
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
        }
    }
    diagnostics?: DiagnosticEntry[]
    errors?: DiagnosticEntry[]
}

function normalizeLegacyDot(content: string): string {
    return content.replace(/\blabel=label=/g, 'label=');
}

export function Editor() {
    const { activeFlow, viewMode, setSelectedNodeId, setSelectedEdgeId } = useStore();
    const nodeStatuses = useStore((state) => state.nodeStatuses);
    const graphAttrs = useStore((state) => state.graphAttrs);
    const setGraphAttrs = useStore((state) => state.setGraphAttrs);
    const setDiagnostics = useStore((state) => state.setDiagnostics);
    const clearDiagnostics = useStore((state) => state.clearDiagnostics);
    const suppressPreview = useStore((state) => state.suppressPreview);
    const [nodes, setNodes] = useNodesState<Node>([]);
    const [edges, setEdges] = useEdgesState<Edge>([]);
    const hydratedRef = useRef(false);
    const previewTimer = useRef<number | null>(null);

    const saveFlow = useCallback((nextNodes: Node[], nextEdges: Edge[]) => {
        if (!activeFlow) return;
        const dot = generateDot(activeFlow, nextNodes, nextEdges, graphAttrs);
        fetch('/api/flows', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name: activeFlow, content: dot }),
        }).catch(console.error);
    }, [activeFlow, graphAttrs]);

    // Auto-load and sync with Backend Preview
    useEffect(() => {
        hydratedRef.current = false;
        if (!activeFlow) {
            setNodes([]);
            setEdges([]);
            clearDiagnostics();
            return;
        }
        clearDiagnostics();

        fetch(`/api/flows/${activeFlow}`)
            .then((res) => res.json())
            .then((data) => {
                const normalizedContent = normalizeLegacyDot(data.content);
                if (normalizedContent !== data.content) {
                    fetch('/api/flows', {
                        method: 'POST',
                        headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ name: activeFlow, content: normalizedContent }),
                    }).catch(console.error);
                }

                return fetch('/preview', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ flow_content: normalizedContent }),
                });
            })
            .then((res) => res.json())
            .then((preview: PreviewResponse) => {
                if (preview.diagnostics) {
                    setDiagnostics(preview.diagnostics);
                } else {
                    clearDiagnostics();
                }

                if (!preview.graph) return;

                if (preview.graph.graph_attrs) {
                    setGraphAttrs(preview.graph.graph_attrs)
                }

                // Convert Preview JSON graph to ReactFlow format
                const rfNodes: Node[] = preview.graph.nodes.map((n, i: number) => ({
                    id: n.id,
                    type: 'customTask',
                    position: { x: 250, y: i * 150 }, // Auto-layout stub
                    data: {
                        label: n.label,
                        shape: n.shape ?? 'box',
                        prompt: n.prompt ?? '',
                        tool_command: n.tool_command ?? '',
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
                        status: 'idle'
                    },
                }));

                const rfEdges: Edge[] = preview.graph.edges.map((e, i: number) => ({
                    id: `e-${e.from}-${e.to}-${i}`,
                    source: e.from,
                    target: e.to,
                    type: EDGE_TYPE,
                    className: EDGE_CLASS,
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

                setNodes(rfNodes);
                setEdges(rfEdges);
                hydratedRef.current = true;
            })
            .catch(console.error);
    }, [activeFlow, setNodes, setEdges, setGraphAttrs, setDiagnostics, clearDiagnostics]);

    const requestPreview = useCallback((dot: string) => {
        fetch('/preview', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ flow_content: dot }),
        })
            .then((res) => res.json())
            .then((preview: PreviewResponse) => {
                if (preview.diagnostics) {
                    setDiagnostics(preview.diagnostics);
                } else {
                    clearDiagnostics();
                }
            })
            .catch(console.error);
    }, [setDiagnostics, clearDiagnostics]);

    useEffect(() => {
        if (!activeFlow || !hydratedRef.current || viewMode === 'execution' || suppressPreview) return;
        const dot = generateDot(activeFlow, nodes, edges, graphAttrs);
        if (previewTimer.current) {
            window.clearTimeout(previewTimer.current);
        }
        previewTimer.current = window.setTimeout(() => requestPreview(dot), 600);

        return () => {
            if (previewTimer.current) {
                window.clearTimeout(previewTimer.current);
            }
        };
    }, [activeFlow, nodes, edges, graphAttrs, viewMode, requestPreview, suppressPreview]);

    // Handle new connections via UI
    const onNodesChange = useCallback((changes: NodeChange<Node>[]) => {
        setNodes((currentNodes) => {
            const updatedNodes = applyNodeChanges(changes, currentNodes);
            saveFlow(updatedNodes, edges);
            return updatedNodes;
        });
    }, [setNodes, saveFlow, edges]);

    const onEdgesChange = useCallback((changes: EdgeChange<Edge>[]) => {
        setEdges((currentEdges) => {
            const updatedEdges = applyEdgeChanges(changes, currentEdges);
            saveFlow(nodes, updatedEdges);
            return updatedEdges;
        });
    }, [setEdges, saveFlow, nodes]);

    const onConnect = useCallback(
        (params: Connection | Edge) => {
            setEdges((currentEdges) => {
                const newEdges = addEdge(params, currentEdges);
                saveFlow(nodes, newEdges);
                return newEdges;
            });
        },
        [setEdges, saveFlow, nodes],
    );

    const onAddNode = useCallback(() => {
        if (!activeFlow) return;
        const newNodeId = `node_${Math.floor(Math.random() * 10000)}`;
        const newNode: Node = {
            id: newNodeId,
            type: 'customTask',
            position: { x: Math.random() * 200 + 100, y: Math.random() * 200 + 100 },
            data: { label: 'New Node', shape: 'box', status: 'idle' }
        };

        setNodes(nds => {
            const newNodes = [...nds, newNode];
            saveFlow(newNodes, edges);
            return newNodes;
        });
    }, [activeFlow, edges, setNodes, saveFlow]);

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

    return (
        <div className="flow-surface w-full h-full relative">
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
                defaultEdgeOptions={{ type: EDGE_TYPE, className: EDGE_CLASS }}
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

            {viewMode === 'editor' && activeFlow && (
                <div className="absolute left-4 top-4 z-10 flex gap-2">
                    <button
                        onClick={onAddNode}
                        className="bg-primary text-primary-foreground shadow-sm px-3 py-1.5 rounded-md text-sm font-medium hover:bg-primary/90 transition-colors"
                    >
                        Add Node
                    </button>
                </div>
            )}

            {activeFlow && <GraphSettings />}
            {activeFlow && <ValidationPanel />}
        </div>
    );
}
