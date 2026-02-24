import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Handle, NodeToolbar, Position, type Node, type NodeProps, useReactFlow } from '@xyflow/react';
import { useStore } from '@/store';
import { generateDot } from '@/lib/dotUtils';
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from '@/lib/llmSuggestions';
import { getHandlerType, getNodeFieldVisibility } from '@/lib/nodeVisibility';

export function TaskNode({ id, data, selected }: NodeProps) {
    const { activeFlow, viewMode } = useStore();
    const humanGate = useStore((state) => state.humanGate);
    const graphAttrs = useStore((state) => state.graphAttrs);
    const nodeDiagnostics = useStore((state) => state.nodeDiagnostics);
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
    const [showAdvanced, setShowAdvanced] = useState(false);
    const [draftType, setDraftType] = useState<string>((data.type as string) || '');
    const [draftMaxRetries, setDraftMaxRetries] = useState<string>(
        data.max_retries !== undefined ? String(data.max_retries) : ''
    );
    const [draftGoalGate, setDraftGoalGate] = useState<boolean>(
        data.goal_gate === true || data.goal_gate === 'true'
    );
    const [draftRetryTarget, setDraftRetryTarget] = useState<string>((data.retry_target as string) || '');
    const [draftFallbackRetryTarget, setDraftFallbackRetryTarget] = useState<string>(
        (data.fallback_retry_target as string) || ''
    );
    const [draftFidelity, setDraftFidelity] = useState<string>((data.fidelity as string) || '');
    const [draftThreadId, setDraftThreadId] = useState<string>((data.thread_id as string) || '');
    const [draftClassName, setDraftClassName] = useState<string>((data.class as string) || '');
    const [draftTimeout, setDraftTimeout] = useState<string>((data.timeout as string) || '');
    const [draftLlmModel, setDraftLlmModel] = useState<string>((data.llm_model as string) || '');
    const [draftLlmProvider, setDraftLlmProvider] = useState<string>((data.llm_provider as string) || '');
    const [draftReasoningEffort, setDraftReasoningEffort] = useState<string>((data.reasoning_effort as string) || '');
    const [draftAutoStatus, setDraftAutoStatus] = useState<boolean>(
        data.auto_status === true || data.auto_status === 'true'
    );
    const [draftAllowPartial, setDraftAllowPartial] = useState<boolean>(
        data.allow_partial === true || data.allow_partial === 'true'
    );
    const status = (data.status as string) || 'idle';
    const nodeShape = (data.shape as string) || 'box';
    const isBoxShape = nodeShape === 'box';
    const handlerType = getHandlerType(draftShape, draftType);
    const visibility = getNodeFieldVisibility(handlerType);
    const diagnosticsForNode = nodeDiagnostics[id] || [];
    const diagnosticsCount = diagnosticsForNode.length;
    const hasDiagnosticError = diagnosticsForNode.some((diag) => diag.severity === 'error');
    const hasDiagnosticWarning = diagnosticsForNode.some((diag) => diag.severity === 'warning');

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
        setDraftType((data.type as string) || '');
        setDraftMaxRetries(data.max_retries !== undefined ? String(data.max_retries) : '');
        setDraftGoalGate(data.goal_gate === true || data.goal_gate === 'true');
        setDraftRetryTarget((data.retry_target as string) || '');
        setDraftFallbackRetryTarget((data.fallback_retry_target as string) || '');
        setDraftFidelity((data.fidelity as string) || '');
        setDraftThreadId((data.thread_id as string) || '');
        setDraftClassName((data.class as string) || '');
        setDraftTimeout((data.timeout as string) || '');
        setDraftLlmModel((data.llm_model as string) || '');
        setDraftLlmProvider((data.llm_provider as string) || '');
        setDraftReasoningEffort((data.reasoning_effort as string) || '');
        setDraftAutoStatus(data.auto_status === true || data.auto_status === 'true');
        setDraftAllowPartial(data.allow_partial === true || data.allow_partial === 'true');
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
            reasoning_effort: draftReasoningEffort,
            auto_status: draftAutoStatus,
            allow_partial: draftAllowPartial,
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

    const shapeClasses: string[] = [];
    let minHeightClass = '';
    let showComponentBars = false;

    switch (nodeShape) {
        case 'diamond':
            shapeClasses.push('rounded-none');
            minHeightClass = 'min-h-[110px] min-w-[220px]';
            break;
        case 'Mdiamond':
            shapeClasses.push('rounded-none');
            minHeightClass = 'min-h-[110px] min-w-[220px]';
            break;
        case 'hexagon':
            shapeClasses.push('rounded-none');
            minHeightClass = 'min-h-[110px] min-w-[220px]';
            break;
        case 'parallelogram':
            shapeClasses.push('rounded-none');
            minHeightClass = 'min-h-[110px] min-w-[220px]';
            break;
        case 'tripleoctagon':
            shapeClasses.push('rounded-none');
            minHeightClass = 'min-h-[110px] min-w-[220px]';
            break;
        case 'component':
            shapeClasses.push('rounded-none', 'pl-8');
            showComponentBars = true;
            minHeightClass = 'min-h-[110px] min-w-[220px]';
            break;
        case 'Msquare':
            shapeClasses.push('rounded-none');
            minHeightClass = 'min-h-[110px] min-w-[220px]';
            break;
        default:
            break;
    }

    const isSvgShape = nodeShape !== 'box';
    const svgWidth = 220;
    const svgHeight = 110;
    const svgStrokeWidth = nodeShape === 'Mdiamond' || nodeShape === 'Msquare' ? 3 : 2;
    const svgFill = 'hsl(var(--card) / 0.95)';
    const svgStroke = (() => {
        if (status === 'success') return 'hsl(142 76% 36%)';
        if (status === 'failed') return 'hsl(var(--destructive))';
        if (status === 'running') return 'hsl(var(--primary))';
        if (isWaiting) return 'hsl(38 92% 50%)';
        if (selected) return 'hsl(var(--foreground))';
        return 'hsl(var(--border))';
    })();
    const svgStrokeInner =
        nodeShape === 'Mdiamond' || nodeShape === 'Msquare' ? svgStroke : '';
    const svgShadow = 'drop-shadow(0 1px 2px hsl(240 10% 4% / 0.24))';
    const svgPathForShape = () => {
        switch (nodeShape) {
            case 'diamond':
            case 'Mdiamond':
                return 'M110 6 L214 55 L110 104 L6 55 Z';
            case 'hexagon':
                return 'M55 6 H165 L214 55 L165 104 H55 L6 55 Z';
            case 'parallelogram':
                return 'M30 6 H214 L190 104 H6 Z';
            case 'tripleoctagon':
                return 'M65 6 H155 L214 35 V75 L155 104 H65 L6 75 V35 Z';
            case 'component':
                return 'M6 6 H214 V104 H6 Z';
            case 'Msquare':
                return 'M6 6 H214 V104 H6 Z';
            default:
                return '';
        }
    };

    return (
        <div
            onDoubleClick={startEditLabel}
            className="flow-node relative"
        >
            <Handle type="target" position={Position.Top} className="w-3 h-3 bg-muted-foreground border-border" />

            <div className="absolute right-2 top-2 z-10 flex flex-col items-end gap-1">
                {selected && viewMode === 'editor' && (
                    <button
                        onClick={openDetailsEditor}
                        className="rounded border border-border bg-background/90 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-muted-foreground hover:text-foreground"
                    >
                        Edit
                    </button>
                )}
                {diagnosticsCount > 0 && (
                    <div
                        className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                            hasDiagnosticError
                                ? 'bg-destructive/15 text-destructive'
                                : hasDiagnosticWarning
                                    ? 'bg-amber-500/15 text-amber-700'
                                    : 'bg-sky-500/15 text-sky-700'
                        }`}
                        title={diagnosticsForNode.map((diag) => diag.message).join('\n')}
                    >
                        {diagnosticsCount} {hasDiagnosticError ? 'Error' : hasDiagnosticWarning ? 'Warn' : 'Info'}
                    </div>
                )}
            </div>
            {isWaiting && (
                <div className="absolute left-2 top-2 z-10 rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-700">
                    Needs Input
                </div>
            )}

            <div
                className={`flow-node__body text-card-foreground p-4 min-w-[150px] relative ${minHeightClass} transition-[color,box-shadow,border-color,background-color] ${isBoxShape ? `bg-card/95 shadow-sm border rounded-md ${borderColor} hover:shadow-md` : 'bg-transparent border-0 shadow-none'} ${shapeClasses.join(' ')}`}
                data-shape={nodeShape}
            >
                {isSvgShape && (
                    <svg
                        width={svgWidth}
                        height={svgHeight}
                        viewBox={`0 0 ${svgWidth} ${svgHeight}`}
                        className="pointer-events-none absolute left-1/2 top-1/2 h-[110px] w-[220px] -translate-x-1/2 -translate-y-1/2 text-border"
                        aria-hidden
                    >
                        <path
                            d={svgPathForShape()}
                            fill={svgFill}
                            stroke={svgStroke}
                            strokeWidth={svgStrokeWidth}
                            style={{ filter: svgShadow }}
                        />
                        {svgStrokeInner && (
                            <path
                                d={svgPathForShape()}
                                fill="none"
                                stroke={svgStrokeInner}
                                strokeWidth={1}
                                transform="translate(2 2) scale(0.96)"
                            />
                        )}
                    </svg>
                )}
                {showComponentBars && (
                    <span
                        aria-hidden
                        className="pointer-events-none absolute left-2 top-2 bottom-2 w-2 border-l-2 border-r-2 border-border/60"
                    />
                )}
                <div className="relative flex flex-col gap-1 items-center justify-center">
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
                                <div className="space-y-1">
                                    <label className="text-xs font-medium text-foreground">Handler Type</label>
                                    <input
                                        value={draftType}
                                        onChange={(event) => setDraftType(event.target.value)}
                                        className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                        placeholder="optional override"
                                    />
                                </div>
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
                                                    {getModelSuggestions(draftLlmProvider).map((model) => (
                                                        <option key={model} value={model} />
                                                    ))}
                                                </datalist>
                                            </div>
                                            <div className="space-y-1">
                                                <label className="text-xs font-medium text-foreground">LLM Provider</label>
                                                <input
                                                    value={draftLlmProvider}
                                                    onChange={(event) => setDraftLlmProvider(event.target.value)}
                                                    list={`llm-provider-options-${id}`}
                                                    className="nodrag h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                                />
                                                <datalist id={`llm-provider-options-${id}`}>
                                                    {LLM_PROVIDER_OPTIONS.map((provider) => (
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
