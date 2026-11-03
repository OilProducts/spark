import type { DiagnosticEntry } from '@/store'

export type FieldDiagnosticMap = Record<string, DiagnosticEntry[]>

const appendFieldDiagnostic = (fieldDiagnostics: FieldDiagnosticMap, field: string, diagnostic: DiagnosticEntry) => {
    if (!fieldDiagnostics[field]) {
        fieldDiagnostics[field] = []
    }
    fieldDiagnostics[field].push(diagnostic)
}

// Node-level rules about a node's typed config (switch, fork, exit, tool,
// subflow or response contract).
const NODE_CONFIG_RULE_IDS = new Set([
    'node_config',
    'switch_writer',
    'exit_result',
    'fork_structure',
    'response_contract',
    'subflow_child',
])

const mapNodeDiagnosticFields = (diagnostic: DiagnosticEntry): string[] => {
    if (diagnostic.rule_id === 'prompt_on_llm_nodes') {
        return ['prompt', 'label']
    }
    if (diagnostic.rule_id === 'type_known') {
        return ['type']
    }
    if (NODE_CONFIG_RULE_IDS.has(diagnostic.rule_id)) {
        return ['config']
    }
    return []
}

const mapEdgeDiagnosticFields = (diagnostic: DiagnosticEntry): string[] => {
    if (diagnostic.rule_id === 'route_description') {
        return ['description']
    }
    if (diagnostic.decision || diagnostic.rule_id === 'decision_id' || diagnostic.rule_id === 'decision_domain') {
        return ['decision']
    }
    return []
}

const mapGraphDiagnosticFields = (): string[] => []

const toFieldDiagnosticMap = (
    diagnostics: DiagnosticEntry[],
    mapDiagnosticFields: (diagnostic: DiagnosticEntry) => string[],
): FieldDiagnosticMap => {
    const fieldDiagnostics: FieldDiagnosticMap = {}
    diagnostics.forEach((diagnostic) => {
        mapDiagnosticFields(diagnostic).forEach((field) => {
            appendFieldDiagnostic(fieldDiagnostics, field, diagnostic)
        })
    })
    return fieldDiagnostics
}

export const resolveNodeFieldDiagnostics = (diagnostics: DiagnosticEntry[], nodeId: string): FieldDiagnosticMap => {
    const nodeDiagnostics = diagnostics.filter((diagnostic) => diagnostic.node_id === nodeId)
    return toFieldDiagnosticMap(nodeDiagnostics, mapNodeDiagnosticFields)
}

// An edge's diagnostics; with a decision, only those for that route (several
// decisions may target the same node).
export const resolveEdgeFieldDiagnostics = (
    diagnostics: DiagnosticEntry[],
    source: string,
    target: string,
    decision?: string,
): FieldDiagnosticMap => {
    const edgeDiagnostics = diagnostics.filter(
        (diagnostic) => diagnostic.edge?.length === 2 && diagnostic.edge[0] === source && diagnostic.edge[1] === target
            && (!decision || !diagnostic.decision || diagnostic.decision === decision),
    )
    return toFieldDiagnosticMap(edgeDiagnostics, mapEdgeDiagnosticFields)
}

export const resolveGraphFieldDiagnostics = (diagnostics: DiagnosticEntry[]): FieldDiagnosticMap => {
    const graphDiagnostics = diagnostics.filter((diagnostic) => !diagnostic.node_id && !diagnostic.edge)
    return toFieldDiagnosticMap(graphDiagnostics, mapGraphDiagnosticFields)
}
