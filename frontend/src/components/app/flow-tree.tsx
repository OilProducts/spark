import { ChevronRight, FileText, Folder, FolderOpen, Trash2 } from 'lucide-react'
import type { MouseEvent, ReactNode } from 'react'
import { useMemo, useState } from 'react'

import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
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
            <FlowTreeDirectoryRow
                node={node}
                depth={depth}
                selectedFlow={selectedFlow}
                onSelectFlow={onSelectFlow}
                onDeleteFlow={onDeleteFlow}
                renderFlowIndicator={renderFlowIndicator}
            />
        )
    }

    return (
        <div className="group relative">
            <Button
                type="button"
                aria-label={node.path}
                title={node.path}
                onClick={() => onSelectFlow(node.path)}
                variant={selectedFlow === node.path ? 'secondary' : 'ghost'}
                className={`h-9 w-full justify-start rounded-md px-3 py-2 pr-8 text-left text-sm transition-colors ${
                    selectedFlow === node.path
                        ? 'font-medium text-secondary-foreground'
                        : 'text-muted-foreground hover:text-foreground'
                }`}
                style={{ paddingLeft: `${indent}px` }}
            >
                <span className="flex items-center gap-2">
                    <FileText className="h-3.5 w-3.5 shrink-0" />
                    {renderFlowIndicator?.(node.path)}
                    <span className="truncate">{node.name}</span>
                </span>
            </Button>
            {onDeleteFlow ? (
                <Button
                    type="button"
                    aria-label="Delete flow"
                    onClick={(event) => {
                        event.stopPropagation()
                        onDeleteFlow(event, node.path)
                    }}
                    variant="ghost"
                    size="icon-xs"
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground opacity-0 transition-all group-hover:opacity-100 hover:text-destructive"
                    title={`Delete ${node.path}`}
                >
                    <Trash2 className="h-3.5 w-3.5" />
                </Button>
            ) : null}
        </div>
    )
}

function FlowTreeDirectoryRow({
    node,
    depth,
    selectedFlow,
    onSelectFlow,
    onDeleteFlow,
    renderFlowIndicator,
}: FlowTreeNodeRowProps & { node: Extract<FlowTreeNode, { kind: 'directory' }> }) {
    const [open, setOpen] = useState(true)
    const indent = 12 + depth * 14

    return (
        <Collapsible open={open} onOpenChange={setOpen} className="space-y-1">
            <CollapsibleTrigger asChild>
                <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    className="h-8 w-full justify-start gap-2 px-3 text-[11px] font-semibold tracking-[0.08em] text-muted-foreground"
                    style={{ paddingLeft: `${indent}px` }}
                    title={node.path}
                >
                    <ChevronRight
                        className={`h-3.5 w-3.5 shrink-0 transition-transform ${
                            open ? 'rotate-90' : ''
                        }`}
                    />
                    {open ? (
                        <FolderOpen className="h-3.5 w-3.5 shrink-0" />
                    ) : (
                        <Folder className="h-3.5 w-3.5 shrink-0" />
                    )}
                    <span className="truncate">{node.name}</span>
                </Button>
            </CollapsibleTrigger>
            <CollapsibleContent className="space-y-1">
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
            </CollapsibleContent>
        </Collapsible>
    )
}
