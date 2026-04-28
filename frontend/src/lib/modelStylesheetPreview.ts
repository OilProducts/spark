export type ModelProperty = 'llm_model' | 'llm_provider' | 'llm_profile' | 'reasoning_effort'
export type ModelValueSource = 'node' | 'stylesheet' | 'graph_default' | 'system_default'

export interface StylesheetPreviewNodeInput {
    id: string
    shape?: string
    class?: string
    llm_model?: string
    llm_provider?: string
    llm_profile?: string
    reasoning_effort?: string
}

export interface StylesheetGraphDefaults {
    llm_model?: string
    llm_provider?: string
    llm_profile?: string
    reasoning_effort?: string
}

export interface EffectivePreviewValue {
    value: string
    source: ModelValueSource
}

export interface SelectorPreviewEntry {
    selector: string
    declarations: Partial<Record<ModelProperty, string>>
    matchedNodeIds: string[]
}

export interface NodePreviewEntry {
    nodeId: string
    classes: string[]
    matchedSelectors: string[]
    effective: Record<ModelProperty, EffectivePreviewValue>
}

export interface ModelStylesheetPreview {
    selectorPreview: SelectorPreviewEntry[]
    nodePreview: NodePreviewEntry[]
}

interface ParsedRule {
    selector: string
    properties: Partial<Record<ModelProperty, string>>
    order: number
}

const ALLOWED_PROPERTIES = new Set<ModelProperty>(['llm_model', 'llm_provider', 'llm_profile', 'reasoning_effort'])
const ALLOWED_REASONING_EFFORTS = new Set(['low', 'medium', 'high', 'xhigh'])
const CLASS_NAME_RE = /^[a-z0-9-]+$/
const NODE_ID_RE = /^[A-Za-z_][A-Za-z0-9_]*$/
const SHAPE_SELECTOR_RE = /^[A-Za-z][A-Za-z0-9_]*$/
const QUOTED_VALUE_RE = /^"(?:[^"\\]|\\.)+"$/
const SYSTEM_DEFAULTS: Record<ModelProperty, string> = {
    llm_model: '',
    llm_provider: '',
    llm_profile: '',
    reasoning_effort: 'high',
}
const MODEL_PROPERTIES: ModelProperty[] = ['llm_model', 'llm_provider', 'llm_profile', 'reasoning_effort']

export function resolveModelStylesheetPreview(
    stylesheet: string,
    nodes: StylesheetPreviewNodeInput[],
    graphDefaults: StylesheetGraphDefaults,
): ModelStylesheetPreview {
    const rules = parseRules(stylesheet)

    const selectorPreview: SelectorPreviewEntry[] = rules.map((rule) => ({
        selector: rule.selector,
        declarations: rule.properties,
        matchedNodeIds: nodes.filter((node) => selectorMatches(rule.selector, node)).map((node) => node.id),
    }))

    const nodePreview: NodePreviewEntry[] = nodes.map((node) => {
        const candidates: Partial<Record<ModelProperty, { specificity: number, order: number, value: string }>> = {}
        const matchedSelectors: string[] = []

        rules.forEach((rule) => {
            if (!selectorMatches(rule.selector, node)) {
                return
            }
            matchedSelectors.push(rule.selector)
            const specificity = selectorSpecificity(rule.selector)
            MODEL_PROPERTIES.forEach((property) => {
                const value = rule.properties[property]
                if (value === undefined) {
                    return
                }
                const current = candidates[property]
                if (!current || specificity > current.specificity || (specificity === current.specificity && rule.order > current.order)) {
                    candidates[property] = { specificity, order: rule.order, value }
                }
            })
        })

        const effective: Record<ModelProperty, EffectivePreviewValue> = {
            llm_model: resolveEffectiveValue('llm_model', node, candidates, graphDefaults),
            llm_provider: resolveEffectiveValue('llm_provider', node, candidates, graphDefaults),
            llm_profile: resolveEffectiveValue('llm_profile', node, candidates, graphDefaults),
            reasoning_effort: resolveEffectiveValue('reasoning_effort', node, candidates, graphDefaults),
        }

        return {
            nodeId: node.id,
            classes: nodeClasses(node),
            matchedSelectors,
            effective,
        }
    })

    return {
        selectorPreview,
        nodePreview,
    }
}

function resolveEffectiveValue(
    property: ModelProperty,
    node: StylesheetPreviewNodeInput,
    candidates: Partial<Record<ModelProperty, { specificity: number, order: number, value: string }>>,
    graphDefaults: StylesheetGraphDefaults,
): EffectivePreviewValue {
    const explicitNodeValue = normalizeValue(node[property])
    if (explicitNodeValue) {
        return { value: explicitNodeValue, source: 'node' }
    }

    const candidate = candidates[property]
    if (candidate) {
        return { value: candidate.value, source: 'stylesheet' }
    }

    const graphDefaultValue = normalizeValue(graphDefaults[property])
    if (graphDefaultValue) {
        return { value: graphDefaultValue, source: 'graph_default' }
    }

    return { value: SYSTEM_DEFAULTS[property], source: 'system_default' }
}

