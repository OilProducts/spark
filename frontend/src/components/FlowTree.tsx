import { FileText, Folder, Trash2 } from 'lucide-react'
import type { MouseEvent, ReactNode } from 'react'
import { useMemo } from 'react'

import { buildFlowTree, type FlowTreeNode } from '@/lib/flowPaths'

interface FlowTreeProps {
    flows: string[]
    selectedFlow: string | null
    onSelectFlow: (flowName: string) => void
    onDeleteFlow?: (event: MouseEvent, flowName: string) => void
    renderFlowIndicator?: (flowName: string) => ReactNode
    dataTestId?: string
}

export function FlowTree({
    flows,
    selectedFlow,
    onSelectFlow,
    onDeleteFlow,
    renderFlowIndicator,
    dataTestId,
}: FlowTreeProps) {
    const tree = useMemo(() => buildFlowTree(flows), [flows])

    return (
        <div data-testid={dataTestId} className="space-y-1">
            {tree.map((node) => (
                <FlowTreeNodeRow
                    key={node.path}
                    node={node}
                    depth={0}
                    selectedFlow={selectedFlow}
                    onSelectFlow={onSelectFlow}
                    onDeleteFlow={onDeleteFlow}
                    renderFlowIndicator={renderFlowIndicator}
                />
            ))}
        </div>
    )
}

interface FlowTreeNodeRowProps {
    node: FlowTreeNode
    depth: number
    selectedFlow: string | null
    onSelectFlow: (flowName: string) => void
    onDeleteFlow?: (event: MouseEvent, flowName: string) => void
    renderFlowIndicator?: (flowName: string) => ReactNode
}

function FlowTreeNodeRow({
    node,
    depth,
    selectedFlow,
    onSelectFlow,
    onDeleteFlow,
    renderFlowIndicator,
}: FlowTreeNodeRowProps) {
    const indent = 12 + depth * 14

    if (node.kind === 'directory') {
        return (
            <div className="space-y-1">
                <div
                    className="flex items-center gap-2 px-3 py-1 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground"
                    style={{ paddingLeft: `${indent}px` }}
                    title={node.path}
                >
                    <Folder className="h-3.5 w-3.5 shrink-0" />
                    <span className="truncate">{node.name}</span>
                </div>
                <div className="space-y-1">
                    {node.children.map((child) => (
                        <FlowTreeNodeRow
                            key={child.path}
                            node={child}
                            depth={depth + 1}
                            selectedFlow={selectedFlow}
                            onSelectFlow={onSelectFlow}
                            onDeleteFlow={onDeleteFlow}
                            renderFlowIndicator={renderFlowIndicator}
                        />
                    ))}
                </div>
            </div>
        )
    }

    return (
        <div className="group relative">
            <button
                aria-label={node.path}
                title={node.path}
                onClick={() => onSelectFlow(node.path)}
                className={`w-full rounded-md px-3 py-2 pr-8 text-left text-sm transition-colors ${
                    selectedFlow === node.path
                        ? 'bg-secondary font-medium text-secondary-foreground'
                        : 'text-muted-foreground hover:bg-muted hover:text-foreground'
                }`}
                style={{ paddingLeft: `${indent}px` }}
            >
                <span className="flex items-center gap-2">
                    <FileText className="h-3.5 w-3.5 shrink-0" />
                    {renderFlowIndicator?.(node.path)}
                    <span className="truncate">{node.name}</span>
                </span>
            </button>
            {onDeleteFlow ? (
                <button
                    aria-label="Delete flow"
                    onClick={(event) => onDeleteFlow(event, node.path)}
                    className="absolute right-2 top-1/2 -translate-y-1/2 p-1 text-muted-foreground opacity-0 transition-all group-hover:opacity-100 hover:text-destructive"
                    title={`Delete ${node.path}`}
                >
                    <Trash2 className="h-3.5 w-3.5" />
                </button>
            ) : null}
        </div>
    )
}
