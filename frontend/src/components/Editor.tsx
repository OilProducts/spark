import { useCallback, useEffect } from 'react';
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

import { useStore } from '@/store';
import { TaskNode } from './TaskNode';
import { generateDot } from '@/lib/dotUtils';

const nodeTypes = {
    customTask: TaskNode,
};
const EDGE_TYPE: Edge['type'] = 'bezier';
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
}

interface PreviewEdge {
    from: string
    to: string
}

interface PreviewResponse {
    graph?: {
        nodes: PreviewNode[]
        edges: PreviewEdge[]
    }
}

function normalizeLegacyDot(content: string): string {
    return content.replace(/\blabel=label=/g, 'label=');
}

export function Editor() {
    const { activeFlow, viewMode, setSelectedNodeId } = useStore();
    const nodeStatuses = useStore((state) => state.nodeStatuses);
    const [nodes, setNodes] = useNodesState<Node>([]);
    const [edges, setEdges] = useEdgesState<Edge>([]);

    const saveFlow = useCallback((nextNodes: Node[], nextEdges: Edge[]) => {
        if (!activeFlow) return;
        const dot = generateDot(activeFlow, nextNodes, nextEdges);
        fetch('/api/flows', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ name: activeFlow, content: dot }),
        }).catch(console.error);
    }, [activeFlow]);

    // Auto-load and sync with Backend Preview
    useEffect(() => {
        if (!activeFlow) return;

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
                if (!preview.graph) return;

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
                        status: 'idle'
                    },
                }));

                const rfEdges: Edge[] = preview.graph.edges.map((e, i: number) => ({
                    id: `e-${e.from}-${e.to}-${i}`,
                    source: e.from,
                    target: e.to,
                    type: EDGE_TYPE,
                    className: EDGE_CLASS,
                }));

                setNodes(rfNodes);
                setEdges(rfEdges);
            })
            .catch(console.error);
    }, [activeFlow, setNodes, setEdges]);

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

    const onSelectionChange = useCallback(({ nodes }: OnSelectionChangeParams) => {
        const selectedNode = nodes.find(n => n.selected);
        setSelectedNodeId(selectedNode ? selectedNode.id : null);
    }, [setSelectedNodeId]);

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
        </div>
    );
}
