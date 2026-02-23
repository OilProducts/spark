import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Handle, Position, type Node, type NodeProps, useReactFlow } from '@xyflow/react';
import { useStore } from '@/store';
import { generateDot } from '@/lib/dotUtils';

export function TaskNode({ id, data, selected }: NodeProps) {
    const { activeFlow } = useStore();
    const { setNodes, getEdges } = useReactFlow();
    const inputRef = useRef<HTMLInputElement>(null);

    const displayLabel = (data.label as string) || 'Task Node';
    const [isEditingLabel, setIsEditingLabel] = useState(false);
    const [draftLabel, setDraftLabel] = useState(displayLabel);
    const status = (data.status as string) || 'idle';

    useEffect(() => {
        if (isEditingLabel) {
            inputRef.current?.focus();
            inputRef.current?.select();
        }
    }, [isEditingLabel]);

    const persistLabel = (nextLabel: string) => {
        if (!activeFlow) return;

        let updatedNodes: Node[] = [];
        setNodes((currentNodes) => {
            updatedNodes = currentNodes.map((node) => {
                if (node.id !== id) return node;
                return { ...node, data: { ...node.data, label: nextLabel } };
            });
            return updatedNodes;
        });

        if (updatedNodes.length > 0) {
            const dot = generateDot(activeFlow, updatedNodes, getEdges());
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
            persistLabel(nextLabel);
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

    let borderColor = 'border-border';
    if (status === 'success') borderColor = 'border-green-500';
    if (status === 'failed') borderColor = 'border-destructive';
    if (status === 'running') borderColor = 'border-primary ring-2 ring-primary ring-offset-2 ring-offset-background';
    else if (selected) borderColor = 'border-foreground ring-1 ring-ring ring-offset-2 ring-offset-background';

    return (
        <div
            onDoubleClick={startEditLabel}
            className={`bg-card text-card-foreground shadow-sm rounded-md border p-4 min-w-[150px] relative ${borderColor} transition-colors`}
        >
            <Handle type="target" position={Position.Top} className="w-3 h-3 bg-muted-foreground border-border" />

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
        </div>
    );
}
