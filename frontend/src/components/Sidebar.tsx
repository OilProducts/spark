import { useStore } from "@/store"
import { FilePlus, Trash2 } from "lucide-react"
import { useEffect, useState } from "react"
import { useReactFlow, type Node } from "@xyflow/react"
import { generateDot } from "@/lib/dotUtils"

export function Sidebar() {
    const { viewMode, activeFlow, setActiveFlow, selectedNodeId } = useStore()
    const humanGate = useStore((state) => state.humanGate)
    const [tab, setTab] = useState<'flows' | 'edit'>('flows')
    const [flows, setFlows] = useState<string[]>([])
    const { getNodes, setNodes, getEdges, setEdges } = useReactFlow()

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
        const graphName = fileName.replace('.dot', '');

        const initialContent = `digraph ${graphName} {\n  start [shape=Mdiamond, label="Start"];\n  end [shape=Msquare, label="End"];\n  start -> end;\n}`;

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

    const handlePropertyChange = (key: string, value: string) => {
        if (!selectedNodeId || !activeFlow) return;

        let newNodes: Node[] = [];
        setNodes(nds => {
            newNodes = nds.map(node => {
                if (node.id === selectedNodeId) {
                    return { ...node, data: { ...node.data, [key]: value } };
                }
                return node;
            });
            return newNodes;
        });

        // Auto-save
        setTimeout(() => {
            if (newNodes.length > 0) {
                const dot = generateDot(activeFlow, newNodes, getEdges());
                fetch('/api/flows', {
                    method: 'POST',
                    headers: { 'Content-Type': 'application/json' },
                    body: JSON.stringify({ name: activeFlow, content: dot })
                });
            }
        }, 100);
    }

    const selectedNode = getNodes().find(n => n.id === selectedNodeId);
    const activeTab = viewMode === 'execution' ? 'flows' : tab;

    return (
        <nav className="w-72 border-r bg-background flex flex-col shrink-0 overflow-hidden z-40">
            <div className="flex p-4 pb-2">
                <div className="inline-flex h-9 items-center justify-center rounded-md bg-muted p-1 text-muted-foreground w-full">
                    <button
                        onClick={() => setTab('flows')}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 flex-1 ${activeTab === 'flows' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Flows
                    </button>
                    <button
                        onClick={() => setTab('edit')}
                        disabled={viewMode === 'execution'}
                        className={`inline-flex items-center justify-center whitespace-nowrap rounded-sm px-3 py-1 text-sm font-medium ring-offset-background transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-2 flex-1 disabled:opacity-50 disabled:cursor-not-allowed ${activeTab === 'edit' ? 'bg-background text-foreground shadow-sm' : 'hover:text-foreground'
                            }`}
                    >
                        Node
                    </button>
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

                            <div className="space-y-1.5 flex flex-col h-48">
                                <label className="text-sm font-medium">Prompt Instruction</label>
                                <textarea
                                    value={(selectedNode?.data?.prompt as string) || ''}
                                    onChange={(e) => handlePropertyChange('prompt', e.target.value)}
                                    className="flex flex-1 w-full rounded-md border border-input bg-background px-3 py-2 text-xs font-mono shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring resize-none"
                                    placeholder="Enter system prompt instructions..."
                                />
                            </div>
                            {(selectedNode?.data?.shape as string) === 'parallelogram' && (
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
                            {(selectedNode?.data?.shape as string) === 'component' && (
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
                        </div>
                    )}
                </div>
            )}
        </nav>
    )
}
