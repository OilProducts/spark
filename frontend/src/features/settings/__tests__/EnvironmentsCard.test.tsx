import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { normalizeOrigin } from '@/lib/environments'
import { EnvironmentsCard } from '../EnvironmentsCard'

const { isTauri, openUrl } = vi.hoisted(() => ({ isTauri: vi.fn(() => false), openUrl: vi.fn().mockResolvedValue(undefined) }))
vi.mock('@tauri-apps/api/core', () => ({ isTauri }))
vi.mock('@tauri-apps/plugin-opener', () => ({ openUrl }))
afterEach(() => { vi.clearAllMocks(); vi.unstubAllGlobals(); isTauri.mockReturnValue(false) })

const saved = { origin: 'http://127.0.0.1:9000', expected_id: 'env-b', label: 'Build box' }
const stored = () => JSON.parse(localStorage.getItem('spark.environments') ?? 'null')
const descriptor = (environment_id: string) => vi.fn().mockResolvedValue(new Response(JSON.stringify({ environment_id, display_name: 'B', protocol_version: 1 })))

it('accepts only bare http(s) origins', () => {
    expect(normalizeOrigin(' HTTPS://Spark.Example:443/ ')).toBe('https://spark.example')
    expect(normalizeOrigin('http://127.0.0.1:9000')).toBe('http://127.0.0.1:9000')
    for (const bad of ['ftp://host', 'javascript:alert(1)', 'http://user:pw@host', 'http://host/path', 'http://host/?q=1', 'http://host/#x', 'host:9000', '']) {
        expect(normalizeOrigin(bad), bad).toBeNull()
    }
})

it('adds an entry from the keyboard, persists it, and rejects a bad origin with an alert', async () => {
    const user = userEvent.setup()
    render(<EnvironmentsCard />)
    await user.type(screen.getByLabelText('Origin'), 'http://127.0.0.1:9000/path')
    await user.type(screen.getByLabelText('Expected environment ID'), 'env-b{Enter}')
    expect(screen.getByRole('alert')).toHaveTextContent(/no path, query or credentials/)
    expect(stored()).toBeNull()

    await user.clear(screen.getByLabelText('Origin'))
    await user.type(screen.getByLabelText('Origin'), 'http://127.0.0.1:9000/')
    await user.type(screen.getByLabelText('Label (optional)'), 'Build box{Enter}')
    expect(stored()).toEqual([saved])
    expect(screen.queryByRole('alert')).toBeNull()
    expect(screen.getByLabelText('Origin')).toHaveValue('')
    expect(screen.getByRole('button', { name: 'Open Build box' })).toBeInTheDocument()
})

it('removes an entry', async () => {
    const user = userEvent.setup()
    localStorage.setItem('spark.environments', JSON.stringify([saved, { origin: 'http://127.0.0.1:9001', expected_id: 'env-c', label: '' }]))
    render(<EnvironmentsCard />)
    await user.click(screen.getByRole('button', { name: 'Remove Build box' }))
    expect(stored()).toEqual([{ origin: 'http://127.0.0.1:9001', expected_id: 'env-c', label: '' }])
    expect(screen.queryByText('Build box')).toBeNull()
    // An unlabelled entry is named by its origin.
    expect(screen.getByRole('button', { name: 'Open http://127.0.0.1:9001' })).toBeInTheDocument()
})

it('refuses to open an origin that reports another environment or cannot be reached', async () => {
    const user = userEvent.setup()
    const navigate = vi.fn()
    localStorage.setItem('spark.environments', JSON.stringify([saved]))
    vi.stubGlobal('fetch', descriptor('env-other'))
    render(<EnvironmentsCard navigate={navigate} />)
    await user.click(screen.getByRole('button', { name: 'Open Build box' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('http://127.0.0.1:9000 is not environment env-b (it reports env-other). It was not opened.')
    vi.stubGlobal('fetch', vi.fn().mockRejectedValue(new TypeError('Failed to fetch')))
    await user.click(screen.getByRole('button', { name: 'Open Build box' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('Could not check http://127.0.0.1:9000.')
    expect(navigate).not.toHaveBeenCalled()
    expect(openUrl).not.toHaveBeenCalled()
})

it('opens a verified origin in this tab from the keyboard', async () => {
    const user = userEvent.setup()
    const navigate = vi.fn()
    const fetch = descriptor('env-b')
    localStorage.setItem('spark.environments', JSON.stringify([saved]))
    vi.stubGlobal('fetch', fetch)
    render(<EnvironmentsCard navigate={navigate} />)
    await user.tab()
    expect(screen.getByRole('button', { name: 'Open Build box' })).toHaveFocus()
    await user.keyboard('{Enter}')
    expect(fetch).toHaveBeenCalledWith('http://127.0.0.1:9000/environment', { credentials: 'omit', redirect: 'error' })
    expect(navigate).toHaveBeenCalledWith('http://127.0.0.1:9000/')
    expect(openUrl).not.toHaveBeenCalled()
    expect(screen.queryByRole('alert')).toBeNull()
})

it('opens a verified origin in the system browser from the desktop', async () => {
    const user = userEvent.setup()
    const navigate = vi.fn()
    isTauri.mockReturnValue(true)
    localStorage.setItem('spark.environments', JSON.stringify([saved]))
    vi.stubGlobal('fetch', descriptor('env-b'))
    render(<EnvironmentsCard navigate={navigate} />)
    expect(screen.getByText(/saved on this device only/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Open Build box' }))
    expect(openUrl).toHaveBeenCalledWith('http://127.0.0.1:9000/')
    expect(navigate).not.toHaveBeenCalled()
    expect(within(screen.getByRole('status')).getByText('Opened Build box in your browser.')).toBeInTheDocument()
})
