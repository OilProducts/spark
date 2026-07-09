import type { DiagnosticEntry } from '@/state/store-types'

type UnknownRecord = Record<string, unknown>

export class ApiSchemaError extends Error {
    endpoint: string

    constructor(endpoint: string, message: string) {
        super(`${endpoint}: ${message}`)
        this.name = 'ApiSchemaError'
        this.endpoint = endpoint
    }
}

export class ApiHttpError extends Error {
    endpoint: string
    status: number
    detail: string | null

    constructor(endpoint: string, status: number, detail: string | null) {
        const suffix = detail ? `: ${detail}` : ''
        super(`${endpoint} responded with HTTP ${status}${suffix}`)
        this.name = 'ApiHttpError'
        this.endpoint = endpoint
        this.status = status
        this.detail = detail
    }
}

export function expectObjectRecord(value: unknown, endpoint: string): UnknownRecord {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        throw new ApiSchemaError(endpoint, 'Expected object response payload.')
    }
    return value as UnknownRecord
}

export function expectString(value: unknown, endpoint: string, fieldName: string): string {
    if (typeof value !== 'string') {
        throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be a string.`)
    }
    return value
}

export function asOptionalString(value: unknown): string | undefined {
    if (typeof value !== 'string') {
        return undefined
    }
    return value
}

export function asOptionalNullableString(value: unknown): string | null | undefined {
    if (value === null) {
        return null
    }
    return asOptionalString(value)
}

export function asStringRecord(value: unknown): Record<string, string> | undefined {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return undefined
    }
    const nextRecord: Record<string, string> = {}
    Object.entries(value as UnknownRecord).forEach(([key, entryValue]) => {
        if (typeof entryValue === 'string') {
            nextRecord[key] = entryValue
        }
    })
    return nextRecord
}

export function asOptionalStringArray(value: unknown): string[] | undefined {
    if (!Array.isArray(value)) {
        return undefined
    }
    if (!value.every((entry) => typeof entry === 'string')) {
        return undefined
    }
    return [...value]
}

export function asUnknownRecord(value: unknown): Record<string, unknown> | null {
    if (!value || typeof value !== 'object' || Array.isArray(value)) {
        return null
    }
    return value as Record<string, unknown>
}

export function parseDiagnosticEntry(value: unknown, endpoint: string): DiagnosticEntry | null {
    const record = asUnknownRecord(value)
    if (!record) {
        return null
    }
    const severity = typeof record.severity === 'string' ? record.severity : null
    const message = typeof record.message === 'string' ? record.message : null
    const ruleId = typeof record.rule_id === 'string'
        ? record.rule_id
        : typeof record.rule === 'string'
            ? record.rule
            : null
    if (!severity || !message || !ruleId) {
        return null
    }
    if (severity !== 'error' && severity !== 'warning' && severity !== 'info') {
        throw new ApiSchemaError(endpoint, `Expected diagnostic severity to be error|warning|info; got "${severity}".`)
    }
    const edgeRaw = record.edge
    const edge = Array.isArray(edgeRaw)
        && edgeRaw.length === 2
        && typeof edgeRaw[0] === 'string'
        && typeof edgeRaw[1] === 'string'
        ? [edgeRaw[0], edgeRaw[1]] as [string, string]
        : undefined
    return {
        rule_id: ruleId,
        severity,
        message,
        line: typeof record.line === 'number' ? record.line : undefined,
        node_id: typeof record.node_id === 'string'
            ? record.node_id
            : typeof record.node === 'string'
                ? record.node
                : undefined,
        edge,
        fix: typeof record.fix === 'string' ? record.fix : undefined,
    }
}

export function parseDiagnosticList(value: unknown, endpoint: string, fieldName: string): DiagnosticEntry[] | undefined {
    if (value === undefined) {
        return undefined
    }
    if (!Array.isArray(value)) {
        throw new ApiSchemaError(endpoint, `Expected "${fieldName}" to be an array when present.`)
    }
    return value
        .map((entry) => parseDiagnosticEntry(entry, endpoint))
        .filter((entry): entry is DiagnosticEntry => entry !== null)
}

export function parseHttpErrorDetail(payload: unknown): string | null {
    const record = asUnknownRecord(payload)
    if (!record) {
        return null
    }
    if (typeof record.error === 'string' && record.error.trim().length > 0) {
        return record.error.trim()
    }
    if (typeof record.detail === 'string' && record.detail.trim().length > 0) {
        return record.detail.trim()
    }
    const detailRecord = asUnknownRecord(record.detail)
    if (detailRecord && typeof detailRecord.error === 'string' && detailRecord.error.trim().length > 0) {
        return detailRecord.error.trim()
    }
    return null
}

export async function parseJsonPayload(response: Response, endpoint: string): Promise<unknown> {
    try {
        return await response.json()
    } catch {
        throw new ApiSchemaError(endpoint, 'Expected JSON response body.')
    }
}

export async function extractHttpError(response: Response, endpoint: string): Promise<ApiHttpError> {
    let detail: string | null = null
    try {
        const bodyText = await response.text()
        if (bodyText.trim().length > 0) {
            try {
                const payload = JSON.parse(bodyText) as unknown
                detail = parseHttpErrorDetail(payload)
            } catch {
                detail = bodyText.trim()
            }
        }
    } catch {
        detail = null
    }
    return new ApiHttpError(endpoint, response.status, detail)
}

export async function fetchJsonWithValidation<T>(
    url: string,
    init: RequestInit | undefined,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
): Promise<T> {
    const response = await fetch(url, init)
    if (!response.ok) {
        throw await extractHttpError(response, endpoint)
    }
    const payload = await parseJsonPayload(response, endpoint)
    return parser(payload, endpoint)
}

export async function fetchTextWithValidation<T>(
    url: string,
    init: RequestInit | undefined,
    endpoint: string,
    parser: (payload: unknown, endpoint: string) => T,
): Promise<T> {
    const response = await fetch(url, init)
    if (!response.ok) {
        throw await extractHttpError(response, endpoint)
    }
    const payload = await response.text()
    return parser(payload, endpoint)
}

export { isAbortError } from '../abortError'
