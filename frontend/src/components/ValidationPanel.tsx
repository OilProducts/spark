import { useState } from 'react';
import { useReactFlow } from '@xyflow/react';
import { useStore } from '@/store';

const severityStyles: Record<string, string> = {
    error: 'bg-destructive/15 text-destructive',
    warning: 'bg-amber-500/15 text-amber-700',
    info: 'bg-sky-500/15 text-sky-700',
};

type SeverityFilter = 'all' | 'error' | 'warning' | 'info';
type SortMode = 'severity' | 'line' | 'rule';

const severityRank: Record<string, number> = {
    error: 0,
    warning: 1,
    info: 2,
};

const filterTestIds: Record<SeverityFilter, string> = {
    all: 'validation-filter-all',
    error: 'validation-filter-error',
    warning: 'validation-filter-warning',
    info: 'validation-filter-info',
};

export function ValidationPanel() {
    const diagnostics = useStore((state) => state.diagnostics);
    const viewMode = useStore((state) => state.viewMode);
    const hasValidationErrors = useStore((state) => state.hasValidationErrors);
    const setSelectedNodeId = useStore((state) => state.setSelectedNodeId);
    const setSelectedEdgeId = useStore((state) => state.setSelectedEdgeId);
    const { getNode, getEdges, setCenter, setNodes, setEdges } = useReactFlow();
    const [severityFilter, setSeverityFilter] = useState<SeverityFilter>('all');
    const [sortMode, setSortMode] = useState<SortMode>('severity');

    if (viewMode !== 'editor' || diagnostics.length === 0) {
        return null;
    }

    const filteredDiagnostics = diagnostics.filter((diag) =>
        severityFilter === 'all' ? true : diag.severity === severityFilter,
    );

    const sortedDiagnostics = [...filteredDiagnostics].sort((left, right) => {
        if (sortMode === 'line') {
            const leftLine = left.line ?? Number.MAX_SAFE_INTEGER;
            const rightLine = right.line ?? Number.MAX_SAFE_INTEGER;
            if (leftLine !== rightLine) return leftLine - rightLine;
        }
        if (sortMode === 'rule') {
            const byRule = left.rule_id.localeCompare(right.rule_id);
            if (byRule !== 0) return byRule;
        }
        const leftRank = severityRank[left.severity] ?? Number.MAX_SAFE_INTEGER;
        const rightRank = severityRank[right.severity] ?? Number.MAX_SAFE_INTEGER;
        if (leftRank !== rightRank) return leftRank - rightRank;
        if ((left.line ?? Number.MAX_SAFE_INTEGER) !== (right.line ?? Number.MAX_SAFE_INTEGER)) {
            return (left.line ?? Number.MAX_SAFE_INTEGER) - (right.line ?? Number.MAX_SAFE_INTEGER);
        }
        return left.message.localeCompare(right.message);
    });

    const centerOnNode = (nodeId: string) => {
        const node = getNode(nodeId);
        if (!node) return;
        const position =
            (node as typeof node & { positionAbsolute?: { x: number; y: number } }).positionAbsolute ??
            node.position;
        const width = node.width ?? 0;
        const height = node.height ?? 0;
        setCenter(position.x + width / 2, position.y + height / 2, { zoom: 1.1, duration: 200 });
    };

    const centerOnEdge = (source: string, target: string) => {
        const sourceNode = getNode(source);
        const targetNode = getNode(target);
        if (!sourceNode || !targetNode) return;
        const sourcePos =
            (sourceNode as typeof sourceNode & { positionAbsolute?: { x: number; y: number } }).positionAbsolute ??
            sourceNode.position;
        const targetPos =
            (targetNode as typeof targetNode & { positionAbsolute?: { x: number; y: number } }).positionAbsolute ??
            targetNode.position;
        const sourceCenterX = sourcePos.x + (sourceNode.width ?? 0) / 2;
        const sourceCenterY = sourcePos.y + (sourceNode.height ?? 0) / 2;
        const targetCenterX = targetPos.x + (targetNode.width ?? 0) / 2;
        const targetCenterY = targetPos.y + (targetNode.height ?? 0) / 2;
        setCenter((sourceCenterX + targetCenterX) / 2, (sourceCenterY + targetCenterY) / 2, {
            zoom: 1.05,
            duration: 200,
        });
    };

    const focusCanvasEntity = (selector: string) => {
        const target = document.querySelector<HTMLElement>(selector);
        if (!target) return;
        target.tabIndex = -1;
        target.focus({ preventScroll: true });
    };

    const selectNode = (nodeId: string) => {
        setSelectedNodeId(nodeId);
        setSelectedEdgeId(null);
        setNodes((nodes) => nodes.map((node) => ({ ...node, selected: node.id === nodeId })));
        setEdges((edges) => edges.map((edge) => ({ ...edge, selected: false })));
        focusCanvasEntity(`.react-flow__node[data-id="${nodeId}"]`);
        centerOnNode(nodeId);
    };

    const selectEdge = (source: string, target: string) => {
        const edge = getEdges().find((edge) => edge.source === source && edge.target === target);
        if (!edge) return;
        setSelectedNodeId(null);
        setSelectedEdgeId(edge.id);
        setEdges((edges) => edges.map((edgeItem) => ({ ...edgeItem, selected: edgeItem.id === edge.id })));
        setNodes((nodes) => nodes.map((node) => ({ ...node, selected: false })));
        focusCanvasEntity(`.react-flow__edge[data-id="${edge.id}"]`);
        centerOnEdge(source, target);
    };

    return (
        <div
            data-testid="validation-panel"
            className="absolute left-4 bottom-4 z-20 w-80 rounded-md border border-border bg-card/95 p-3 shadow-lg"
        >
            <div className="flex items-center justify-between">
                <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Validation</div>
                <div className="text-[11px] font-medium text-muted-foreground">
                    {hasValidationErrors ? 'Errors present' : 'Warnings only'}
                </div>
            </div>
            <div className="mt-2 flex items-center justify-between gap-2">
                <select
                    data-testid="validation-sort-select"
                    value={sortMode}
                    onChange={(event) => setSortMode(event.target.value as SortMode)}
                    className="h-7 rounded border border-border bg-background px-2 text-[11px]"
                >
                    <option value="severity">Sort: Severity</option>
                    <option value="line">Sort: Line</option>
                    <option value="rule">Sort: Rule</option>
                </select>
                <div data-testid="validation-visible-count" className="text-[11px] text-muted-foreground">
                    {sortedDiagnostics.length} visible
                </div>
            </div>
            <div className="mt-2 grid grid-cols-4 gap-1">
                {(['all', 'error', 'warning', 'info'] as SeverityFilter[]).map((filterValue) => (
                    <button
                        key={filterValue}
                        data-testid={filterTestIds[filterValue]}
                        onClick={() => setSeverityFilter(filterValue)}
                        className={`rounded border px-1.5 py-1 text-[10px] font-medium uppercase tracking-wide ${
                            severityFilter === filterValue
                                ? 'border-foreground/50 bg-muted text-foreground'
                                : 'border-border/60 bg-background text-muted-foreground hover:bg-muted/50'
                        }`}
                    >
                        {filterValue}
                    </button>
                ))}
            </div>
            <div className="mt-2 max-h-44 space-y-2 overflow-y-auto pr-1">
                {sortedDiagnostics.map((diag, index) => (
                    <button
                        key={`${diag.rule_id}-${index}`}
                        data-testid="validation-diagnostic-item"
                        onClick={() => {
                            if (diag.node_id) {
                                selectNode(diag.node_id);
                            } else if (diag.edge && diag.edge.length === 2) {
                                selectEdge(diag.edge[0], diag.edge[1]);
                            }
                        }}
                        className="w-full rounded-md border border-border/60 bg-background/85 px-2 py-1 text-left text-xs transition-colors hover:bg-muted"
                    >
                        <div className="flex items-start gap-2">
                            <span
                                className={`mt-0.5 rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                                    severityStyles[diag.severity] || 'bg-muted text-muted-foreground'
                                }`}
                            >
                                {diag.severity}
                            </span>
                            <div className="flex-1">
                                <div className="text-foreground">{diag.message}</div>
                                <div className="text-[10px] text-muted-foreground">
                                    {diag.rule_id}
                                    {diag.line ? ` • line ${diag.line}` : ''}
                                </div>
                            </div>
                        </div>
                    </button>
                ))}
                {sortedDiagnostics.length === 0 && (
                    <div className="rounded-md border border-dashed border-border/60 bg-background/80 px-2 py-2 text-xs text-muted-foreground">
                        No diagnostics for current filter.
                    </div>
                )}
            </div>
        </div>
    );
}
