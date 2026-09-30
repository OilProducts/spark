import { fetchWorkspaceJsonValidated } from '@/lib/api/apiClient'
import { ApiSchemaError, expectObjectRecord, expectString } from '@/lib/api/shared'

export type ClaudeCodeConnection = {
    status: 'connected' | 'disconnected' | 'pending'
    account: { kind: string; email: string | null; plan: string | null } | null
    login_url: string | null
    message: string | null
}

export function parseClaudeCodeConnection(payload: unknown, endpoint: string): ClaudeCodeConnection {
    const record = expectObjectRecord(payload, endpoint)
    const status = record.status
    if (status !== 'connected' && status !== 'disconnected' && status !== 'pending') {
        throw new ApiSchemaError(endpoint, 'Invalid Claude Code connection status.')
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
        message: nullableText(record.message, 'message'),
    }
}

export function claudeCodeConnectionRequest(action: 'status' | 'login' | 'cancel', signal?: AbortSignal) {
    return fetchWorkspaceJsonValidated(action === 'status' ? '/claude/connection' : '/claude/login', {
        method: action === 'status' ? 'GET' : action === 'cancel' ? 'DELETE' : 'POST',
        cache: 'no-store', signal,
    }, `Claude Code ${action}`, parseClaudeCodeConnection)
}

export function submitClaudeCodeSignInCode(code: string) {
    return fetchWorkspaceJsonValidated('/claude/login/code', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ code }),
        cache: 'no-store',
    }, 'Claude Code sign-in code', parseClaudeCodeConnection)
}

// Also matches turns that failed before Spark recognized sign-in errors.
export function isClaudeCodeAuthError(message: string | null | undefined): boolean {
    return !!message && /claude code needs sign-in|claude code turn failed \([^)]*\): (failed to authenticate|.*please run \/login)/i.test(message)
}
