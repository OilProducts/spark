import type { CSSProperties, ReactNode } from 'react';

import {
    getShapeNodeDimensions,
    normalizeWorkflowNodeShape,
    type WorkflowNodeDimensions,
    type WorkflowNodeShape,
} from '@/lib/workflowNodeShape';

export type WorkflowNodeFramePalette = {
    fillColor: string
    strokeColor: string
    nestedStrokeColor: string
}

type WorkflowNodeSvgFrameProps = {
    viewBoxWidth: number
    viewBoxHeight: number
    children: ReactNode
    testId: string
}

const NODE_FILL = 'hsl(var(--background))';

// Air: hairline frames on the plain background, no glows; status tints only the stroke.
function statusPalette(token: string): WorkflowNodeFramePalette {
    return {
        fillColor: NODE_FILL,
        strokeColor: `hsl(var(--${token}) / 0.6)`,
        nestedStrokeColor: `hsl(var(--${token}) / 0.3)`,
    };
}

export function getWorkflowNodeFramePalette({
    status,
    selected,
    isWaiting,
}: {
    status: string
    selected: boolean
    isWaiting: boolean
}): WorkflowNodeFramePalette {
    if (selected) {
        return {
            fillColor: 'hsl(var(--accent))',
            strokeColor: 'hsl(var(--primary))',
            nestedStrokeColor: 'hsl(var(--primary) / 0.4)',
        };
    }
    if (isWaiting) return statusPalette('warning');
    if (status === 'running') return statusPalette('info');
    if (status === 'failed') return statusPalette('destructive');
    if (status === 'success') return statusPalette('success');
    return {
        fillColor: NODE_FILL,
        strokeColor: 'hsl(var(--muted-foreground) / 0.35)',
        nestedStrokeColor: 'hsl(var(--muted-foreground) / 0.2)',
    };
}

export function getWorkflowNodeContainerStyle(shape: WorkflowNodeShape): CSSProperties {
    const { width, height } = getShapeNodeDimensions(shape);
    return { width, height };
}

export function getWorkflowNodeContentClassName(shape: WorkflowNodeShape): string {
    switch (shape) {
        case 'Mdiamond':
        case 'Msquare':
            return 'px-7 py-5';
        case 'diamond':
            return 'px-9 py-6';
        case 'hexagon':
        case 'parallelogram':
            return 'px-8 py-6';
        case 'component':
            return 'pl-12 pr-6 py-6';
        case 'tripleoctagon':
            return 'px-8 py-6';
        case 'house':
            return 'px-8 pt-8 pb-5';
        case 'box':
        default:
            return 'px-4 py-4';
    }
}

export function getWorkflowNodeOverlayOffsetClassName(shape: WorkflowNodeShape): string {
    switch (shape) {
        case 'Mdiamond':
        case 'diamond':
            return 'top-3';
        case 'house':
            return 'top-3';
        default:
            return 'top-2';
    }
}

function SvgFrame({ viewBoxWidth, viewBoxHeight, children, testId }: WorkflowNodeSvgFrameProps) {
    return (
        <svg
            data-testid={testId}
            viewBox={`0 0 ${viewBoxWidth} ${viewBoxHeight}`}
            className="absolute inset-0 h-full w-full overflow-visible"
            aria-hidden
        >
            {children}
        </svg>
    );
}

function diamondPath(width: number, height: number, inset = 0) {
    return `M ${width / 2} ${inset} L ${width - inset} ${height / 2} L ${width / 2} ${height - inset} L ${inset} ${height / 2} Z`;
}

function hexagonPath(width: number, height: number, inset = 0) {
    const shoulder = Math.round(width * 0.18);
    return `M ${shoulder + inset} ${inset} H ${width - shoulder - inset} L ${width - inset} ${height / 2} L ${width - shoulder - inset} ${height - inset} H ${shoulder + inset} L ${inset} ${height / 2} Z`;
}

function parallelogramPath(width: number, height: number, inset = 0) {
    const slant = Math.round(width * 0.13);
    return `M ${slant + inset} ${inset} H ${width - inset} L ${width - slant - inset} ${height - inset} H ${inset} Z`;
}

function housePath(width: number, height: number, inset = 0) {
    const roofBaseY = Math.round(height * 0.3);
    return `M ${width / 2} ${inset} L ${width - inset} ${roofBaseY} V ${height - inset} H ${inset} V ${roofBaseY} Z`;
}

function octagonPath(width: number, height: number, inset = 0) {
    const edge = Math.round(Math.min(width, height) * 0.18);
    return `M ${edge + inset} ${inset} H ${width - edge - inset} L ${width - inset} ${edge + inset} V ${height - edge - inset} L ${width - edge - inset} ${height - inset} H ${edge + inset} L ${inset} ${height - edge - inset} V ${edge + inset} Z`;
}

