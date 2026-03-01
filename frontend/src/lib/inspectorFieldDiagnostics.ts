import type { DiagnosticEntry } from '@/store'

export type FieldDiagnosticMap = Record<string, DiagnosticEntry[]>

const appendFieldDiagnostic = (fieldDiagnostics: FieldDiagnosticMap, field: string, diagnostic: DiagnosticEntry) => {
    if (!fieldDiagnostics[field]) {
        fieldDiagnostics[field] = []
    }
    fieldDiagnostics[field].push(diagnostic)
}

const resolveRetryTargetFields = (diagnostic: DiagnosticEntry): string[] => {
    const message = diagnostic.message.toLowerCase()
    const fields: string[] = []

    const mentionsFallbackRetryTarget = /\bfallback_retry_target\b/.test(message)
    const retryOnlyMessage = message.replaceAll('fallback_retry_target', '')
    const mentionsRetryTarget = /\bretry_target\b/.test(retryOnlyMessage)

    if (mentionsRetryTarget) {
        fields.push('retry_target')
    }
    if (mentionsFallbackRetryTarget) {
        fields.push('fallback_retry_target')
    }

    return fields.length > 0 ? fields : ['retry_target', 'fallback_retry_target']
}

const mapNodeDiagnosticFields = (diagnostic: DiagnosticEntry): string[] => {
    if (diagnostic.rule_id === 'prompt_on_llm_nodes') {
        return ['prompt', 'label']
    }
    if (diagnostic.rule_id === 'type_known') {
        return ['type']
    }
    if (diagnostic.rule_id === 'goal_gate_has_retry') {
        return ['goal_gate', 'retry_target', 'fallback_retry_target']
    }
    if (diagnostic.rule_id === 'retry_target_exists') {
        return resolveRetryTargetFields(diagnostic)
    }
    if (diagnostic.rule_id === 'fidelity_valid') {
        return ['fidelity']
    }
    return []
}

const mapEdgeDiagnosticFields = (diagnostic: DiagnosticEntry): string[] => {
    if (diagnostic.rule_id === 'condition_syntax') {
        return ['condition']
    }
    if (diagnostic.rule_id === 'fidelity_valid') {
        return ['fidelity']
    }
    return []
}

const mapGraphDiagnosticFields = (diagnostic: DiagnosticEntry): string[] => {
    if (diagnostic.rule_id === 'stylesheet_syntax') {
        return ['model_stylesheet']
    }
    if (diagnostic.rule_id === 'retry_target_exists') {
        return resolveRetryTargetFields(diagnostic)
    }
    if (diagnostic.rule_id === 'fidelity_valid' && diagnostic.message.toLowerCase().startsWith('graph fidelity')) {
        return ['default_fidelity']
    }
    return []
}

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

export const resolveEdgeFieldDiagnostics = (
    diagnostics: DiagnosticEntry[],
    source: string,
    target: string,
): FieldDiagnosticMap => {
    const edgeDiagnostics = diagnostics.filter(
        (diagnostic) => diagnostic.edge?.length === 2 && diagnostic.edge[0] === source && diagnostic.edge[1] === target,
    )
    return toFieldDiagnosticMap(edgeDiagnostics, mapEdgeDiagnosticFields)
}

export const resolveGraphFieldDiagnostics = (diagnostics: DiagnosticEntry[]): FieldDiagnosticMap => {
    const graphDiagnostics = diagnostics.filter((diagnostic) => !diagnostic.node_id && !diagnostic.edge)
    return toFieldDiagnosticMap(graphDiagnostics, mapGraphDiagnosticFields)
}
