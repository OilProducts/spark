import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { DialogProvider } from '@/components/app/dialog-controller'
import { fetchPipelineCancelValidated } from '@/lib/attractorClient'

import { useRunActions } from '../hooks/useRunActions'

vi.mock('@/lib/attractorClient', async (importOriginal) => ({
    ...await importOriginal<typeof import('@/lib/attractorClient')>(),
    fetchPipelineCancelValidated: vi.fn().mockResolvedValue({}),
}))

function CancelButton({ status }: { status: string }) {
    const { requestCancel } = useRunActions()
    return <button type="button" onClick={() => void requestCancel('run-1', status)}>Cancel</button>
}

describe('useRunActions', () => {
    it('cancels a run waiting on a question once confirmed, saying what happens to the question', async () => {
        const user = userEvent.setup()
        render(<DialogProvider><CancelButton status="waiting" /></DialogProvider>)
        await user.click(screen.getByRole('button', { name: 'Cancel' }))
        expect(await screen.findByText('Its question closes unanswered and the run stops.')).toBeVisible()
        await user.click(screen.getByRole('button', { name: 'Cancel run' }))
        await waitFor(() => expect(fetchPipelineCancelValidated).toHaveBeenCalledWith('run-1'))
    })
})