function componentPath(width: number, height: number, inset = 0) {
    const tabWidth = Math.round(width * 0.11);
    const upperTabTop = Math.round(height * 0.16);
    const upperTabBottom = Math.round(height * 0.4);
    const lowerTabTop = Math.round(height * 0.56);
    const lowerTabBottom = Math.round(height * 0.8);
    return [
        `M ${tabWidth + inset} ${inset}`,
        `H ${width - inset}`,
        `V ${height - inset}`,
        `H ${tabWidth + inset}`,
        `V ${lowerTabBottom}`,
        `H ${inset}`,
        `V ${lowerTabTop}`,
        `H ${tabWidth + inset}`,
        `V ${upperTabBottom}`,
        `H ${inset}`,
        `V ${upperTabTop}`,
        `H ${tabWidth + inset}`,
        'Z',
    ].join(' ');
}

function BoxNodeFrame({ palette }: { palette: WorkflowNodeFramePalette }) {
    return (
        <div
            data-testid="workflow-node-frame-box"
            className={`absolute inset-0 rounded-lg border`}
            style={{
                backgroundColor: palette.fillColor,
                borderColor: palette.strokeColor,
            }}
            aria-hidden
        />
    );
}

function StartNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-Mdiamond"
        >
            <path d={diamondPath(dimensions.width, dimensions.height)} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
            <path d={diamondPath(dimensions.width - 20, dimensions.height - 20)} transform="translate(10 10)" fill="none" stroke={palette.nestedStrokeColor} strokeWidth={1} />
        </SvgFrame>
    );
}

function ExitNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-Msquare"
        >
            <rect x={1.5} y={1.5} width={dimensions.width - 3} height={dimensions.height - 3} rx={8} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
            <rect x={11} y={11} width={dimensions.width - 22} height={dimensions.height - 22} rx={6} fill="none" stroke={palette.nestedStrokeColor} strokeWidth={1} />
        </SvgFrame>
    );
}

function HumanGateNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-hexagon"
        >
            <path d={hexagonPath(dimensions.width, dimensions.height)} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
        </SvgFrame>
    );
}

function ConditionalNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-diamond"
        >
            <path d={diamondPath(dimensions.width, dimensions.height)} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
        </SvgFrame>
    );
}

function ParallelNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-component"
        >
            <path d={componentPath(dimensions.width, dimensions.height)} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
        </SvgFrame>
    );
}

function FanInNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-tripleoctagon"
        >
            <path d={octagonPath(dimensions.width, dimensions.height)} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
            <path d={octagonPath(dimensions.width - 18, dimensions.height - 18)} transform="translate(9 9)" fill="none" stroke={palette.nestedStrokeColor} strokeWidth={1} />
            <path d={octagonPath(dimensions.width - 34, dimensions.height - 34)} transform="translate(17 17)" fill="none" stroke={palette.nestedStrokeColor} strokeWidth={1} />
        </SvgFrame>
    );
}

function ToolNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-parallelogram"
        >
            <path d={parallelogramPath(dimensions.width, dimensions.height)} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
        </SvgFrame>
    );
}

function ManagerNodeFrame({ palette, dimensions }: { palette: WorkflowNodeFramePalette; dimensions: WorkflowNodeDimensions }) {
    return (
        <SvgFrame
            viewBoxWidth={dimensions.width}
            viewBoxHeight={dimensions.height}
            testId="workflow-node-frame-house"
        >
            <path d={housePath(dimensions.width, dimensions.height)} fill={palette.fillColor} stroke={palette.strokeColor} strokeWidth={1.25} />
        </SvgFrame>
    );
}

export function WorkflowNodeFrame({
    shape,
    palette,
}: {
    shape: WorkflowNodeShape
    palette: WorkflowNodeFramePalette
}) {
    const normalizedShape = normalizeWorkflowNodeShape(shape);
    const dimensions = getShapeNodeDimensions(normalizedShape);

    switch (normalizedShape) {
        case 'Mdiamond':
            return <StartNodeFrame palette={palette} dimensions={dimensions} />;
        case 'Msquare':
            return <ExitNodeFrame palette={palette} dimensions={dimensions} />;
        case 'hexagon':
            return <HumanGateNodeFrame palette={palette} dimensions={dimensions} />;
        case 'diamond':
            return <ConditionalNodeFrame palette={palette} dimensions={dimensions} />;
        case 'component':
            return <ParallelNodeFrame palette={palette} dimensions={dimensions} />;
        case 'tripleoctagon':
            return <FanInNodeFrame palette={palette} dimensions={dimensions} />;
        case 'parallelogram':
            return <ToolNodeFrame palette={palette} dimensions={dimensions} />;
        case 'house':
            return <ManagerNodeFrame palette={palette} dimensions={dimensions} />;
        case 'box':
        default:
            return <BoxNodeFrame palette={palette} />;
    }
}