function parseRules(stylesheet: string): ParsedRule[] {
    const text = stylesheet.trim()
    const rules: ParsedRule[] = []
    let idx = 0
    let order = 0

    while (idx < text.length) {
        while (idx < text.length && /\s/.test(text[idx])) {
            idx += 1
        }
        if (idx >= text.length) {
            break
        }

        const openBrace = findUnquoted(text, '{', idx)
        if (openBrace === -1) {
            break
        }

        const selector = text.slice(idx, openBrace).trim()
        const closeBrace = findUnquoted(text, '}', openBrace + 1)
        if (closeBrace === -1) {
            break
        }

        const body = text.slice(openBrace + 1, closeBrace).trim()
        const parsedProperties = parseProperties(body)
        if (isValidSelector(selector) && parsedProperties) {
            rules.push({
                selector,
                properties: parsedProperties,
                order,
            })
        }

        order += 1
        idx = closeBrace + 1
    }

    return rules
}

function parseProperties(body: string): Partial<Record<ModelProperty, string>> | null {
    const properties: Partial<Record<ModelProperty, string>> = {}
    const statements = splitUnquoted(body, ';')

    for (const statement of statements) {
        const trimmed = statement.trim()
        if (!trimmed) {
            continue
        }

        if (countUnquoted(trimmed, ':') !== 1) {
            return null
        }

        const colonIndex = findUnquoted(trimmed, ':')
        if (colonIndex === -1) {
            return null
        }

        const key = trimmed.slice(0, colonIndex).trim() as ModelProperty
        const rawValue = trimmed.slice(colonIndex + 1).trim()
        const value = parseValue(rawValue)
        if (!ALLOWED_PROPERTIES.has(key) || value === null || value === '') {
            return null
        }
        if (key === 'reasoning_effort' && !ALLOWED_REASONING_EFFORTS.has(value)) {
            return null
        }

        properties[key] = value
    }

    return Object.keys(properties).length > 0 ? properties : null
}

function parseValue(value: string): string | null {
    if (value.startsWith('"') || value.endsWith('"')) {
        if (!QUOTED_VALUE_RE.test(value)) {
            return null
        }
        return unescapeQuotedValue(value.slice(1, -1))
    }

    if (value.includes('"')) {
        return null
    }

    return value
}

function unescapeQuotedValue(value: string): string {
    const output: string[] = []
    let idx = 0

    while (idx < value.length) {
        const char = value[idx]
        if (char !== '\\') {
            output.push(char)
            idx += 1
            continue
        }

        if (idx + 1 >= value.length) {
            output.push('\\')
            idx += 1
            continue
        }

        const escaped = value[idx + 1]
        if (escaped === '"') {
            output.push('"')
        } else if (escaped === '\\') {
            output.push('\\')
        } else if (escaped === 'n') {
            output.push('\n')
        } else if (escaped === 't') {
            output.push('\t')
        } else {
            output.push('\\')
            output.push(escaped)
        }

        idx += 2
    }

    return output.join('')
}

function findUnquoted(text: string, token: string, start = 0): number {
    let inQuotes = false
    let escaped = false

    for (let idx = start; idx < text.length; idx += 1) {
        const char = text[idx]
        if (char === '\\' && inQuotes && !escaped) {
            escaped = true
            continue
        }
        if (char === '"' && !escaped) {
            inQuotes = !inQuotes
        } else if (char === token && !inQuotes) {
            return idx
        }
        escaped = false
    }

    return -1
}

function countUnquoted(text: string, token: string): number {
    let count = 0
    let inQuotes = false
    let escaped = false

    for (let idx = 0; idx < text.length; idx += 1) {
        const char = text[idx]
        if (char === '\\' && inQuotes && !escaped) {
            escaped = true
            continue
        }
        if (char === '"' && !escaped) {
            inQuotes = !inQuotes
        } else if (char === token && !inQuotes) {
            count += 1
        }
        escaped = false
    }

    return count
}

function splitUnquoted(text: string, token: string): string[] {
    const parts: string[] = []
    let start = 0
    let inQuotes = false
    let escaped = false

    for (let idx = 0; idx < text.length; idx += 1) {
        const char = text[idx]
        if (char === '\\' && inQuotes && !escaped) {
            escaped = true
            continue
        }
        if (char === '"' && !escaped) {
            inQuotes = !inQuotes
        } else if (char === token && !inQuotes) {
            parts.push(text.slice(start, idx))
            start = idx + 1
        }
        escaped = false
    }

    parts.push(text.slice(start))
    return parts
}

function isValidSelector(selector: string): boolean {
    if (selector === '*') {
        return true
    }
    if (selector.startsWith('.')) {
        return CLASS_NAME_RE.test(selector.slice(1))
    }
    if (selector.startsWith('#')) {
        return NODE_ID_RE.test(selector.slice(1))
    }
    return SHAPE_SELECTOR_RE.test(selector)
}

function selectorMatches(selector: string, node: StylesheetPreviewNodeInput): boolean {
    if (selector === '*') {
        return true
    }
    if (selector.startsWith('#')) {
        return node.id === selector.slice(1)
    }
    if (selector.startsWith('.')) {
        const className = selector.slice(1)
        return nodeClasses(node).includes(className)
    }
    return normalizeValue(node.shape).toLowerCase() === selector.trim().toLowerCase()
}

function selectorSpecificity(selector: string): number {
    if (selector.startsWith('#')) {
        return 3
    }
    if (selector.startsWith('.')) {
        return 2
    }
    if (SHAPE_SELECTOR_RE.test(selector)) {
        return 1
    }
    if (selector === '*') {
        return 0
    }
    return -1
}

function nodeClasses(node: StylesheetPreviewNodeInput): string[] {
    return normalizeValue(node.class)
        .split(',')
        .map((item) => item.trim())
        .filter((item) => item.length > 0)
}

function normalizeValue(value: string | undefined): string {
    return (value || '').trim()
}
