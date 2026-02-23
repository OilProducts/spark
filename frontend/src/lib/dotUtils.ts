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

        const label = `label="${escapeDotString(labelValue)}"`;
        const shape = shapeValue ? `shape=${formatAttrValue(shapeValue)}` : '';
        const prompt = promptValue ? `prompt="${escapeDotString(promptValue)}"` : '';

        const attrs = [
            `label=${label}`,
            shape,
            prompt
        ].filter(Boolean).join(', ');

        dot += `  ${n.id} [${attrs}];\n`;
    });

    edges.forEach(e => {
        dot += `  ${e.source} -> ${e.target};\n`;
    });

    dot += `}\n`;
    return dot;
}
