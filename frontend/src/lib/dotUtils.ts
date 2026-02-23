import type { Edge, Node } from '@xyflow/react';

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

export function generateDot(flowName: string, nodes: Node[], edges: Edge[]): string {
    // Strip .dot for graph name
    const name = flowName.replace('.dot', '');

    let dot = `digraph ${name} {\n`;

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
        dot += `  ${e.source} -> ${e.target};\n`;
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
