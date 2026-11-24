import { fetchWorkspaceJsonValidated } from './apiClient'
import {
    activeApiPrefix,
    environmentProxyPrefix,
    parseEnvironmentDescriptor,
    parseEnvironmentLink,
    parseEnvironmentLinks,
    type EnvironmentDescriptor,
    type EnvironmentLink,
} from '../activeEnvironment'
import {
    ApiSchemaError,
    asOptionalNullableString,
    asOptionalString,
    asUnknownRecord,
    expectObjectRecord,
    expectString,
    fetchJsonWithValidation,
} from './shared'

export { environmentProxyPrefix, type EnvironmentDescriptor, type EnvironmentLink }

export type PairingCodeReceipt = {
    environment_id: string
    code: string
    expires_at: string
}

export type PeerToken = {
    token_id: string
    label: string
    created_at: string
    revoked_at: string | null
    /** The token authenticating the request that listed it. */
    is_current: boolean
}

export type DelegationQuestionOption = {
    label: string
    value?: string
    description?: string
}

export type DelegationSubQuestion = {
    id: string
    header?: string
    question: string
    options?: DelegationQuestionOption[]
}

export type DelegationQuestion = {
    question_id: string
    prompt?: string
    options?: DelegationQuestionOption[]
    questions?: DelegationSubQuestion[]
    form?: Record<string, unknown>
}

export type DelegationLastState = {
    phase?: string
    status?: string
    target_kind?: string
    target_id?: string
    questions: DelegationQuestion[]
}

export type DelegationOut = {
    delegation_id: string
    environment_id: string
    project_path: string
    kind: string
    request_key: string
    phase: string
    target_id: string | null
    paused: string | null
    cancel_requested_at: string | null
    created_at: string
    updated_at: string
    last_state: DelegationLastState | null
    result: Record<string, unknown> | null
}

export type DelegationAnswerBody = {
    selected_value?: string
    answers?: Record<string, string>
    form?: { action: 'accept' | 'decline' | 'cancel'; content?: Record<string, unknown> }
}

function parsePairingCode(payload: unknown, endpoint: string): PairingCodeReceipt {
    const record = expectObjectRecord(payload, endpoint)
    return {
        environment_id: asOptionalString(record.environment_id) ?? '',
        code: expectString(record.code, endpoint, 'code'),
        expires_at: expectString(record.expires_at, endpoint, 'expires_at'),
    }
}

function parsePeerTokens(payload: unknown, endpoint: string): PeerToken[] {
    const record = expectObjectRecord(payload, endpoint)
    const items = Array.isArray(record.tokens) ? record.tokens : []
    return items.map((item) => {
        const row = expectObjectRecord(item, endpoint)
        return {
            token_id: expectString(row.token_id, endpoint, 'token_id'),
            label: asOptionalString(row.label) ?? '',
            created_at: asOptionalString(row.created_at) ?? '',
            revoked_at: asOptionalNullableString(row.revoked_at) ?? null,
            is_current: row.is_current === true,
        }
    })
}

/** A peer relays its `GET /workspace/api/projects`: an array of project records. */
function parseLinkProjects(payload: unknown, endpoint: string): string[] {
    if (!Array.isArray(payload)) {
        throw new ApiSchemaError(endpoint, 'Expected an array of projects.')
    }
    return payload.map((item) => expectString(expectObjectRecord(item, endpoint).project_path, endpoint, 'project_path'))
}

function parseOptions(value: unknown): DelegationQuestionOption[] | undefined {
    if (!Array.isArray(value)) return undefined
    const options: DelegationQuestionOption[] = []
    for (const item of value) {
        const record = asUnknownRecord(item)
        const label = asOptionalString(record?.label) ?? asOptionalString(record?.value)
        if (!label) continue
        options.push({
            label,
            value: asOptionalString(record?.value),
            description: asOptionalString(record?.description),
        })
    }
    return options.length > 0 ? options : undefined
}

function parseQuestions(value: unknown): DelegationQuestion[] {
    if (!Array.isArray(value)) return []
    const questions: DelegationQuestion[] = []
    for (const item of value) {
        const record = asUnknownRecord(item)
        const question_id = asOptionalString(record?.question_id)
        if (!question_id) continue
        const subQuestions = Array.isArray(record?.questions)
            ? record.questions
                .map((entry) => asUnknownRecord(entry))
                .filter((entry): entry is Record<string, unknown> => Boolean(entry && typeof entry.id === 'string'))
                .map((entry) => ({
                    id: String(entry.id),
                    header: asOptionalString(entry.header),
                    question: asOptionalString(entry.question) ?? String(entry.id),
                    options: parseOptions(entry.options),
                }))
            : undefined
        questions.push({
            question_id,
            prompt: asOptionalString(record?.prompt) ?? asOptionalString(record?.text),
            options: parseOptions(record?.options),
            questions: subQuestions && subQuestions.length > 0 ? subQuestions : undefined,
            form: asUnknownRecord(record?.form) ?? undefined,
        })
    }
    return questions
}

