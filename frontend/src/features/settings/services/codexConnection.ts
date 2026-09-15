import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'
import { ApiSchemaError, expectObjectRecord, expectString } from '@/lib/api/shared'

export type CodexConnection = {
    status: 'connected' | 'disconnected' | 'pending'
    account: { kind: string; email: string | null; plan: string | null } | null
    login_url: string | null
    user_code: string | null
    message: string | null
}

export function parseCodexConnection(payload: unknown, endpoint: string): CodexConnection {
    const record = expectObjectRecord(payload, endpoint)
    const status = record.status
    if (status !== 'connected' && status !== 'disconnected' && status !== 'pending') {
        throw new ApiSchemaError(endpoint, 'Invalid Codex connection status.')
    }
    const nullableText = (value: unknown, field: string) => value === null ? null : expectString(value, endpoint, field)
    const rawAccount = record.account === null ? null : expectObjectRecord(record.account, endpoint)
    const loginUrl = nullableText(record.login_url, 'login_url')
    if (loginUrl !== null) {
        let url: URL
        try { url = new URL(loginUrl) } catch { throw new ApiSchemaError(endpoint, 'Invalid sign-in URL.') }
        if (url.protocol !== 'https:' || url.username || url.password) throw new ApiSchemaError(endpoint, 'Unsafe sign-in URL.')
    }
    if (status === 'pending' && !loginUrl) throw new ApiSchemaError(endpoint, 'Missing sign-in URL.')
    return {
        status,
        account: rawAccount ? {
            kind: expectString(rawAccount.kind, endpoint, 'account.kind'),
            email: nullableText(rawAccount.email, 'account.email'),
            plan: nullableText(rawAccount.plan, 'account.plan'),
        } : null,
        login_url: loginUrl,
        user_code: nullableText(record.user_code, 'user_code'),
        message: nullableText(record.message, 'message'),
    }
}

export function codexConnectionRequest(action: 'status' | 'browser' | 'device' | 'cancel', signal?: AbortSignal) {
    const path = action === 'status' ? '/codex/connection' : '/codex/login'
    return fetchWorkspaceJsonValidated(path, {
        method: action === 'status' ? 'GET' : action === 'cancel' ? 'DELETE' : 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: action === 'browser' || action === 'device' ? JSON.stringify({ method: action }) : undefined,
        cache: 'no-store', signal,
    }, `Codex ${action}`, parseCodexConnection)
}

export function isCodexAuthError(message: string | null | undefined): boolean {
    return !!message && /codex connection needs sign-in|access token could not be refreshed|refresh token was already used/i.test(message)
}
