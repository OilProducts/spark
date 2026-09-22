import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, expect, it, vi } from 'vitest'
import { DialogProvider } from '@/components/app/dialog-controller'
import { RuntimeSettingsEditor } from '../RuntimeSettingsEditor'

afterEach(() => { cleanup(); vi.unstubAllGlobals() })
it('starts a replacement draft for invalid stored field types and requires explicit Save', async () => {
    const defaults = { runs_dir:null, flows_dir:null, ui_dir:null, project_roots:[] }
    let view: Record<string, unknown> = {scope:'workspace', revision:'one', stored:{project_roots:42}, effective:null,
        repair_defaults:defaults, restart_fields:[], validation_errors:['Invalid runtime section.']}
    const writes: Record<string, unknown>[] = []
    vi.stubGlobal('fetch', vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
        if (init?.method === 'PATCH') {
            const body = JSON.parse(String(init.body)) as Record<string, unknown>
            writes.push(body)
            view = {...view, revision:'two', stored:body.value, effective:defaults, validation_errors:[]}
        }
        return new Response(JSON.stringify({runtime:view}), {status:200, headers:{'Content-Type':'application/json'}})
    }))
    const user = userEvent.setup()
    render(<DialogProvider><RuntimeSettingsEditor /></DialogProvider>)
    await screen.findByText('Invalid runtime section.')
    await user.click(screen.getByRole('button', {name:'Start replacement draft with defaults'}))
    expect(writes).toHaveLength(0)
    await user.type(screen.getByLabelText('Flows directory'), '/replacement')
    await user.click(screen.getByRole('button', {name: /^Save/}))
    await waitFor(() => expect(writes).toHaveLength(1))
    expect(writes[0]).toEqual({expected_revision:'one',section:'runtime',value:{...defaults,flows_dir:'/replacement'}})
})

it.each([42, 'missing-profile'])('retains the revision and scoped error for stored execution selection %s until explicit repair', async (stored) => {
    const { ProjectSettingsDialog } = await import('@/app/ProjectSettingsDialog')
    const writes: unknown[] = []
    vi.stubGlobal('fetch', vi.fn(async (_input: RequestInfo | URL, init?: RequestInit) => {
        if (init?.method === 'PATCH') {
            writes.push(JSON.parse(String(init.body)))
            return new Response(JSON.stringify({ detail: 'Settings changed since this document was read. Reload before saving.' }), { status: 409 })
        }
        return new Response(JSON.stringify({
            execution: { revision: 'invalid-selection-revision', stored, effective: null, validation_errors: ['Invalid project execution profile selection.'] },
            execution_placement: {
                config: { filename: 'settings.toml', path: '/settings.toml', loaded: true },
                profiles: [{ id: 'native', enabled: true, mode: 'native' }], validation_errors: [],
            },
        }), { status: 200 })
    }))
    const user = userEvent.setup()
    render(<DialogProvider><ProjectSettingsDialog open projectPath="/project" onOpenChange={vi.fn()} /></DialogProvider>)
    expect(await screen.findByTestId('project-settings-error')).toHaveTextContent('Invalid project execution profile selection.')
    const select = screen.getByTestId('project-default-execution-profile')
    expect(select).toBeEnabled()
    expect(select).toHaveTextContent('Select a replacement or workspace default')
    expect(screen.getByTestId('project-settings-save-button')).toBeDisabled()
    await user.click(select)
    await user.click(screen.getByRole('option', { name: 'native', exact: true }))
    expect(writes).toHaveLength(0)
    await user.click(select)
    await user.click(screen.getByRole('option', { name: 'Use workspace default' }))
    expect(writes).toHaveLength(0)
    await user.click(screen.getByTestId('project-settings-save-button'))
    expect(await screen.findByTestId('project-settings-save-error')).toHaveTextContent('Settings changed')
    expect(writes).toEqual([{ project_path: '/project', expected_revision: 'invalid-selection-revision', execution_profile_id: null }])
    expect(select).toHaveTextContent('Use workspace default')
})
