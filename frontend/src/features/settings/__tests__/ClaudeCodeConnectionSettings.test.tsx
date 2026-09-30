import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { ClaudeCodeConnectionControls } from '../ClaudeCodeConnectionSettings'
import {
    claudeCodeConnectionRequest, parseClaudeCodeConnection, submitClaudeCodeSignInCode, type ClaudeCodeConnection,
} from '../services/claudeCodeConnection'
import { MessageRow } from '@/components/app/transcript/SegmentRows'

vi.mock('../services/claudeCodeConnection', async (original) => ({
    ...await original<object>(), claudeCodeConnectionRequest: vi.fn(), submitClaudeCodeSignInCode: vi.fn(),
}))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }))
afterEach(() => { cleanup(); vi.resetAllMocks() })

const disconnected: ClaudeCodeConnection = { status: 'disconnected', account: null, login_url: null, message: null }
const pending: ClaudeCodeConnection = { ...disconnected, status: 'pending', login_url: 'https://claude.com/cai/oauth/authorize?code=true' }
const connected: ClaudeCodeConnection = { ...disconnected, status: 'connected', account: { kind: 'claude.ai', email: 'spark@example.test', plan: 'max' } }

it('signs in with one click, accepts a pasted code and clears the sign-in link', async () => {
    const user = userEvent.setup()
    vi.mocked(claudeCodeConnectionRequest).mockResolvedValueOnce(disconnected).mockResolvedValueOnce(pending)
    render(<ClaudeCodeConnectionControls />)
    await screen.findByText('Claude Code needs sign-in.')
    await user.click(screen.getByRole('button', { name: 'Sign in to Claude' }))
    expect(claudeCodeConnectionRequest).toHaveBeenLastCalledWith('login')
    expect(screen.getByRole('link', { name: 'Open the sign-in page' })).toHaveAttribute('href', pending.login_url)
    expect(screen.getByRole('button', { name: 'Submit code' })).toBeDisabled()

    vi.mocked(submitClaudeCodeSignInCode).mockResolvedValueOnce({ ...pending, message: 'Invalid code. Please make sure the full code was copied.' })
    await user.type(screen.getByRole('textbox', { name: 'Sign-in code' }), 'abc')
    await user.click(screen.getByRole('button', { name: 'Submit code' }))
    expect(submitClaudeCodeSignInCode).toHaveBeenLastCalledWith('abc')
    await screen.findByText('Invalid code. Please make sure the full code was copied.')
    expect(screen.getByRole('textbox', { name: 'Sign-in code' })).toHaveValue('')

    vi.mocked(claudeCodeConnectionRequest).mockResolvedValue(connected)
    await screen.findByText(/Signed in as spark@example.test \(max\)/, {}, { timeout: 3000 })
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Sign in again' })).toBeInTheDocument()
})

it('starts sign-in from a failed Claude Code message and keeps the conversation', async () => {
    const user = userEvent.setup()
    vi.mocked(claudeCodeConnectionRequest).mockResolvedValueOnce(disconnected).mockResolvedValue(pending)
    render(<ul>
        <MessageRow entry={{ id: 'user', role: 'user', content: 'Please continue my work.', timestamp: '', status: 'complete' }} formatConversationTimestamp={() => ''} />
        <MessageRow entry={{ id: 'failed', role: 'assistant', content: '', timestamp: '', status: 'failed',
            error: 'claude code turn failed (success): Failed to authenticate: OAuth session expired and could not be refreshed' }} formatConversationTimestamp={() => ''} />
    </ul>)
    expect(screen.getByText('Claude Code needs sign-in. Your conversation is saved.')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Sign in to Claude' }))
    await screen.findByText(/Waiting for sign-in/)
    expect(claudeCodeConnectionRequest).toHaveBeenCalledWith('login')
    await user.click(screen.getByRole('button', { name: 'Cancel sign-in' }))
    expect(claudeCodeConnectionRequest).toHaveBeenLastCalledWith('cancel')
    expect(screen.getByText('Please continue my work.')).toBeInTheDocument()
})

it('leaves other failures alone and rejects unsafe sign-in links', () => {
    render(<ul><MessageRow entry={{ id: 'failed', role: 'assistant', content: '', timestamp: '', status: 'failed',
        error: 'claude code turn failed (error_during_execution): simulated failure' }} formatConversationTimestamp={() => ''} /></ul>)
    expect(screen.queryByRole('button', { name: 'Sign in to Claude' })).not.toBeInTheDocument()
    for (const login_url of ['javascript:alert(1)', 'http://example.test', 'https://user:password@example.test', 'not a url', null]) {
        expect(() => parseClaudeCodeConnection({ ...pending, login_url }, 'test')).toThrow()
    }
    expect(parseClaudeCodeConnection(connected, 'test')).toEqual(connected)
})
