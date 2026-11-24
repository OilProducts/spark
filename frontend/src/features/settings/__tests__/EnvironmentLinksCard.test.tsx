import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { beforeEach, expect, it, vi } from 'vitest'
import { EnvironmentLinksCard } from '../EnvironmentLinksCard'
import {
    addEnvironmentLink,
    createPairingCode,
    fetchEnvironmentDescriptor,
    fetchEnvironmentLinks,
    fetchLinkProjects,
    fetchPeerTokens,
    removeEnvironmentLink,
    revokePeerToken,
} from '@/lib/api/environmentsApi'

vi.mock('@/lib/api/environmentsApi', () => ({
    fetchEnvironmentDescriptor: vi.fn(),
    fetchPeerTokens: vi.fn(),
    fetchEnvironmentLinks: vi.fn(),
    createPairingCode: vi.fn(),
    revokePeerToken: vi.fn(),
    addEnvironmentLink: vi.fn(),
    removeEnvironmentLink: vi.fn(),
    fetchLinkProjects: vi.fn(),
}))

beforeEach(() => {
    vi.resetAllMocks()
    vi.mocked(fetchEnvironmentDescriptor).mockResolvedValue({
        environment_id: 'env-local-1',
        display_name: 'mac-studio',
        protocol_version: 1,
        ui_api_version: 1,
        spark_version: '0.1.0',
    })
    vi.mocked(fetchPeerTokens).mockResolvedValue([
        { token_id: 'tok-1', label: 'Laptop', created_at: '2026-11-23T00:00:00Z', revoked_at: null },
    ])
    vi.mocked(fetchEnvironmentLinks).mockResolvedValue([
        { environment_id: 'env-build', origin: 'https://build.example:8000', label: 'Build box', created_at: '2026-11-23T00:00:00Z' },
    ])
})

it('shows this environment, creates a pairing code, revokes a token, and inspects/manages links', async () => {
    const user = userEvent.setup()
    vi.mocked(createPairingCode).mockResolvedValue({
        environment_id: 'env-local-1',
        code: 'secret-code-123',
        expires_at: '2026-11-23T00:10:00Z',
    })
    vi.mocked(revokePeerToken).mockResolvedValue(undefined)
    vi.mocked(fetchLinkProjects).mockResolvedValue(['/srv/app', '/srv/worker'])
    vi.mocked(addEnvironmentLink).mockResolvedValue({
        environment_id: 'env-gpu',
        origin: 'https://gpu.example:8000',
        label: 'GPU',
        created_at: '2026-11-23T00:01:00Z',
    })
    vi.mocked(removeEnvironmentLink).mockResolvedValue(undefined)

    render(<EnvironmentLinksCard />)

    expect(await screen.findByText('env-local-1')).toBeInTheDocument()
    expect(screen.getByText('Laptop')).toBeInTheDocument()
    expect(screen.getByText('Build box')).toBeInTheDocument()

    await user.type(screen.getByLabelText('Token label (optional)'), 'CI runner')
    await user.click(screen.getByRole('button', { name: 'Create pairing code' }))
    expect(createPairingCode).toHaveBeenCalledWith('CI runner')
    expect(await screen.findByTestId('pairing-code-value')).toHaveTextContent('secret-code-123')

    await user.click(screen.getByRole('button', { name: 'Projects on Build box' }))
    expect(fetchLinkProjects).toHaveBeenCalledWith('env-build')
    expect(await screen.findByText('/srv/app')).toBeInTheDocument()
    expect(screen.getByText('/srv/worker')).toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Revoke Laptop' }))
    expect(revokePeerToken).toHaveBeenCalledWith('tok-1')

    await user.type(screen.getByLabelText('Remote origin'), 'https://gpu.example:8000')
    await user.type(screen.getByLabelText('Remote environment ID'), 'env-gpu')
    await user.type(screen.getByLabelText('Pairing code'), 'code-xyz')
    await user.type(screen.getByLabelText('Link label (optional)'), 'GPU')
    await user.click(screen.getByRole('button', { name: 'Add link' }))
    expect(addEnvironmentLink).toHaveBeenCalledWith({
        origin: 'https://gpu.example:8000',
        expected_id: 'env-gpu',
        code: 'code-xyz',
        label: 'GPU',
    })

    await user.click(screen.getByRole('button', { name: 'Remove link Build box' }))
    expect(removeEnvironmentLink).toHaveBeenCalledWith('env-build')
})
