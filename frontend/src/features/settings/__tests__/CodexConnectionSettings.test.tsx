import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { CodexConnectionControls } from '../CodexConnectionSettings'
import { codexConnectionRequest, parseCodexConnection, type CodexConnection } from '../services/codexConnection'
import { MessageRow } from '@/components/app/transcript/SegmentRows'

vi.mock('../services/codexConnection', async (original) => ({ ...await original<object>(), codexConnectionRequest: vi.fn() }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false }))
afterEach(() => { cleanup(); vi.resetAllMocks() })

const disconnected: CodexConnection = { status: 'disconnected', account: null, login_url: null, user_code: null, message: null }
const pending: CodexConnection = { ...disconnected, status: 'pending', login_url: 'https://auth.openai.com/codex/device', user_code: 'ABCD-1234' }
const connected: CodexConnection = { ...disconnected, status: 'connected', account: { kind: 'chatgpt', email: 'spark@example.test', plan: 'plus' } }

it('connects with a device code, polls completion and clears the login link', async () => {
    const user = userEvent.setup()
    vi.mocked(codexConnectionRequest).mockResolvedValueOnce(disconnected).mockResolvedValueOnce(pending).mockResolvedValue(connected)
    render(<CodexConnectionControls />)
    await screen.findByText('Your Codex connection needs sign-in.')
    await user.click(screen.getByRole('button', { name: 'Use a device code' }))
    expect(codexConnectionRequest).toHaveBeenLastCalledWith('device')
    expect(screen.getByText('ABCD-1234')).toBeInTheDocument()
    expect(screen.getByRole('link', { name: /Continue sign-in/ })).toHaveAttribute('href', pending.login_url)
    expect(screen.queryByRole('button', { name: 'Connect Codex' })).not.toBeInTheDocument()
    await screen.findByText(/Connected as spark@example.test/, {}, { timeout: 3000 })
    expect(screen.queryByText('ABCD-1234')).not.toBeInTheDocument()
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
})

it('reattaches to a pending browser login and allows cancellation after a polling error', async () => {
    const user = userEvent.setup()
    vi.mocked(codexConnectionRequest).mockResolvedValueOnce({ ...pending, user_code: null }).mockRejectedValueOnce(new Error('Connection lost.'))
    render(<CodexConnectionControls />)
    await screen.findByText('Waiting for sign-in…')
    await screen.findByRole('alert', {}, { timeout: 3000 })
    expect(screen.getByRole('link', { name: /Continue sign-in/ })).toBeInTheDocument()
    vi.mocked(codexConnectionRequest).mockResolvedValueOnce({ ...disconnected, message: 'Sign-in cancelled. Existing credentials were kept.' })
    await user.click(screen.getByRole('button', { name: 'Cancel sign-in' }))
    expect(codexConnectionRequest).toHaveBeenLastCalledWith('cancel')
    await screen.findByText('Sign-in cancelled. Existing credentials were kept.')
    expect(screen.queryByRole('link')).not.toBeInTheDocument()
})

it('offers reconnect on an existing failed transcript without removing the saved message', async () => {
    const user = userEvent.setup()
    vi.mocked(codexConnectionRequest).mockResolvedValue(disconnected)
    render(<ul>
        <MessageRow entry={{ id: 'user', role: 'user', content: 'Please continue my work.', timestamp: '', status: 'complete' }} formatConversationTimestamp={() => ''} />
        <MessageRow entry={{ id: 'failed', role: 'assistant', content: 'Your access token could not be refreshed because your refresh token was already used.', timestamp: '', status: 'failed' }} formatConversationTimestamp={() => ''} />
    </ul>)
    await user.click(screen.getByRole('button', { name: 'Reconnect Codex' }))
    expect(screen.getByRole('dialog')).toHaveTextContent('Your conversation is saved.')
    await screen.findByRole('button', { name: 'Connect Codex' })
    await user.click(screen.getByRole('button', { name: 'Close' }))
    await waitFor(() => expect(screen.queryByRole('dialog')).not.toBeInTheDocument())
    expect(screen.getByText('Please continue my work.')).toBeInTheDocument()
})

it('does not turn unrelated failures into sign-in prompts and rejects unsafe auth responses', () => {
    render(<ul><MessageRow entry={{ id: 'failed', role: 'assistant', content: 'Rate limit exceeded.', timestamp: '', status: 'failed' }} formatConversationTimestamp={() => ''} /></ul>)
    expect(screen.queryByRole('button', { name: 'Reconnect Codex' })).not.toBeInTheDocument()
    for (const login_url of ['javascript:alert(1)', 'http://example.test', 'https://user:password@example.test', 'not a url', null]) {
        expect(() => parseCodexConnection({ ...pending, login_url }, 'test')).toThrow()
    }
    expect(parseCodexConnection(connected, 'test')).toEqual(connected)
})

it('keeps partial assistant output visible when authentication fails later in the turn', () => {
    render(<ul><MessageRow entry={{ id: 'partial', role: 'assistant', content: 'Finished the first change.',
        error: 'Your Codex connection needs sign-in.', timestamp: '', status: 'failed' }} formatConversationTimestamp={() => ''} /></ul>)
    expect(screen.getByText('Finished the first change.')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Reconnect Codex' })).toBeInTheDocument()
})
