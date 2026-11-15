const EMPTY_GRAPH_ATTRS: Record<string, string> = {}
const EMPTY_DIAGNOSTICS: Record<string, import('@/store').DiagnosticEntry[]> = {}
import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Handle, Position, type Node, type NodeProps, useReactFlow } from '@xyflow/react'

import { saveFlowContent } from '@/lib/flowPersistence'
import {
    getReactFlowNodeTypeForShape,
    getShapeNodeStyle,
    normalizeWorkflowNodeShape,
    type WorkflowNodeShape,
} from '@/lib/workflowNodeShape'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'
import { buildCanonicalFlowModelFromEditorState, buildFlowDefinitionFromCanonicalFlowModel } from '@/lib/canonicalFlowModel'
import { useEditorGraphBridgeRef } from '@/features/editor/EditorGraphBridgeContext'
import { statusToneClassName } from '@/features/runs/model/shared'

import {
    WORKFLOW_NODE_KINDS,
    WorkflowNodeFrame,
    getWorkflowNodeContainerStyle,
    getWorkflowNodeContentClassName,
    getWorkflowNodeFramePalette,
} from './workflowNodeFrames'
import { useCanvasSessionMode, useCanvasRunId } from './canvasSessionContext'
import { getDerivedPreviewMeta } from './derivedPreview'

const FLOW_PORT_HANDLES = [
    {
        side: 'top',
        position: Position.Top,
        sourceId: 'source-top',
        targetId: 'target-top',
        sourceStyle: { left: '58%', top: 0, transform: 'translate(-50%, -50%)' },
        targetStyle: { left: '42%', top: 0, transform: 'translate(-50%, -50%)' },
    },
    {
        side: 'right',
        position: Position.Right,
        sourceId: 'source-right',
        targetId: 'target-right',
        sourceStyle: { left: '100%', top: '58%', transform: 'translate(50%, -50%)' },
        targetStyle: { left: '100%', top: '42%', transform: 'translate(50%, -50%)' },
    },
    {
        side: 'bottom',
        position: Position.Bottom,
        sourceId: 'source-bottom',
        targetId: 'target-bottom',
        sourceStyle: { left: '58%', top: '100%', transform: 'translate(-50%, 50%)' },
        targetStyle: { left: '42%', top: '100%', transform: 'translate(-50%, 50%)' },
    },
    {
        side: 'left',
        position: Position.Left,
        sourceId: 'source-left',
        targetId: 'target-left',
        sourceStyle: { left: 0, top: '58%', transform: 'translate(-50%, -50%)' },
        targetStyle: { left: 0, top: '42%', transform: 'translate(-50%, -50%)' },
    },
] as const

const NODE_STATUS_LABELS: Record<string, string> = {
    running: 'Running',
    success: 'Succeeded',
    failed: 'Failed',
    waiting: 'Needs input',
}

type BaseWorkflowNodeProps = NodeProps & {
    defaultShape: WorkflowNodeShape
}

