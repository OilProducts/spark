import { render, screen } from '@testing-library/react'
import { ReactFlowProvider } from '@xyflow/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it, vi } from 'vitest'

import { useStore } from '@/store'
import { EditorCanvasToolbar } from '../components/EditorCanvasToolbar'

function renderToolbar(overrides: Partial<React.ComponentProps<typeof EditorCanvasToolbar>> = {}) {
    const props: React.ComponentProps<typeof EditorCanvasToolbar> = {
        mode: 'structured',
        modeDisabled: false,
        childFlowsExpanded: false,
        runDisabledReason: null,
        onSelectStructured: vi.fn(),
        onSelectFiles: vi.fn(),
        onSetChildFlowsExpanded: vi.fn(),
        onArrange: vi.fn(),
        onReset: vi.fn(),
        onAddNode: vi.fn(),
        runDefaultProjectPath: null,
        onRun: vi.fn(),
        ...overrides,
    }
    render(<ReactFlowProvider><EditorCanvasToolbar {...props} /></ReactFlowProvider>)
    return props
}

describe('EditorCanvasToolbar', () => {
    it('exposes active view choices and keyboard-operable grouped actions', async () => {
        const user = userEvent.setup()
        const props = renderToolbar()

        expect(screen.getByRole('button', { name: 'Structured' })).toHaveAttribute('aria-pressed', 'true')
        expect(screen.getByRole('button', { name: 'Files' })).toHaveAttribute('aria-pressed', 'false')
        expect(screen.getByRole('button', { name: 'Parent' })).toHaveAttribute('aria-pressed', 'true')
        expect(screen.getByRole('group', { name: 'Layout' })).toBeInTheDocument()
        expect(screen.getByRole('group', { name: 'Actions' })).toBeInTheDocument()
        // The view controls sit in the toolbar, not over the canvas; the editable canvas can be locked.
        const view = screen.getByRole('group', { name: 'Zoom' })
        expect(view).toContainElement(screen.getByRole('button', { name: 'Fit whole graph' }))
        expect(view).toContainElement(screen.getByRole('button', { name: 'Lock canvas' }))

        screen.getByRole('button', { name: 'Auto-arrange layout' }).focus()
        await user.keyboard('{Enter}')
        expect(props.onArrange).toHaveBeenCalledOnce()
    })

    it('wraps the docked toolbar rather than scroll actions away, and hides editing actions in expanded mode', () => {
        renderToolbar({ childFlowsExpanded: true, runDisabledReason: 'Save pending' })

        expect(screen.getByLabelText('Canvas toolbar')).toHaveClass('flex-wrap')
        expect(screen.getByLabelText('Canvas toolbar')).not.toHaveClass('overflow-x-auto')
        expect(screen.getByRole('group', { name: 'Child flows' })).toBeInTheDocument()
        expect(screen.queryByRole('group', { name: 'Layout' })).not.toBeInTheDocument()
        expect(screen.queryByRole('button', { name: '+ Node' })).not.toBeInTheDocument()
        expect(screen.getByRole('button', { name: 'Run…' })).toBeDisabled()
        expect(screen.getByRole('button', { name: 'Run…' })).toHaveAttribute('title', 'Save pending')
    })

    it('shows only the mode choices in the file view', async () => {
        const user = userEvent.setup()
        const props = renderToolbar({ mode: 'files' })

        expect(screen.getByRole('button', { name: 'Files' })).toHaveAttribute('aria-pressed', 'true')
        await user.click(screen.getByRole('button', { name: 'Structured' }))
        expect(props.onSelectStructured).toHaveBeenCalledOnce()
        expect(screen.queryByRole('button', { name: 'Parent' })).not.toBeInTheDocument()
        expect(screen.queryByRole('group', { name: 'Actions' })).not.toBeInTheDocument()
    })

    it('asks which project to run in, suggesting the default first', async () => {
        useStore.setState({ projectRegistry: {
            '/home/me': { directoryPath: '/home/me', displayName: 'Home', isDefault: true, isFavorite: false, lastAccessedAt: null },
            '/work/app': { directoryPath: '/work/app', isFavorite: false, lastAccessedAt: '2026-10-01T00:00:00Z' },
        } })
        const user = userEvent.setup()
        const props = renderToolbar({ runDefaultProjectPath: '/work/app' })

        await user.click(screen.getByRole('button', { name: 'Run…' }))
        const items = await screen.findAllByTestId('project-picker-item')
        expect(items.map((item) => item.textContent)).toEqual(['appsuggested', 'Homedefault'])
        await user.click(items[1])
        expect(props.onRun).toHaveBeenCalledWith('/home/me')
    })
})
