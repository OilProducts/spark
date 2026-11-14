import { ChevronRight, FileText, Folder, FolderOpen, Trash2 } from 'lucide-react'
import type { MouseEvent, ReactNode } from 'react'
import { useMemo, useState } from 'react'

import { ListGroupLabel } from '@/components/app/side-panel-row'
import { Button } from '@/components/ui/button'
import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from '@/components/ui/collapsible'
import { buildFlowTree, type FlowTreeNode } from '@/lib/flowPaths'

/** A flow folder as people read it: "math-research" is "Math research", as flow pickers group it. */
export const folderTitle = (folder: string) => folder.charAt(0).toUpperCase() + folder.slice(1).replace(/[-_]+/g, ' ')

/** What a flow is called and does, shown in place of its file name. */
export interface FlowLabel {
    title: string
    description: string
    /** Why the flow doesn't load; the row says it won't open. */
    error?: string
}

interface FlowTreeProps {
    flows: string[]
    /** Titles and descriptions by flow name; a flow without one shows its file name. */
    flowLabels?: Record<string, FlowLabel>
    /** The project the list is for, naming its own flows' heading. */
    projectLabel?: string
    /** The project's own flows (its new and customized ones): when any are listed, they come first under
     *  "This project", and the rest under "Bundled". */
    projectFlows?: ReadonlySet<string>
    selectedFlow: string | null
    onSelectFlow: (flowName: string) => void
    onDeleteFlow?: (event: MouseEvent, flowName: string) => void
    /** Which flows offer Delete; all do without it. */
    canDeleteFlow?: (flowName: string) => boolean
    renderFlowIndicator?: (flowName: string) => ReactNode
    dataTestId?: string
}

export function FlowTree({
    flows,
    flowLabels,
    projectLabel,
    projectFlows,
    selectedFlow,
    onSelectFlow,
    onDeleteFlow,
    canDeleteFlow,
    renderFlowIndicator,
    dataTestId,
}: FlowTreeProps) {
    const sections = useMemo(() => {
        const own = flows.filter((flow) => projectFlows?.has(flow))
        if (own.length === 0) {
            return [{ label: null, tree: buildFlowTree(flows) }]
        }
        return [
            { label: projectLabel ? `${projectLabel}'s flows` : 'This project', tree: buildFlowTree(own) },
            { label: 'Bundled with Spark', tree: buildFlowTree(flows.filter((flow) => !projectFlows?.has(flow))) },
        ].filter((section) => section.tree.length > 0)
    }, [flows, projectFlows, projectLabel])

    return (
        <div data-testid={dataTestId} className="space-y-4">
            {sections.map(({ label, tree }) => (
                <section key={label ?? 'flows'} aria-label={label ?? undefined} className="space-y-1">
                    {label ? <ListGroupLabel><h3>{label}</h3></ListGroupLabel> : null}
                    {tree.map((node) => (
                        <FlowTreeNodeRow
                            key={node.path}
                            node={node}
                            depth={0}
                            flowLabels={flowLabels}
                            selectedFlow={selectedFlow}
                            onSelectFlow={onSelectFlow}
                            onDeleteFlow={onDeleteFlow}
                            canDeleteFlow={canDeleteFlow}
                            renderFlowIndicator={renderFlowIndicator}
                        />
                    ))}
                </section>
            ))}
        </div>
    )
}

interface FlowTreeNodeRowProps {
    node: FlowTreeNode
    flowLabels?: Record<string, FlowLabel>
    depth: number
    selectedFlow: string | null
    onSelectFlow: (flowName: string) => void
    onDeleteFlow?: (event: MouseEvent, flowName: string) => void
    canDeleteFlow?: (flowName: string) => boolean
    renderFlowIndicator?: (flowName: string) => ReactNode
}

