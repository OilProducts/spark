import { useEffect, useRef, useState, type KeyboardEvent } from 'react'
import { Handle, NodeToolbar, Position, type Node, type NodeProps, useReactFlow } from '@xyflow/react'

import { saveFlowContent } from '@/lib/flowPersistence'
import { getToolHookCommandWarning } from '@/lib/graphAttrValidation'
import { fetchLlmProfiles } from '@/lib/api/llmProfilesApi'
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection, type LlmProfileMetadata } from '@/lib/llmSuggestions'
import { getHandlerType, getNodeFieldVisibility } from '@/lib/nodeVisibility'
import {
    WORKFLOW_NODE_SHAPE_OPTIONS,
    getReactFlowNodeTypeForShape,
    getShapeNodeStyle,
    getShapeTypeMismatchWarning,
    normalizeWorkflowNodeShape,
    type WorkflowNodeShape,
} from '@/lib/workflowNodeShape'
import { cn } from '@/lib/utils'
import { useStore } from '@/store'
import { generateDot } from '@/lib/dotUtils'
import { useEditorGraphBridgeRef } from '@/features/editor/EditorGraphBridgeContext'

import {
    WorkflowNodeFrame,
    getWorkflowNodeContainerStyle,
    getWorkflowNodeContentClassName,
    getWorkflowNodeFramePalette,
    getWorkflowNodeOverlayOffsetClassName,
} from './workflowNodeFrames'
import { useCanvasSessionMode } from './canvasSessionContext'
import { getDerivedPreviewMeta } from './derivedPreview'

const BUILTIN_HANDLER_OPTIONS = [
    'start',
    'exit',
    'codergen',
    'wait.human',
    'conditional',
    'parallel',
    'parallel.fan_in',
    'tool',
    'stack.manager_loop',
] as const

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

type BaseWorkflowNodeProps = NodeProps & {
    defaultShape: WorkflowNodeShape
}

function getStatusBadgeClassName(status: string) {
    if (status === 'success') {
        return 'bg-green-500/20 text-green-600'
    }
    if (status === 'running') {
        return 'bg-primary/20 text-primary'
    }
    if (status === 'failed') {
        return 'bg-destructive/20 text-destructive'
    }
    return 'bg-muted/50 text-muted-foreground'
}