function BaseWorkflowNode({ id, data, selected, defaultShape }: BaseWorkflowNodeProps) {
    const canvasMode = useCanvasSessionMode()
    const runId = useCanvasRunId()
    const isEditorCanvas = canvasMode === 'editor'
    const isRunCanvas = canvasMode === 'runs'
    const flowName = useStore((state) => (isEditorCanvas ? state.activeFlow : null))
    const isExpandedChildPreview = useStore((state) => (
        isEditorCanvas && state.activeFlow
            ? (state.editorExpandChildFlowsByFlow[state.activeFlow] ?? state.preferredExpandChildFlows)
            : false
    ))
    const isFilesMode = useStore((state) => isEditorCanvas && state.editorMode === 'files')
    const executionHumanGate = useStore((state) => (runId ? state.runDetailSessionsByRunId[runId]?.humanGate ?? null : null))
    const flowMetadata = useStore((state) => (
        isEditorCanvas ? state.flowMetadata : (runId ? state.runDetailSessionsByRunId[runId]?.graphAttrs ?? EMPTY_GRAPH_ATTRS : EMPTY_GRAPH_ATTRS)
    ))
    const nodeDiagnostics = useStore((state) => (
        isEditorCanvas ? state.nodeDiagnostics : (runId ? state.runDetailSessionsByRunId[runId]?.nodeDiagnostics ?? EMPTY_DIAGNOSTICS : EMPTY_DIAGNOSTICS)
    ))
    const editorGraphBridgeRef = useEditorGraphBridgeRef()
    const { setNodes, getEdges } = useReactFlow()
    const readEdges = () => editorGraphBridgeRef?.current?.getEdges() ?? getEdges()
    const updateNodes = (updater: Parameters<typeof setNodes>[0]) =>
        (editorGraphBridgeRef?.current?.setNodes ?? setNodes)(updater)
    const inputRef = useRef<HTMLInputElement>(null)
    const humanGate = isRunCanvas ? executionHumanGate : null

    const displayLabel = (typeof data.label === 'string' && data.label.trim()) || id
    const [isEditingLabel, setIsEditingLabel] = useState(false)
    const [draftLabel, setDraftLabel] = useState(displayLabel)

    const status = (data.status as string) || 'idle'
    const derivedPreviewMeta = getDerivedPreviewMeta(data)
    const isReadOnlyPreviewNode = derivedPreviewMeta?.kind === 'child-cluster' || derivedPreviewMeta?.kind === 'child-node'
    // In Files mode the file view owns edits; the canvas only selects nodes.
    const isReadOnlyEditorCanvas = isEditorCanvas && (isExpandedChildPreview || isFilesMode)
    const renderedShape = normalizeWorkflowNodeShape((data.shape as string) || defaultShape)
    const diagnosticsForNode = nodeDiagnostics[id] || []
    const diagnosticsCount = diagnosticsForNode.length
    const hasDiagnosticError = diagnosticsForNode.some((diag) => diag.severity === 'error')
    const hasDiagnosticWarning = diagnosticsForNode.some((diag) => diag.severity === 'warning')
    const isWaiting = isRunCanvas && (humanGate?.nodeId === id || status === 'waiting')
    const framePalette = getWorkflowNodeFramePalette({ status, selected, isWaiting })
    const nodeKind = WORKFLOW_NODE_KINDS[renderedShape]
    // A child flow's title box is captioned as the child flow, not by its box shape.
    const showKind = derivedPreviewMeta?.kind !== 'child-cluster'
    const kindCaption = !showKind ? 'Child flow' : nodeKind.caption.toLowerCase() !== displayLabel.toLowerCase() ? nodeKind.caption : null
    const displayStatus = isWaiting ? 'waiting' : status
    const containerStyle = getWorkflowNodeContainerStyle(renderedShape)
    const handleClassName = cn(
        // Above the content layer, so a drag from anywhere on a handle wires a route instead of moving the node.
        'z-20 h-2.5 w-2.5 transition-opacity',
        isEditorCanvas && !isReadOnlyEditorCanvas
            ? 'border-border/70 bg-background opacity-0 group-hover:opacity-100'
            : 'pointer-events-none border-transparent bg-transparent opacity-0',
        selected && isEditorCanvas && !isReadOnlyEditorCanvas ? 'opacity-100' : null,
    )

    useEffect(() => {
        if (isEditingLabel) {
            inputRef.current?.focus()
            inputRef.current?.select()
        }
    }, [isEditingLabel])


    const persistNodeData = (nextData: Record<string, unknown>) => {
        if (!isEditorCanvas || !flowName || isReadOnlyEditorCanvas) {
            return
        }

        const bridge = editorGraphBridgeRef?.current
        bridge?.recordFieldEdit(null, { nodes: bridge.getNodes().filter((node) => node.id === id) })
        let updatedNodes: Node[] = []
        updateNodes((currentNodes) => {
            updatedNodes = currentNodes.map((node) => {
                if (node.id !== id) {
                    return node
                }
                const mergedData = { ...node.data }
                Object.entries(nextData).forEach(([key, value]) => {
                    if (value === undefined) {
                        delete mergedData[key]
                        return
                    }
                    mergedData[key] = value
                })
                const nextShape = normalizeWorkflowNodeShape((mergedData.shape as string) || defaultShape)
                return {
                    ...node,
                    type: getReactFlowNodeTypeForShape(nextShape),
                    style: getShapeNodeStyle(nextShape),
                    data: mergedData,
                }
            })
            return updatedNodes
        })

        if (updatedNodes.length > 0) {
            const flow = buildFlowDefinitionFromCanonicalFlowModel(flowName, buildCanonicalFlowModelFromEditorState(flowName, {
                nodes: updatedNodes,
                edges: readEdges(),
                flowMetadata,
                flow: editorGraphBridgeRef?.current?.getFlow(),
            }))
            void saveFlowContent(flowName, { flow })
        }
    }

    const startEditLabel = (event: React.MouseEvent<HTMLDivElement>) => {
        if (!isEditorCanvas || isReadOnlyEditorCanvas) {
            return
        }
        event.stopPropagation()
        setDraftLabel(displayLabel)
        setIsEditingLabel(true)
    }

    const commitLabel = () => {
        const nextLabel = draftLabel.trim() || id
        setIsEditingLabel(false)
        if (nextLabel !== displayLabel) {
            persistNodeData({ label: nextLabel })
        }
    }

    const cancelLabel = () => {
        setDraftLabel(displayLabel)
        setIsEditingLabel(false)
    }

    const onLabelKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
        if (event.key === 'Enter') {
            event.preventDefault()
            commitLabel()
        } else if (event.key === 'Escape') {
            event.preventDefault()
            cancelLabel()
        }
    }

    // A step's settings are in the inspector, which a selected step opens: Edit moves there.
    const focusInspector = (event: React.MouseEvent<HTMLButtonElement>) => {
        event.stopPropagation()
        document.querySelector<HTMLElement>('[data-testid="inspector-panel"] h2')?.focus()
    }

    return (
        <div
            data-testid={`workflow-node-${renderedShape}`}
            data-workflow-shape={renderedShape}
            data-node-id={id}
            data-node-status={isWaiting ? 'waiting' : status}
            data-node-selected={selected ? 'true' : 'false'}
            className={cn(
                'group relative select-none',
                isReadOnlyPreviewNode ? 'opacity-90' : null,
            )}
            style={containerStyle}
        >
            <WorkflowNodeFrame shape={renderedShape} palette={framePalette} />

            {FLOW_PORT_HANDLES.map((handle) => (
                <Handle
                    key={handle.targetId}
                    id={handle.targetId}
                    type="target"
                    position={handle.position}
                    className={handleClassName}
                    style={handle.targetStyle}
                    aria-label={`${handle.side} target port`}
                />
            ))}

            {/* Above the node, so badges never collide with a non-rectangular frame. */}
            <div className="absolute bottom-full right-0 z-20 mb-1 flex items-center gap-1">
                {selected && isEditorCanvas && !isReadOnlyEditorCanvas && (
                    <button
                        type="button"
                        onClick={focusInspector}
                        aria-label={`Edit ${displayLabel} in step settings`}
                        className="rounded border border-border bg-background px-2 py-0.5 text-xs font-medium text-muted-foreground hover:text-foreground"
                    >
                        Edit
                    </button>
                )}
                {diagnosticsCount > 0 && (
                    <div
                        data-testid="node-diagnostic-badge"
                        className={cn(
                            'rounded-full border border-border bg-background px-2 py-0.5 text-xs font-medium',
                            hasDiagnosticError
                                ? 'text-destructive'
                                : hasDiagnosticWarning
                                    ? 'text-warning'
                                    : 'text-info',
                        )}
                        title={diagnosticsForNode.map((diag) => diag.message).join('\n')}
                    >
                        {diagnosticsCount} {hasDiagnosticError ? 'error' : hasDiagnosticWarning ? 'warning' : 'note'}{diagnosticsCount === 1 ? '' : 's'}
                    </div>
                )}
            </div>

            <div
                onDoubleClick={startEditLabel}
                className={cn(
                    'absolute inset-0 z-10 flex items-center justify-center text-center',
                    getWorkflowNodeContentClassName(renderedShape),
                )}
            >
                <div className="flex w-full min-w-0 flex-col items-center justify-center gap-1">
                    {/* One caption line: the kind (a start node labelled Start needs no second Start),
                        or in a run its status, which the shape and icon already place, so the caption fits
                        narrow shapes and status costs the frame no extra height. Near a diamond's top
                        there is room for the status word alone. */}
                    {showKind || kindCaption || displayStatus !== 'idle' ? (
                        <span className="canvas-node-caption inline-flex max-w-full items-center gap-1 whitespace-nowrap text-muted-foreground">
                            {showKind && (displayStatus === 'idle' || (renderedShape !== 'diamond' && renderedShape !== 'Mdiamond'))
                                ? <nodeKind.Icon className="size-[1.15em] shrink-0" aria-hidden />
                                : null}
                            {displayStatus === 'idle' ? kindCaption : (
                                <>
                                    {kindCaption ? <span className="sr-only">{kindCaption}</span> : null}
                                    <span className={cn('font-medium', statusToneClassName(displayStatus))}>
                                        {NODE_STATUS_LABELS[displayStatus] ?? displayStatus}
                                    </span>
                                </>
                            )}
                        </span>
                    ) : null}
                    {isEditingLabel ? (
                        <input
                            ref={inputRef}
                            aria-label="Step name"
                            value={draftLabel}
                            onChange={(event) => setDraftLabel(event.target.value)}
                            onBlur={commitLabel}
                            onKeyDown={onLabelKeyDown}
                            onPointerDown={(event) => event.stopPropagation()}
                            className="nodrag nopan h-7 w-full max-w-[152px] rounded border border-input bg-background px-2 text-center text-sm font-semibold outline-none ring-0 focus-visible:ring-1 focus-visible:ring-ring"
                        />
                    ) : (
                        <span className={cn(
                            'canvas-node-label line-clamp-2 font-semibold',
                            isReadOnlyPreviewNode ? 'text-muted-foreground' : 'text-foreground',
                        )}
                        >
                            {displayLabel}
                        </span>
                    )}
                </div>
            </div>

            {FLOW_PORT_HANDLES.map((handle) => (
                <Handle
                    key={handle.sourceId}
                    id={handle.sourceId}
                    type="source"
                    position={handle.position}
                    className={handleClassName}
                    style={handle.sourceStyle}
                    aria-label={`${handle.side} source port`}
                />
            ))}

        </div>
    )
}

export function TaskNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="box" />
}

export function StartNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="Mdiamond" />
}

export function ExitNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="Msquare" />
}

export function HumanGateNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="hexagon" />
}

export function SwitchNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="diamond" />
}

export function ParallelNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="component" />
}

export function FanInNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="tripleoctagon" />
}

export function ToolNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="parallelogram" />
}

export function ManagerNode(props: NodeProps) {
    return <BaseWorkflowNode {...props} defaultShape="house" />
}
