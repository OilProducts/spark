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
            maxParallel
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
