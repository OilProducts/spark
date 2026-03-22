import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { Handle, NodeToolbar, Position, type Node, type NodeProps, useReactFlow } from '@xyflow/react';
import { useStore } from '@/store';
import { generateDot } from '@/lib/dotUtils';
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from '@/lib/llmSuggestions';
import { getHandlerType, getNodeFieldVisibility } from '@/lib/nodeVisibility';
import { getToolHookCommandWarning } from '@/lib/graphAttrValidation';
import { saveFlowContent } from '@/lib/flowPersistence';
import { fetchPipelineAnswerValidated } from '@/lib/attractorClient';

export function TaskNode({ id, data, selected }: NodeProps) {
    const { activeFlow, viewMode } = useStore();
    const humanGate = useStore((state) => state.humanGate);
    const selectedRunId = useStore((state) => state.selectedRunId);
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
    const [draftToolCommand, setDraftToolCommand] = useState<string>((data['tool.command'] as string) || '');
    const [draftToolHooksPre, setDraftToolHooksPre] = useState<string>((data['tool.hooks.pre'] as string) || '');
    const [draftToolHooksPost, setDraftToolHooksPost] = useState<string>((data['tool.hooks.post'] as string) || '');
    const [draftToolArtifactsPaths, setDraftToolArtifactsPaths] = useState<string>(
        (data['tool.artifacts.paths'] as string) || ''
    );
    const [draftToolArtifactsStdout, setDraftToolArtifactsStdout] = useState<string>(
        (data['tool.artifacts.stdout'] as string) || ''
    );
    const [draftToolArtifactsStderr, setDraftToolArtifactsStderr] = useState<string>(
        (data['tool.artifacts.stderr'] as string) || ''
    );
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
    const [draftManagerPollInterval, setDraftManagerPollInterval] = useState<string>(
        (data['manager.poll_interval'] as string) || ''
    );
    const [draftManagerMaxCycles, setDraftManagerMaxCycles] = useState<string>(
        data['manager.max_cycles'] !== undefined ? String(data['manager.max_cycles']) : ''
    );
    const [draftManagerStopCondition, setDraftManagerStopCondition] = useState<string>(
        (data['manager.stop_condition'] as string) || ''
    );
    const [draftManagerActions, setDraftManagerActions] = useState<string>(
        (data['manager.actions'] as string) || ''
    );
    const [draftHumanDefaultChoice, setDraftHumanDefaultChoice] = useState<string>(
        (data['human.default_choice'] as string) || ''
    );
    const status = (data.status as string) || 'idle';
    const handlerType = getHandlerType(draftShape, draftType);
    const visibility = getNodeFieldVisibility(handlerType);
    const draftToolHooksPreWarning = getToolHookCommandWarning(draftToolHooksPre);
    const draftToolHooksPostWarning = getToolHookCommandWarning(draftToolHooksPost);
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
            void saveFlowContent(activeFlow, dot);
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
        setDraftToolCommand((data['tool.command'] as string) || '');
        setDraftToolHooksPre((data['tool.hooks.pre'] as string) || '');
        setDraftToolHooksPost((data['tool.hooks.post'] as string) || '');
        setDraftToolArtifactsPaths((data['tool.artifacts.paths'] as string) || '');
        setDraftToolArtifactsStdout((data['tool.artifacts.stdout'] as string) || '');
        setDraftToolArtifactsStderr((data['tool.artifacts.stderr'] as string) || '');
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
        setDraftManagerPollInterval((data['manager.poll_interval'] as string) || '');
        setDraftManagerMaxCycles(data['manager.max_cycles'] !== undefined ? String(data['manager.max_cycles']) : '');
        setDraftManagerStopCondition((data['manager.stop_condition'] as string) || '');
        setDraftManagerActions((data['manager.actions'] as string) || '');
        setDraftHumanDefaultChoice((data['human.default_choice'] as string) || '');
        setIsEditingDetails(true);
    };

    const closeDetailsEditor = () => {
        setIsEditingDetails(false);
    };

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
            reasoning_effort: draftReasoningEffort,
            auto_status: draftAutoStatus,
            allow_partial: draftAllowPartial,
            'manager.poll_interval': draftManagerPollInterval,
            'manager.max_cycles': draftManagerMaxCycles,
            'manager.stop_condition': draftManagerStopCondition,
            'manager.actions': draftManagerActions,
            'human.default_choice': draftHumanDefaultChoice,
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

    return (
        <div
            onDoubleClick={startEditLabel}
            className={`bg-card/95 text-card-foreground shadow-sm rounded-md border p-4 min-w-[150px] relative ${borderColor} transition-[color,box-shadow,border-color,background-color] hover:shadow-md`}
        >
            <Handle type="target" position={Position.Top} className="w-3 h-3 bg-muted-foreground border-border" />

            <div className="absolute right-2 top-2 flex flex-col items-end gap-1">
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
                        data-testid="node-diagnostic-badge"
                        className={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide ${
                            hasDiagnosticError
                                ? 'bg-destructive/15 text-destructive'
                                : hasDiagnosticWarning
                                    ? 'bg-amber-500/15 text-amber-800'
                                    : 'bg-sky-500/15 text-sky-700'
                        }`}
                        title={diagnosticsForNode.map((diag) => diag.message).join('\n')}
                    >
                        {diagnosticsCount} {hasDiagnosticError ? 'Error' : hasDiagnosticWarning ? 'Warn' : 'Info'}
                    </div>
                )}
            </div>
            {isWaiting && (
                <div className="absolute left-2 top-2 rounded-full bg-amber-500/15 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-amber-800">
                    Needs Input
                </div>
            )}

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
                                <option value="house">Manager Loop</option>
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
                                        placeholder='child.status == \"success\"'
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
                                    <option value="start">start</option>
                                    <option value="exit">exit</option>
                                    <option value="codergen">codergen</option>
                                    <option value="wait.human">wait.human</option>
                                    <option value="conditional">conditional</option>
                                    <option value="parallel">parallel</option>
                                    <option value="parallel.fan_in">parallel.fan_in</option>
                                    <option value="tool">tool</option>
                                    <option value="stack.manager_loop">stack.manager_loop</option>
                                </datalist>
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
                        <div className="text-xs font-semibold uppercase tracking-wide text-amber-800">
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
                                        if (!humanGate.runId) return
                                        fetchPipelineAnswerValidated(humanGate.runId, humanGate.id, option.value).catch(console.error)
                                    }}
                                    disabled={!humanGate.runId || !selectedRunId}
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