function FlowTreeNodeRow({
    node,
    depth,
    flowLabels,
    selectedFlow,
    onSelectFlow,
    onDeleteFlow,
    canDeleteFlow,
    renderFlowIndicator,
}: FlowTreeNodeRowProps) {
    const indent = 12 + depth * 14

    if (node.kind === 'directory') {
        return (
            <FlowTreeDirectoryRow
                node={node}
                depth={depth}
                flowLabels={flowLabels}
                selectedFlow={selectedFlow}
                onSelectFlow={onSelectFlow}
                onDeleteFlow={onDeleteFlow}
                canDeleteFlow={canDeleteFlow}
                renderFlowIndicator={renderFlowIndicator}
            />
        )
    }

    const label = flowLabels?.[node.path]
    const title = label?.title.trim() || null
    const description = label?.description.trim() || null
    const error = label?.error || null
    return (
        <div className="group relative">
            <Button
                type="button"
                aria-label={`${title ? `${title} (${node.path})` : node.path}${error ? ', has errors' : ''}`}
                // The row truncates its title and description; the tooltip has them in full.
                title={[title, node.path, error ?? description].filter(Boolean).join('\n')}
                aria-current={selectedFlow === node.path ? 'true' : undefined}
                data-flow-name={node.path}
                onClick={() => onSelectFlow(node.path)}
                variant="ghost"
                className={`h-auto min-h-9 w-full justify-start rounded-md px-3 py-2 pr-8 text-left text-sm transition-colors ${
                    selectedFlow === node.path
                        ? 'font-medium rounded-none text-primary shadow-[inset_2px_0_0_hsl(var(--primary))]'
                        : 'text-muted-foreground hover:text-primary'
                }`}
                style={{ paddingLeft: `${indent}px` }}
            >
                <span className="flex w-full min-w-0 items-start gap-2">
                    <FileText className="mt-0.5 h-3.5 w-3.5 shrink-0" />
                    {/* The indicator wraps under the name only when both don't fit, so the name keeps the row's width. */}
                    <span className="flex min-w-0 flex-1 flex-col gap-0.5">
                        <span className="flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1">
                            <span className="min-w-0 max-w-full truncate">{title ?? node.name}</span>
                            {renderFlowIndicator?.(node.path)}
                        </span>
                        {error ? (
                            <span data-testid="flow-tree-error" className="block w-full truncate text-xs font-normal text-destructive">Has errors · won’t open until fixed</span>
                        ) : description ? <span data-testid="flow-tree-description" className="block w-full truncate text-xs font-normal text-muted-foreground">{description}</span> : null}
                    </span>
                </span>
            </Button>
            {onDeleteFlow && (canDeleteFlow?.(node.path) ?? true) ? (
                <Button
                    type="button"
                    aria-label={`Delete ${title ?? node.path}`}
                    onClick={(event) => {
                        event.stopPropagation()
                        onDeleteFlow(event, node.path)
                    }}
                    variant="ghost"
                    size="icon-xs"
                    className="absolute right-2 top-1/2 -translate-y-1/2 text-muted-foreground opacity-0 transition-all group-hover:opacity-100 focus-visible:opacity-100 hover:text-destructive"
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
    flowLabels,
    selectedFlow,
    onSelectFlow,
    onDeleteFlow,
    canDeleteFlow,
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
                    className="h-8 w-full justify-start gap-2 px-3 text-sm font-medium text-foreground"
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
                    <span className="truncate">{folderTitle(node.name)}</span>
                </Button>
            </CollapsibleTrigger>
            <CollapsibleContent className="space-y-1">
                {node.children.map((child) => (
                    <FlowTreeNodeRow
                        key={child.path}
                        node={child}
                        depth={depth + 1}
                        flowLabels={flowLabels}
                        selectedFlow={selectedFlow}
                        onSelectFlow={onSelectFlow}
                        onDeleteFlow={onDeleteFlow}
                        canDeleteFlow={canDeleteFlow}
                        renderFlowIndicator={renderFlowIndicator}
                    />
                ))}
            </CollapsibleContent>
        </Collapsible>
    )
}
