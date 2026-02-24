import type { Edge, Node } from '@xyflow/react';
import type { GraphAttrs } from '@/store';

function escapeDotString(value: string): string {
    return value
        .replace(/\\/g, '\\\\')
        .replace(/"/g, '\\"')
        .replace(/\n/g, '\\n');
}

function formatAttrValue(value: string): string {
    if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(value)) {
        return value;
    }
    return `"${escapeDotString(value)}"`;
}

export function generateDot(
    flowName: string,
    nodes: Node[],
    edges: Edge[],
    graphAttrs: GraphAttrs = {}
): string {
    // Strip .dot for graph name
    const name = flowName.replace('.dot', '');

    let dot = `digraph ${name} {\n`;

    const graphAttrLines = [
        _formatGraphAttr('goal', graphAttrs.goal),
        _formatGraphAttr('label', graphAttrs.label),
        _formatGraphAttr('model_stylesheet', graphAttrs.model_stylesheet),
        _formatIntAttr('default_max_retry', graphAttrs.default_max_retry ?? ''),
        _formatGraphAttr('retry_target', graphAttrs.retry_target),
        _formatGraphAttr('fallback_retry_target', graphAttrs.fallback_retry_target),
        _formatGraphAttr('default_fidelity', graphAttrs.default_fidelity),
        _formatGraphAttr('ui_default_llm_model', graphAttrs.ui_default_llm_model),
        _formatGraphAttr('ui_default_llm_provider', graphAttrs.ui_default_llm_provider),
        _formatGraphAttr('ui_default_reasoning_effort', graphAttrs.ui_default_reasoning_effort),
    ].filter(Boolean);

    if (graphAttrLines.length > 0) {
        dot += `  graph [${graphAttrLines.join(', ')}];\n`;
    }

    nodes.forEach(n => {
        const labelValue = typeof n.data.label === 'string' ? n.data.label : n.id;
        const shapeValue = typeof n.data.shape === 'string' ? n.data.shape : '';
        const promptValue = typeof n.data.prompt === 'string' ? n.data.prompt : '';
        const toolCommandValue = typeof n.data.tool_command === 'string' ? n.data.tool_command : '';
        const joinPolicyValue = typeof n.data.join_policy === 'string' ? n.data.join_policy : '';
        const errorPolicyValue = typeof n.data.error_policy === 'string' ? n.data.error_policy : '';
        const maxParallelValue = typeof n.data.max_parallel === 'string' || typeof n.data.max_parallel === 'number'
            ? n.data.max_parallel
            : '';
        const typeValue = typeof n.data.type === 'string' ? n.data.type : '';
        const maxRetriesValue = typeof n.data.max_retries === 'string' || typeof n.data.max_retries === 'number'
            ? n.data.max_retries
            : '';
        const goalGateValue = n.data.goal_gate === true || n.data.goal_gate === 'true';
        const retryTargetValue = typeof n.data.retry_target === 'string' ? n.data.retry_target : '';
        const fallbackRetryTargetValue = typeof n.data.fallback_retry_target === 'string'
            ? n.data.fallback_retry_target
            : '';
        const fidelityValue = typeof n.data.fidelity === 'string' ? n.data.fidelity : '';
        const threadIdValue = typeof n.data.thread_id === 'string' ? n.data.thread_id : '';
        const classValue = typeof n.data.class === 'string' ? n.data.class : '';
        const timeoutValue = typeof n.data.timeout === 'string' ? n.data.timeout : '';
        const llmModelValue = typeof n.data.llm_model === 'string' ? n.data.llm_model : '';
        const llmProviderValue = typeof n.data.llm_provider === 'string' ? n.data.llm_provider : '';
        const reasoningEffortValue = typeof n.data.reasoning_effort === 'string' ? n.data.reasoning_effort : '';
        const autoStatusValue = n.data.auto_status === true || n.data.auto_status === 'true';
        const allowPartialValue = n.data.allow_partial === true || n.data.allow_partial === 'true';

        const label = `"${escapeDotString(labelValue)}"`;
        const shape = shapeValue ? `shape=${formatAttrValue(shapeValue)}` : '';
        const prompt = promptValue ? `prompt="${escapeDotString(promptValue)}"` : '';
        const toolCommand = toolCommandValue ? `tool_command="${escapeDotString(toolCommandValue)}"` : '';
        const joinPolicy = joinPolicyValue ? `join_policy=${formatAttrValue(joinPolicyValue)}` : '';
        const errorPolicy = errorPolicyValue ? `error_policy=${formatAttrValue(errorPolicyValue)}` : '';
        const maxParallel = _formatIntAttr('max_parallel', maxParallelValue);

        const attrs = [
            `label=${label}`,
            shape,
            prompt,
            toolCommand,
            joinPolicy,
            errorPolicy,
            maxParallel,
            typeValue ? `type=${formatAttrValue(typeValue)}` : '',
            _formatIntAttr('max_retries', maxRetriesValue),
            goalGateValue ? `goal_gate=true` : '',
            retryTargetValue ? `retry_target=${formatAttrValue(retryTargetValue)}` : '',
            fallbackRetryTargetValue ? `fallback_retry_target=${formatAttrValue(fallbackRetryTargetValue)}` : '',
            fidelityValue ? `fidelity=${formatAttrValue(fidelityValue)}` : '',
            threadIdValue ? `thread_id="${escapeDotString(threadIdValue)}"` : '',
            classValue ? `class="${escapeDotString(classValue)}"` : '',
            timeoutValue ? _formatDurationAttr('timeout', timeoutValue) : '',
            llmModelValue ? `llm_model=${formatAttrValue(llmModelValue)}` : '',
            llmProviderValue ? `llm_provider=${formatAttrValue(llmProviderValue)}` : '',
            reasoningEffortValue ? `reasoning_effort=${formatAttrValue(reasoningEffortValue)}` : '',
            autoStatusValue ? `auto_status=true` : '',
            allowPartialValue ? `allow_partial=true` : ''
        ].filter(Boolean).join(', ');

        dot += `  ${n.id} [${attrs}];\n`;
    });

    edges.forEach(e => {
        const edgeData = (e.data || {}) as Record<string, unknown>;
        const labelValue = typeof edgeData.label === 'string' ? edgeData.label : '';
        const conditionValue = typeof edgeData.condition === 'string' ? edgeData.condition : '';
        const weightValue = typeof edgeData.weight === 'string' || typeof edgeData.weight === 'number'
            ? edgeData.weight
            : '';
        const fidelityValue = typeof edgeData.fidelity === 'string' ? edgeData.fidelity : '';
        const threadIdValue = typeof edgeData.thread_id === 'string' ? edgeData.thread_id : '';
        const loopRestartValue = edgeData.loop_restart === true || edgeData.loop_restart === 'true';

        const edgeAttrs = [
            labelValue ? `label="${escapeDotString(labelValue)}"` : '',
            conditionValue ? `condition="${escapeDotString(conditionValue)}"` : '',
            _formatIntAttr('weight', weightValue),
            fidelityValue ? `fidelity=${formatAttrValue(fidelityValue)}` : '',
            threadIdValue ? `thread_id="${escapeDotString(threadIdValue)}"` : '',
            loopRestartValue ? `loop_restart=true` : '',
        ].filter(Boolean).join(', ');

        if (edgeAttrs) {
            dot += `  ${e.source} -> ${e.target} [${edgeAttrs}];\n`;
        } else {
            dot += `  ${e.source} -> ${e.target};\n`;
        }
    });

    dot += `}\n`;
    return dot;
}

function _formatIntAttr(key: string, value: string | number): string {
    if (value === "" || value === null || value === undefined) return "";
    const parsed = typeof value === 'number' ? Math.floor(value) : parseInt(value, 10);
    if (Number.isNaN(parsed)) return "";
    return `${key}=${parsed}`;
}

function _formatGraphAttr(key: string, value?: string): string {
    if (!value) return "";
    return `${key}="${escapeDotString(value)}"`;
}

function _formatDurationAttr(key: string, value: string): string {
    const trimmed = value.trim();
    if (trimmed === "") return "";
    if (/^\d+(ms|s|m|h|d)$/.test(trimmed)) {
        return `${key}=${trimmed}`;
    }
    return `${key}="${escapeDotString(trimmed)}"`;
}
