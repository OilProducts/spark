import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Handle, NodeToolbar, Position, type Node, type NodeProps, useReactFlow } from '@xyflow/react';
import { useStore } from '@/store';
import { generateDot } from '@/lib/dotUtils';

export function TaskNode({ id, data, selected }: NodeProps) {
    const { activeFlow, viewMode } = useStore();
    const humanGate = useStore((state) => state.humanGate);
    const graphAttrs = useStore((state) => state.graphAttrs);
    const { setNodes, getEdges } = useReactFlow();
    const inputRef = useRef<HTMLInputElement>(null);

    const displayLabel = (data.label as string) || 'Task Node';
    const [isEditingLabel, setIsEditingLabel] = useState(false);
    const [draftLabel, setDraftLabel] = useState(displayLabel);
    const [isEditingDetails, setIsEditingDetails] = useState(false);
    const [draftShape, setDraftShape] = useState<string>((data.shape as string) || 'box');
    const [draftPrompt, setDraftPrompt] = useState<string>((data.prompt as string) || '');
    const [draftToolCommand, setDraftToolCommand] = useState<string>((data.tool_command as string) || '');
    const [draftJoinPolicy, setDraftJoinPolicy] = useState<string>((data.join_policy as string) || 'wait_all');
    const [draftErrorPolicy, setDraftErrorPolicy] = useState<string>((data.error_policy as string) || 'continue');
    const [draftMaxParallel, setDraftMaxParallel] = useState<string>(
        data.max_parallel !== undefined ? String(data.max_parallel) : '4'
    );
    const status = (data.status as string) || 'idle';

    useEffect(() => {
        if (isEditingLabel) {
            inputRef.current?.focus();
            inputRef.current?.select();
        }
    }, [isEditingLabel]);

    const persistNodeData = (nextData: Record<string, unknown>) => {
        if (!activeFlow) return;

        let updatedNodes: Node[] = [];
        setNodes((currentNodes) => {
            updatedNodes = currentNodes.map((node) => {
                if (node.id !== id) return node;
                return { ...node, data: { ...node.data, ...nextData } };
            });
            return updatedNodes;
        });

        if (updatedNodes.length > 0) {
            const dot = generateDot(activeFlow, updatedNodes, getEdges(), graphAttrs);
            fetch('/api/flows', {
                method: 'POST',
                headers: { 'Content-Type': 'application/json' },
                body: JSON.stringify({ name: activeFlow, content: dot }),
            }).catch(console.error);
        }
    };

    const startEditLabel = (event: React.MouseEvent<HTMLDivElement>) => {
        event.stopPropagation();
        setDraftLabel(displayLabel);
        setIsEditingLabel(true);
    };

    const commitLabel = () => {
        const nextLabel = draftLabel.trim() || id;
        setIsEditingLabel(false);
        if (nextLabel !== displayLabel) {
            persistNodeData({ label: nextLabel });
        }
    };

    const cancelLabel = () => {
        setDraftLabel(displayLabel);
        setIsEditingLabel(false);
    };

    const onLabelKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
        if (event.key === 'Enter') {
            event.preventDefault();
            commitLabel();
        } else if (event.key === 'Escape') {
            event.preventDefault();
            cancelLabel();
        }
    };

    const openDetailsEditor = (event: React.MouseEvent<HTMLButtonElement>) => {
        event.stopPropagation();
        setDraftShape((data.shape as string) || 'box');
        setDraftPrompt((data.prompt as string) || '');
        setDraftToolCommand((data.tool_command as string) || '');
        setDraftJoinPolicy((data.join_policy as string) || 'wait_all');
        setDraftErrorPolicy((data.error_policy as string) || 'continue');
        setDraftMaxParallel(data.max_parallel !== undefined ? String(data.max_parallel) : '4');
        setIsEditingDetails(true);
    };

    const closeDetailsEditor = () => {
        setIsEditingDetails(false);
    };

    const saveDetails = () => {
        persistNodeData({
            shape: draftShape,
            prompt: draftPrompt,
            tool_command: draftToolCommand,
            join_policy: draftJoinPolicy,
            error_policy: draftErrorPolicy,
            max_parallel: draftMaxParallel,
        });
        setIsEditingDetails(false);
    };

    const isWaiting = humanGate?.nodeId === id || status === 'waiting';

    let borderColor = 'border-border';
    if (status === 'success') borderColor = 'border-green-500';
    if (status === 'failed') borderColor = 'border-destructive';
    if (status === 'running') borderColor = 'border-primary ring-2 ring-primary ring-offset-2 ring-offset-background';
    if (isWaiting) borderColor = 'border-amber-500 ring-2 ring-amber-500/40 ring-offset-2 ring-offset-background';
    else if (selected) borderColor = 'border-foreground ring-1 ring-ring ring-offset-2 ring-offset-background';

    return (
        <div
            onDoubleClick={startEditLabel}
            className={`bg-card/95 text-card-foreground shadow-sm rounded-md border p-4 min-w-[150px] relative ${borderColor} transition-[color,box-shadow,border-color,background-color] hover:shadow-md`}
        >
            <Handle type="target" position={Position.Top} className="w-3 h-3 bg-muted-foreground border-border" />

            {selected && viewMode === 'editor' && (
                <button
                    onClick={openDetailsEditor}
                    className="absolute right-2 top-2 rounded border border-border bg-background/90 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                >
                    Edit
                </button>
            )}
            {isWaiting && (
                <div className="absolute left-2 top-2 rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-700">
                    Needs Input
                </div>
            )}

            <div className="flex flex-col gap-1 items-center justify-center">
                {isEditingLabel ? (
                    <input
                        ref={inputRef}
                        value={draftLabel}
                        onChange={(event) => setDraftLabel(event.target.value)}
                        onBlur={commitLabel}
                        onKeyDown={onLabelKeyDown}
                        onPointerDown={(event) => event.stopPropagation()}
                        className="nodrag nopan h-7 w-[140px] rounded border border-input bg-background px-2 text-center text-sm font-semibold outline-none ring-0 focus-visible:ring-1 focus-visible:ring-ring"
                    />
                ) : (
                    <span className="text-sm font-semibold">{displayLabel}</span>
                )}
                {status !== 'idle' && (
                    <span className={`text-[10px] px-1.5 py-0.5 rounded-sm uppercase tracking-wider font-medium
            ${status === 'success' ? 'bg-green-500/20 text-green-500' : ''}
            ${status === 'running' ? 'bg-primary/20 text-primary' : ''}
            ${status === 'failed' ? 'bg-destructive/20 text-destructive' : ''}
          `}>
                        {status}
                    </span>
                )}
            </div>

            <Handle type="source" position={Position.Bottom} className="w-3 h-3 bg-muted-foreground border-border" />

            <NodeToolbar
                isVisible={isEditingDetails}
                position={Position.Bottom}
                className="nodrag nopan"
            >
                <div className="mt-2 w-64 rounded-md border border-border bg-card p-3 shadow-lg">
                    <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                        Node Properties
                    </div>
                    <div className="mt-2 space-y-2">
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Shape / Type</label>
                            <select
                                value={draftShape}
                                onChange={(event) => setDraftShape(event.target.value)}
                                className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
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
                        <div className="space-y-1">
                            <label className="text-xs font-medium text-foreground">Prompt</label>
                            <textarea
                                value={draftPrompt}
                                onChange={(event) => setDraftPrompt(event.target.value)}
                                className="nodrag h-20 w-full resize-none rounded-md border border-input bg-background px-2 py-1 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            />
                        </div>
                        {draftShape === 'parallelogram' && (
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Tool Command</label>
                                <input
                                    value={draftToolCommand}
                                    onChange={(event) => setDraftToolCommand(event.target.value)}
                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="e.g. pytest -q"
                                />
                            </div>
                        )}
                        {draftShape === 'component' && (
                            <>
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Join Policy</label>
                                    <select
                                        value={draftJoinPolicy}
                                        onChange={(event) => setDraftJoinPolicy(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    >
                                        <option value="wait_all">Wait All</option>
                                        <option value="first_success">First Success</option>
                                        <option value="k_of_n">K of N</option>
                                        <option value="quorum">Quorum</option>
                                    </select>
                                </div>
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Error Policy</label>
                                    <select
                                        value={draftErrorPolicy}
                                        onChange={(event) => setDraftErrorPolicy(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    >
                                        <option value="continue">Continue</option>
                                        <option value="fail_fast">Fail Fast</option>
                                        <option value="ignore">Ignore</option>
                                    </select>
                                </div>
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Max Parallel</label>
                                    <input
                                        value={draftMaxParallel}
                                        onChange={(event) => setDraftMaxParallel(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="4"
                                    />
                                </div>
                            </>
                        )}
                    </div>
                    <div className="mt-3 flex items-center justify-end gap-2">
                        <button
                            onClick={closeDetailsEditor}
                            className="h-7 rounded-md border border-border px-2 text-[11px] font-medium text-muted-foreground hover:text-foreground"
                        >
                            Cancel
                        </button>
                        <button
                            onClick={saveDetails}
                            className="h-7 rounded-md bg-primary px-2 text-[11px] font-semibold text-primary-foreground hover:bg-primary/90"
                        >
                            Save
                        </button>
                    </div>
                </div>
            </NodeToolbar>

            {humanGate?.nodeId === id && viewMode === 'execution' && (
                <NodeToolbar
                    isVisible
                    position={Position.Bottom}
                    className="nodrag nopan"
                >
                    <div className="mt-2 w-72 rounded-md border border-amber-200 bg-amber-50 p-3 shadow-lg">
                        <div className="text-xs font-semibold uppercase tracking-wide text-amber-700">
                            Human Input Required
                        </div>
                        <div className="mt-2 text-sm text-foreground">
                            {humanGate.prompt || 'Select next route'}
                        </div>
                        <div className="mt-3 flex flex-col gap-2">
                            {humanGate.options.map((option) => (
                                <button
                                    key={option.value}
                                    onClick={() => {
                                        fetch('/human/answer', {
                                            method: 'POST',
                                            headers: { 'Content-Type': 'application/json' },
                                            body: JSON.stringify({
                                                question_id: humanGate.id,
                                                selected_value: option.value,
                                            }),
                                        }).catch(console.error)
                                    }}
                                    className="rounded-md border border-amber-200 bg-white px-2 py-1 text-left text-xs font-medium text-amber-900 hover:bg-amber-100"
                                >
                                    {option.label}
                                </button>
                            ))}
                        </div>
                    </div>
                </NodeToolbar>
            )}
        </div>
    );
}