function parseDelegation(payload: unknown, endpoint: string): DelegationOut {
    const row = expectObjectRecord(payload, endpoint)
    const lastRecord = asUnknownRecord(row.last_state)
    const last_state: DelegationLastState | null = lastRecord
        ? {
            phase: asOptionalString(lastRecord.phase),
            status: asOptionalString(lastRecord.status),
            target_kind: asOptionalString(lastRecord.target_kind),
            target_id: asOptionalString(lastRecord.target_id),
            questions: parseQuestions(lastRecord.questions),
        }
        : null
    return {
        delegation_id: expectString(row.delegation_id, endpoint, 'delegation_id'),
        environment_id: expectString(row.environment_id, endpoint, 'environment_id'),
        project_path: asOptionalString(row.project_path) ?? '',
        kind: asOptionalString(row.kind) ?? '',
        request_key: asOptionalString(row.request_key) ?? '',
        phase: asOptionalString(row.phase) ?? 'created',
        target_id: asOptionalNullableString(row.target_id) ?? null,
        paused: asOptionalNullableString(row.paused) ?? null,
        cancel_requested_at: asOptionalNullableString(row.cancel_requested_at) ?? null,
        created_at: asOptionalString(row.created_at) ?? '',
        updated_at: asOptionalString(row.updated_at) ?? '',
        last_state,
        result: asUnknownRecord(row.result),
    }
}

function parseDelegations(payload: unknown, endpoint: string): DelegationOut[] {
    const record = expectObjectRecord(payload, endpoint)
    const items = Array.isArray(record.delegations) ? record.delegations : []
    return items.map((item) => parseDelegation(item, endpoint))
}

export function fetchEnvironmentDescriptor(environmentId?: string | null): Promise<EnvironmentDescriptor> {
    const prefix = environmentId === undefined ? activeApiPrefix() : environmentProxyPrefix(environmentId)
    return fetchJsonWithValidation(`${prefix}/environment`, undefined, `GET ${prefix}/environment`, parseEnvironmentDescriptor)
}

export function createPairingCode(label: string): Promise<PairingCodeReceipt> {
    return fetchWorkspaceJsonValidated(
        '/environment/pairing-codes',
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(label.trim() ? { label: label.trim() } : {}),
        },
        'POST /workspace/api/environment/pairing-codes',
        parsePairingCode,
    )
}

export function fetchPeerTokens(): Promise<PeerToken[]> {
    return fetchWorkspaceJsonValidated('/environment/tokens', undefined, 'GET /workspace/api/environment/tokens', parsePeerTokens)
}

export function revokePeerToken(tokenId: string): Promise<void> {
    return fetchWorkspaceJsonValidated(
        `/environment/tokens/${encodeURIComponent(tokenId)}/revoke`,
        { method: 'POST' },
        'POST /workspace/api/environment/tokens/:id/revoke',
        () => undefined,
    )
}

export function fetchEnvironmentLinks(): Promise<EnvironmentLink[]> {
    return fetchWorkspaceJsonValidated('/environments/links', undefined, 'GET /workspace/api/environments/links', parseEnvironmentLinks)
}

export function addEnvironmentLink(input: { origin: string; expected_id: string; code: string; label?: string }): Promise<EnvironmentLink> {
    return fetchWorkspaceJsonValidated(
        '/environments/links',
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({
                origin: input.origin.trim(),
                expected_id: input.expected_id.trim(),
                code: input.code.trim(),
                label: input.label?.trim() ?? '',
            }),
        },
        'POST /workspace/api/environments/links',
        parseEnvironmentLink,
    )
}

export function removeEnvironmentLink(environmentId: string): Promise<void> {
    return fetchWorkspaceJsonValidated(
        `/environments/links/${encodeURIComponent(environmentId)}`,
        { method: 'DELETE' },
        'DELETE /workspace/api/environments/links/:id',
        () => undefined,
    )
}

export function fetchLinkProjects(environmentId: string): Promise<string[]> {
    return fetchWorkspaceJsonValidated(
        `/environments/links/${encodeURIComponent(environmentId)}/projects`,
        undefined,
        'GET /workspace/api/environments/links/:id/projects',
        parseLinkProjects,
    )
}

export function fetchDelegations(filter?: { open?: boolean; paused?: boolean; environment_id?: string }): Promise<DelegationOut[]> {
    const params = new URLSearchParams()
    if (filter?.open) params.set('open', 'true')
    if (filter?.paused) params.set('paused', 'true')
    if (filter?.environment_id) params.set('environment_id', filter.environment_id)
    const suffix = params.toString() ? `?${params.toString()}` : ''
    return fetchWorkspaceJsonValidated(`/delegations${suffix}`, undefined, 'GET /workspace/api/delegations', parseDelegations)
}

export function cancelDelegation(delegationId: string): Promise<DelegationOut> {
    return fetchWorkspaceJsonValidated(
        `/delegations/${encodeURIComponent(delegationId)}/cancel`,
        { method: 'POST' },
        'POST /workspace/api/delegations/:id/cancel',
        parseDelegation,
    )
}

export function answerDelegationQuestion(delegationId: string, questionId: string, body: DelegationAnswerBody): Promise<void> {
    return fetchWorkspaceJsonValidated(
        `/delegations/${encodeURIComponent(delegationId)}/questions/${encodeURIComponent(questionId)}/answer`,
        {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
        },
        'POST /workspace/api/delegations/:id/questions/:qid/answer',
        () => undefined,
    )
}
