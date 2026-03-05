import type { CSSProperties } from 'react';
import { BaseEdge, EdgeLabelRenderer, getBezierPath, type EdgeProps } from '@xyflow/react';
import { useStore } from '@/store';

const WARNING_STROKE = 'hsl(38 92% 50%)';

export function ValidationEdge({
    source,
    target,
    markerEnd,
    style,
    selected,
    sourceX,
    sourceY,
    targetX,
    targetY,
    sourcePosition,
    targetPosition,
}: EdgeProps) {
    const edgeDiagnostics = useStore((state) => state.edgeDiagnostics);
    const diagnosticsForEdge = edgeDiagnostics[`${source}->${target}`] || [];
    const hasError = diagnosticsForEdge.some((diag) => diag.severity === 'error');
    const hasWarning = diagnosticsForEdge.some((diag) => diag.severity === 'warning');
    const hasInfo = diagnosticsForEdge.some((diag) => diag.severity === 'info');

    const [edgePath, labelX, labelY] = getBezierPath({
        sourceX,
        sourceY,
        targetX,
        targetY,
        sourcePosition,
        targetPosition,
    });

    const edgeStyle: CSSProperties = { ...style };
    if (hasError) {
        edgeStyle.stroke = 'hsl(var(--destructive))';
        edgeStyle.strokeWidth = selected ? 4 : 3;
        edgeStyle.opacity = 1;
    } else if (hasWarning) {
        edgeStyle.stroke = WARNING_STROKE;
        edgeStyle.strokeWidth = selected ? 4 : 3;
        edgeStyle.opacity = 0.95;
    } else if (selected) {
        edgeStyle.stroke = 'hsl(var(--primary))';
        edgeStyle.strokeWidth = 4;
        edgeStyle.opacity = 1;
    }

    if (selected) {
        const shadowColor = hasError
            ? 'hsl(var(--destructive) / 0.55)'
            : hasWarning
                ? 'hsl(38 92% 50% / 0.4)'
                : 'hsl(var(--primary) / 0.6)';
        edgeStyle.filter = `drop-shadow(0 0 7px ${shadowColor})`;
    }

    const badgeClass = hasError
        ? 'bg-destructive/15 text-destructive'
        : hasWarning
            ? 'bg-amber-500/15 text-amber-800'
            : 'bg-sky-500/15 text-sky-700';

    return (
        <>
            <BaseEdge path={edgePath} markerEnd={markerEnd} style={edgeStyle} interactionWidth={16} />
            {diagnosticsForEdge.length > 0 && (
                <EdgeLabelRenderer>
                    <div
                        className="nodrag nopan"
                        style={{
                            transform: `translate(-50%, -50%) translate(${labelX}px, ${labelY}px)`,
                        }}
                    >
                        <div
                            data-testid="edge-diagnostic-badge"
                            className={`rounded-full border border-border bg-background/95 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${badgeClass}`}
                            title={diagnosticsForEdge.map((diag) => diag.message).join('\n')}
                        >
                            {diagnosticsForEdge.length} {hasError ? 'Error' : hasWarning ? 'Warn' : hasInfo ? 'Info' : 'Issue'}
                        </div>
                    </div>
                </EdgeLabelRenderer>
            )}
        </>
    );
}