function BaseWorkflowNode({ id, data, selected, defaultShape }: BaseWorkflowNodeProps) {
    const canvasMode = useCanvasSessionMode()
    const isEditorCanvas = canvasMode === 'editor'
    const isRunCanvas = canvasMode === 'execution' || canvasMode === 'runs'
    const flowName = useStore((state) => (isEditorCanvas ? state.activeFlow : state.executionFlow))
    const isExpandedChildPreview = useStore((state) => (
        isEditorCanvas && state.activeFlow
            ? (state.editorExpandChildFlowsByFlow[state.activeFlow] ?? false)
            : false
    ))
    const executionHumanGate = useStore((state) => state.humanGate)
    const graphAttrs = useStore((state) => {
        if (isEditorCanvas) {
            return state.graphAttrs
        }
        return canvasMode === 'runs' ? state.runGraphAttrs : state.executionGraphAttrs
    })
    const nodeDiagnostics = useStore((state) => {
        if (isEditorCanvas) {
            return state.nodeDiagnostics
        }
        return canvasMode === 'runs' ? state.runNodeDiagnostics : state.executionNodeDiagnostics
    })
    const editorGraphBridgeRef = useEditorGraphBridgeRef()
    const { setNodes, getEdges } = useReactFlow()
    const readEdges = () => editorGraphBridgeRef?.current?.getEdges() ?? getEdges()
    const updateNodes = (updater: Parameters<typeof setNodes>[0]) =>
        (editorGraphBridgeRef?.current?.setNodes ?? setNodes)(updater)
    const inputRef = useRef<HTMLInputElement>(null)
    const humanGate = isRunCanvas ? executionHumanGate : null

    const currentShape = normalizeWorkflowNodeShape((data.shape as string) || defaultShape)
    const displayLabel = (data.label as string) || 'Task Node'
    const [isEditingLabel, setIsEditingLabel] = useState(false)
    const [draftLabel, setDraftLabel] = useState(displayLabel)
    const [isEditingDetails, setIsEditingDetails] = useState(false)
    const [draftShape, setDraftShape] = useState<string>(currentShape)
    const [draftPrompt, setDraftPrompt] = useState<string>((data.prompt as string) || '')
    const [draftToolCommand, setDraftToolCommand] = useState<string>((data['tool.command'] as string) || '')
    const [draftToolHooksPre, setDraftToolHooksPre] = useState<string>((data['tool.hooks.pre'] as string) || '')
    const [draftToolHooksPost, setDraftToolHooksPost] = useState<string>((data['tool.hooks.post'] as string) || '')
    const [draftToolArtifactsPaths, setDraftToolArtifactsPaths] = useState<string>(
        (data['tool.artifacts.paths'] as string) || '',
    )
    const [draftToolArtifactsStdout, setDraftToolArtifactsStdout] = useState<string>(
        (data['tool.artifacts.stdout'] as string) || '',
    )
    const [draftToolArtifactsStderr, setDraftToolArtifactsStderr] = useState<string>(
        (data['tool.artifacts.stderr'] as string) || '',
    )
    const [draftJoinPolicy, setDraftJoinPolicy] = useState<string>((data.join_policy as string) || 'wait_all')
    const [draftErrorPolicy, setDraftErrorPolicy] = useState<string>((data.error_policy as string) || 'continue')
    const [draftMaxParallel, setDraftMaxParallel] = useState<string>(
        data.max_parallel !== undefined ? String(data.max_parallel) : '4',
    )
    const [showAdvanced, setShowAdvanced] = useState(false)
    const [draftType, setDraftType] = useState<string>((data.type as string) || '')
    const [draftMaxRetries, setDraftMaxRetries] = useState<string>(
        data.max_retries !== undefined ? String(data.max_retries) : '',
    )
    const [draftGoalGate, setDraftGoalGate] = useState<boolean>(
        data.goal_gate === true || data.goal_gate === 'true',
    )
    const [draftRetryTarget, setDraftRetryTarget] = useState<string>((data.retry_target as string) || '')
    const [draftFallbackRetryTarget, setDraftFallbackRetryTarget] = useState<string>(
        (data.fallback_retry_target as string) || '',
    )
    const [draftFidelity, setDraftFidelity] = useState<string>((data.fidelity as string) || '')
    const [draftThreadId, setDraftThreadId] = useState<string>((data.thread_id as string) || '')
    const [draftClassName, setDraftClassName] = useState<string>((data.class as string) || '')
    const [draftTimeout, setDraftTimeout] = useState<string>((data.timeout as string) || '')
    const [draftLlmModel, setDraftLlmModel] = useState<string>((data.llm_model as string) || '')
    const [draftLlmProvider, setDraftLlmProvider] = useState<string>((data.llm_provider as string) || '')
    const [draftLlmProfile, setDraftLlmProfile] = useState<string>((data.llm_profile as string) || '')
    const [draftReasoningEffort, setDraftReasoningEffort] = useState<string>((data.reasoning_effort as string) || '')
    const [llmProfiles, setLlmProfiles] = useState<LlmProfileMetadata[]>([])
    const [draftAutoStatus, setDraftAutoStatus] = useState<boolean>(
        data.auto_status === true || data.auto_status === 'true',
    )
    const [draftAllowPartial, setDraftAllowPartial] = useState<boolean>(
        data.allow_partial === true || data.allow_partial === 'true',
    )
    const [draftManagerPollInterval, setDraftManagerPollInterval] = useState<string>(
        (data['manager.poll_interval'] as string) || '',
    )
    const [draftManagerMaxCycles, setDraftManagerMaxCycles] = useState<string>(
        data['manager.max_cycles'] !== undefined ? String(data['manager.max_cycles']) : '',
    )
    const [draftManagerStopCondition, setDraftManagerStopCondition] = useState<string>(
        (data['manager.stop_condition'] as string) || '',
    )
    const [draftManagerActions, setDraftManagerActions] = useState<string>(
        (data['manager.actions'] as string) || '',
    )
    const [draftHumanDefaultChoice, setDraftHumanDefaultChoice] = useState<string>(
        (data['human.default_choice'] as string) || '',
    )

    const status = (data.status as string) || 'idle'
    const derivedPreviewMeta = getDerivedPreviewMeta(data)
    const isReadOnlyPreviewNode = derivedPreviewMeta?.kind === 'child-cluster' || derivedPreviewMeta?.kind === 'child-node'
    const isReadOnlyEditorCanvas = isEditorCanvas && isExpandedChildPreview
    const renderedShape = isEditingDetails ? normalizeWorkflowNodeShape(draftShape) : currentShape
    const handlerType = getHandlerType(renderedShape, draftType)
    const visibility = getNodeFieldVisibility(handlerType)
    const draftToolHooksPreWarning = getToolHookCommandWarning(draftToolHooksPre)
    const draftToolHooksPostWarning = getToolHookCommandWarning(draftToolHooksPost)
    const diagnosticsForNode = nodeDiagnostics[id] || []
    const diagnosticsCount = diagnosticsForNode.length
    const hasDiagnosticError = diagnosticsForNode.some((diag) => diag.severity === 'error')
    const hasDiagnosticWarning = diagnosticsForNode.some((diag) => diag.severity === 'warning')
    const shapeTypeMismatchWarning = getShapeTypeMismatchWarning(renderedShape, draftType)
    const isWaiting = isRunCanvas && (humanGate?.nodeId === id || status === 'waiting')
    const framePalette = getWorkflowNodeFramePalette({ status, selected, isWaiting })
    const overlayOffsetClassName = getWorkflowNodeOverlayOffsetClassName(renderedShape)
    const containerStyle = getWorkflowNodeContainerStyle(renderedShape)
    const handleClassName = cn(
        'h-2.5 w-2.5 transition-opacity',
        isEditorCanvas && !isReadOnlyEditorCanvas
            ? 'border-border/70 bg-background/90 opacity-0 group-hover:opacity-100'
            : 'pointer-events-none border-transparent bg-transparent opacity-0',
        selected && isEditorCanvas && !isReadOnlyEditorCanvas ? 'opacity-100' : null,
    )

    useEffect(() => {
        if (isEditingLabel) {
            inputRef.current?.focus()
            inputRef.current?.select()
        }
    }, [isEditingLabel])

    useEffect(() => {
        void fetchLlmProfiles().then(setLlmProfiles)
    }, [])

    const persistNodeData = (nextData: Record<string, unknown>) => {
        if (!isEditorCanvas || !flowName || isReadOnlyEditorCanvas) {
            return
        }

        let updatedNodes: Node[] = []
        updateNodes((currentNodes) => {
            updatedNodes = currentNodes.map((node) => {
                if (node.id !== id) {
                    return node
                }
                const mergedData = { ...node.data, ...nextData }
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
            const dot = generateDot(flowName, updatedNodes, readEdges(), graphAttrs)
            void saveFlowContent(flowName, dot)
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

    const openDetailsEditor = (event: React.MouseEvent<HTMLButtonElement>) => {
        if (isReadOnlyEditorCanvas) {
            return
        }
        event.stopPropagation()
        setDraftShape((data.shape as string) || defaultShape)
        setDraftPrompt((data.prompt as string) || '')
        setDraftToolCommand((data['tool.command'] as string) || '')
        setDraftToolHooksPre((data['tool.hooks.pre'] as string) || '')
        setDraftToolHooksPost((data['tool.hooks.post'] as string) || '')
        setDraftToolArtifactsPaths((data['tool.artifacts.paths'] as string) || '')
        setDraftToolArtifactsStdout((data['tool.artifacts.stdout'] as string) || '')
        setDraftToolArtifactsStderr((data['tool.artifacts.stderr'] as string) || '')
        setDraftJoinPolicy((data.join_policy as string) || 'wait_all')
        setDraftErrorPolicy((data.error_policy as string) || 'continue')
        setDraftMaxParallel(data.max_parallel !== undefined ? String(data.max_parallel) : '4')
        setDraftType((data.type as string) || '')
        setDraftMaxRetries(data.max_retries !== undefined ? String(data.max_retries) : '')
        setDraftGoalGate(data.goal_gate === true || data.goal_gate === 'true')
        setDraftRetryTarget((data.retry_target as string) || '')
        setDraftFallbackRetryTarget((data.fallback_retry_target as string) || '')
        setDraftFidelity((data.fidelity as string) || '')
        setDraftThreadId((data.thread_id as string) || '')
        setDraftClassName((data.class as string) || '')
        setDraftTimeout((data.timeout as string) || '')
        setDraftLlmModel((data.llm_model as string) || '')
        setDraftLlmProvider((data.llm_provider as string) || '')
        setDraftLlmProfile((data.llm_profile as string) || '')
        setDraftReasoningEffort((data.reasoning_effort as string) || '')
        setDraftAutoStatus(data.auto_status === true || data.auto_status === 'true')
        setDraftAllowPartial(data.allow_partial === true || data.allow_partial === 'true')
        setDraftManagerPollInterval((data['manager.poll_interval'] as string) || '')
        setDraftManagerMaxCycles(data['manager.max_cycles'] !== undefined ? String(data['manager.max_cycles']) : '')
        setDraftManagerStopCondition((data['manager.stop_condition'] as string) || '')
        setDraftManagerActions((data['manager.actions'] as string) || '')
        setDraftHumanDefaultChoice((data['human.default_choice'] as string) || '')
        setIsEditingDetails(true)
    }

    const closeDetailsEditor = () => {
        setIsEditingDetails(false)
    }

    const saveDetails = () => {
        persistNodeData({
            shape: draftShape,
            prompt: draftPrompt,
            'tool.command': draftToolCommand,
            'tool.hooks.pre': draftToolHooksPre,
            'tool.hooks.post': draftToolHooksPost,
            'tool.artifacts.paths': draftToolArtifactsPaths,
            'tool.artifacts.stdout': draftToolArtifactsStdout,
            'tool.artifacts.stderr': draftToolArtifactsStderr,
            join_policy: draftJoinPolicy,
            error_policy: draftErrorPolicy,
            max_parallel: draftMaxParallel,
            type: draftType,
            max_retries: draftMaxRetries,
            goal_gate: draftGoalGate,
            retry_target: draftRetryTarget,
            fallback_retry_target: draftFallbackRetryTarget,
            fidelity: draftFidelity,
            thread_id: draftThreadId,
            class: draftClassName,
            timeout: draftTimeout,
            llm_model: draftLlmModel,
            llm_provider: draftLlmProvider,
            llm_profile: draftLlmProfile,
            reasoning_effort: draftReasoningEffort,
            auto_status: draftAutoStatus,
            allow_partial: draftAllowPartial,
            'manager.poll_interval': draftManagerPollInterval,
            'manager.max_cycles': draftManagerMaxCycles,
            'manager.stop_condition': draftManagerStopCondition,
            'manager.actions': draftManagerActions,
            'human.default_choice': draftHumanDefaultChoice,
        })
        setIsEditingDetails(false)
    }

    return (
        <div
            data-testid={`workflow-node-${renderedShape}`}
            data-workflow-shape={renderedShape}
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

            <div className={cn('absolute left-2 right-2 z-20 flex items-start justify-between', overlayOffsetClassName)}>
                <div className="min-w-0">
                    {isWaiting && (
                        <div className="rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-800">
                            Needs Input
                        </div>
                    )}
                    {isReadOnlyPreviewNode && (
                        <div className="mt-1 rounded-full bg-slate-500/12 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-slate-700">
                            Read-only Preview
                        </div>
                    )}
                </div>
                <div className="flex min-w-0 flex-col items-end gap-1">
                    {selected && isEditorCanvas && !isReadOnlyEditorCanvas && (
                        <button
                            onClick={openDetailsEditor}
                            className="rounded border border-border bg-background/90 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                        >
                            Edit
                        </button>
                    )}
                    {diagnosticsCount > 0 && (
                        <div
                            data-testid="node-diagnostic-badge"
                            className={cn(
                                'rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide',
                                hasDiagnosticError
                                    ? 'bg-destructive/15 text-destructive'
                                    : hasDiagnosticWarning
                                        ? 'bg-amber-500/15 text-amber-800'
                                        : 'bg-sky-500/15 text-sky-700',
                            )}
                            title={diagnosticsForNode.map((diag) => diag.message).join('\n')}
                        >
                            {diagnosticsCount} {hasDiagnosticError ? 'Error' : hasDiagnosticWarning ? 'Warn' : 'Info'}
                        </div>
                    )}
                </div>
            </div>

            <div
                onDoubleClick={startEditLabel}
                className={cn(
                    'absolute inset-0 z-10 flex items-center justify-center text-center',
                    getWorkflowNodeContentClassName(renderedShape),
                )}
            >
                <div className="flex w-full flex-col items-center justify-center gap-1">
                    {isEditingLabel ? (
                        <input
                            ref={inputRef}
                            value={draftLabel}
                            onChange={(event) => setDraftLabel(event.target.value)}
                            onBlur={commitLabel}
                            onKeyDown={onLabelKeyDown}
                            onPointerDown={(event) => event.stopPropagation()}
                            className="nodrag nopan h-7 w-full max-w-[152px] rounded border border-input bg-background px-2 text-center text-sm font-semibold outline-none ring-0 focus-visible:ring-1 focus-visible:ring-ring"
                        />
                    ) : (
                        <span className={cn(
                            'line-clamp-3 text-sm font-semibold',
                            isReadOnlyPreviewNode ? 'text-muted-foreground' : 'text-foreground',
                        )}
                        >
                            {displayLabel}
                        </span>
                    )}
                    {status !== 'idle' && (
                        <span
                            className={cn(
                                'rounded-sm px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wider',
                                getStatusBadgeClassName(status),
                            )}
                        >
                            {status}
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

            <NodeToolbar isVisible={isEditingDetails} position={Position.Bottom} className="nodrag nopan">
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
                                {WORKFLOW_NODE_SHAPE_OPTIONS.map((option) => (
                                    <option key={option.value} value={option.value}>
                                        {option.label}
                                    </option>
                                ))}
                            </select>
                        </div>
                        {visibility.showPrompt && (
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Prompt</label>
                                <textarea
                                    value={draftPrompt}
                                    onChange={(event) => setDraftPrompt(event.target.value)}
                                    className="nodrag h-20 w-full resize-none rounded-md border border-input bg-background px-2 py-1 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                />
                            </div>
                        )}
                        {visibility.showToolCommand && (
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
                        {visibility.showParallelOptions && (
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
                        {visibility.showManagerOptions && (
                            <>
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Manager Poll Interval</label>
                                    <input
                                        value={draftManagerPollInterval}
                                        onChange={(event) => setDraftManagerPollInterval(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="25ms"
                                    />
                                </div>
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Manager Max Cycles</label>
                                    <input
                                        value={draftManagerMaxCycles}
                                        onChange={(event) => setDraftManagerMaxCycles(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="3"
                                    />
                                </div>
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Manager Stop Condition</label>
                                    <input
                                        value={draftManagerStopCondition}
                                        onChange={(event) => setDraftManagerStopCondition(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder='child.status == "success"'
                                    />
                                </div>
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Manager Actions</label>
                                    <input
                                        value={draftManagerActions}
                                        onChange={(event) => setDraftManagerActions(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="observe,steer"
                                    />
                                </div>
                            </>
                        )}
                        {visibility.showHumanDefaultChoice && (
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Human Default Choice</label>
                                <input
                                    value={draftHumanDefaultChoice}
                                    onChange={(event) => setDraftHumanDefaultChoice(event.target.value)}
                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="target node id"
                                />
                            </div>
                        )}
                        {visibility.showTypeOverride && (
                            <div className="space-y-1">
                                <label className="text-xs font-medium text-foreground">Handler Type</label>
                                <input
                                    value={draftType}
                                    onChange={(event) => setDraftType(event.target.value)}
                                    list={`node-handler-type-options-${id}`}
                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                    placeholder="optional override"
                                />
                                <datalist id={`node-handler-type-options-${id}`}>
                                    {BUILTIN_HANDLER_OPTIONS.map((option) => (
                                        <option key={option} value={option}>
                                            {option}
                                        </option>
                                    ))}
                                </datalist>
                                {shapeTypeMismatchWarning && (
                                    <p data-testid="node-toolbar-shape-type-warning" className="text-[11px] text-amber-800">
                                        {shapeTypeMismatchWarning}
                                    </p>
                                )}
                            </div>
                        )}
                        {visibility.showAdvanced && (
                            <button
                                onClick={() => setShowAdvanced((prev) => !prev)}
                                className="mt-1 w-full rounded-md border border-border bg-background px-2 py-1 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                            >
                                {showAdvanced ? 'Hide Advanced' : 'Show Advanced'}
                            </button>
                        )}
                        {visibility.showAdvanced && showAdvanced && (
                            <div className="space-y-2 pt-1">
                                {visibility.showGeneralAdvanced && (
                                    <>
                                        <div className="grid grid-cols-2 gap-2">
                                            <div className="space-y-1">
                                                <label className="text-xs font-medium text-foreground">Max Retries</label>
                                                <input
                                                    value={draftMaxRetries}
                                                    onChange={(event) => setDraftMaxRetries(event.target.value)}
                                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                            </div>
                                            <div className="space-y-1">
                                                <label className="text-xs font-medium text-foreground">Timeout</label>
                                                <input
                                                    value={draftTimeout}
                                                    onChange={(event) => setDraftTimeout(event.target.value)}
                                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                    placeholder="900s"
                                                />
                                            </div>
                                        </div>
                                        <div className="flex items-center gap-2">
                                            <input
                                                id={`goal-gate-${id}`}
                                                type="checkbox"
                                                checked={draftGoalGate}
                                                onChange={(event) => setDraftGoalGate(event.target.checked)}
                                                className="h-4 w-4 rounded border border-input"
                                            />
                                            <label htmlFor={`goal-gate-${id}`} className="text-xs font-medium">
                                                Goal Gate
                                            </label>
                                        </div>
                                        <div className="space-y-1">
                                            <label className="text-xs font-medium text-foreground">Retry Target</label>
                                            <input
                                                value={draftRetryTarget}
                                                onChange={(event) => setDraftRetryTarget(event.target.value)}
                                                className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            />
                                        </div>
                                        <div className="space-y-1">
                                            <label className="text-xs font-medium text-foreground">Fallback Retry Target</label>
                                            <input
                                                value={draftFallbackRetryTarget}
                                                onChange={(event) => setDraftFallbackRetryTarget(event.target.value)}
                                                className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            />
                                        </div>
                                        {visibility.showToolCommand && (
                                            <>
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-foreground">Pre Hook Override</label>
                                                    <input
                                                        data-testid="node-toolbar-attr-input-tool.hooks.pre"
                                                        value={draftToolHooksPre}
                                                        onChange={(event) => setDraftToolHooksPre(event.target.value)}
                                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        placeholder="e.g. ./hooks/pre.sh"
                                                    />
                                                    {draftToolHooksPreWarning && (
                                                        <p data-testid="node-toolbar-attr-warning-tool.hooks.pre" className="text-[11px] text-amber-800">
                                                            {draftToolHooksPreWarning}
                                                        </p>
                                                    )}
                                                </div>
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-foreground">Post Hook Override</label>
                                                    <input
                                                        data-testid="node-toolbar-attr-input-tool.hooks.post"
                                                        value={draftToolHooksPost}
                                                        onChange={(event) => setDraftToolHooksPost(event.target.value)}
                                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        placeholder="e.g. ./hooks/post.sh"
                                                    />
                                                    {draftToolHooksPostWarning && (
                                                        <p data-testid="node-toolbar-attr-warning-tool.hooks.post" className="text-[11px] text-amber-800">
                                                            {draftToolHooksPostWarning}
                                                        </p>
                                                    )}
                                                </div>
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-foreground">Artifact Paths</label>
                                                    <input
                                                        data-testid="node-toolbar-attr-input-tool.artifacts.paths"
                                                        value={draftToolArtifactsPaths}
                                                        onChange={(event) => setDraftToolArtifactsPaths(event.target.value)}
                                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        placeholder="e.g. dist/**,reports/*.json"
                                                    />
                                                </div>
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-foreground">Stdout Artifact</label>
                                                    <input
                                                        data-testid="node-toolbar-attr-input-tool.artifacts.stdout"
                                                        value={draftToolArtifactsStdout}
                                                        onChange={(event) => setDraftToolArtifactsStdout(event.target.value)}
                                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        placeholder="e.g. stdout.txt"
                                                    />
                                                </div>
                                                <div className="space-y-1">
                                                    <label className="text-xs font-medium text-foreground">Stderr Artifact</label>
                                                    <input
                                                        data-testid="node-toolbar-attr-input-tool.artifacts.stderr"
                                                        value={draftToolArtifactsStderr}
                                                        onChange={(event) => setDraftToolArtifactsStderr(event.target.value)}
                                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                        placeholder="e.g. stderr.txt"
                                                    />
                                                </div>
                                            </>
                                        )}
                                        <div className="grid grid-cols-2 gap-2">
                                            <div className="space-y-1">
                                                <label className="text-xs font-medium text-foreground">Fidelity</label>
                                                <input
                                                    value={draftFidelity}
                                                    onChange={(event) => setDraftFidelity(event.target.value)}
                                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                            </div>
                                            <div className="space-y-1">
                                                <label className="text-xs font-medium text-foreground">Thread ID</label>
                                                <input
                                                    value={draftThreadId}
                                                    onChange={(event) => setDraftThreadId(event.target.value)}
                                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                            </div>
                                        </div>
                                        <div className="space-y-1">
                                            <label className="text-xs font-medium text-foreground">Class</label>
                                            <input
                                                value={draftClassName}
                                                onChange={(event) => setDraftClassName(event.target.value)}
                                                className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                            />
                                        </div>
                                    </>
                                )}
                                {visibility.showLlmSettings && (
                                    <>
                                        <div className="grid grid-cols-2 gap-2">
                                            <div className="space-y-1">
                                                <label className="text-xs font-medium text-foreground">LLM Model</label>
                                                <input
                                                    value={draftLlmModel}
                                                    onChange={(event) => setDraftLlmModel(event.target.value)}
                                                    list={`llm-model-options-${id}`}
                                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                                <datalist id={`llm-model-options-${id}`}>
                                                    {getModelSuggestions(draftLlmProfile || draftLlmProvider, llmProfiles).map((model) => (
                                                        <option key={model} value={model} />
                                                    ))}
                                                </datalist>
                                            </div>
                                            <div className="space-y-1">
                                                <label className="text-xs font-medium text-foreground">LLM Provider</label>
                                                <input
                                                    value={draftLlmProfile || draftLlmProvider}
                                                    onChange={(event) => {
                                                        const selection = splitLlmSelection(event.target.value, llmProfiles)
                                                        setDraftLlmProvider(selection.llm_provider)
                                                        setDraftLlmProfile(selection.llm_profile)
                                                    }}
                                                    list={`llm-provider-options-${id}`}
                                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                                <datalist id={`llm-provider-options-${id}`}>
                                                    {getLlmSelectionOptions(llmProfiles).map((provider) => (
                                                        <option key={provider} value={provider} />
                                                    ))}
                                                </datalist>
                                            </div>
                                        </div>
                                        <div className="space-y-1">
                                            <label className="text-xs font-medium text-foreground">Reasoning Effort</label>
                                            <input
                                                value={draftReasoningEffort}
                                                onChange={(event) => setDraftReasoningEffort(event.target.value)}
                                                className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                placeholder="high"
                                            />
                                        </div>
                                    </>
                                )}
                                {visibility.showGeneralAdvanced && (
                                    <div className="flex items-center gap-4">
                                        <label className="flex items-center gap-2 text-xs font-medium">
                                            <input
                                                type="checkbox"
                                                checked={draftAutoStatus}
                                                onChange={(event) => setDraftAutoStatus(event.target.checked)}
                                                className="h-4 w-4 rounded border border-input"
                                            />
                                            Auto Status
                                        </label>
                                        <label className="flex items-center gap-2 text-xs font-medium">
                                            <input
                                                type="checkbox"
                                                checked={draftAllowPartial}
                                                onChange={(event) => setDraftAllowPartial(event.target.checked)}
                                                className="h-4 w-4 rounded border border-input"
                                            />
                                            Allow Partial
                                        </label>
                                    </div>
                                )}
                            </div>
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

export function ConditionalNode(props: NodeProps) {
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
