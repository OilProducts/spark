import { TriggersSessionController } from '@/app/AppSessionControllers'
import { TriggersPanel } from '@/features/triggers/TriggersPanel'
import { createEmptyTriggerForm } from '@/features/triggers/model/triggerForm'
import { useStore } from '@/store'
import { act, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const jsonResponse = (payload: unknown) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  })
const TEST_TRIGGER_FLOW = 'test-planning.dot'

const resolveRequestUrl = (input: RequestInfo | URL): string => {
  if (typeof input === 'string') return input
  if (input instanceof URL) return input.toString()
  return input.url
}

const resetTriggerState = () => {
  useStore.setState({
    viewMode: 'triggers',
    activeProjectPath: null,
    projectRegistry: {},
    projectSessionsByPath: {},
    recentProjectPaths: [],
    triggersSession: {
      status: 'idle',
      error: null,
      triggers: [],
      selectedTriggerId: null,
      scopeFilter: 'all',
      revealedWebhookSecrets: {},
      createFormOpen: false,
      newTriggerDraft: {
        form: createEmptyTriggerForm(null),
        targetBehavior: 'default',
      },
      editTriggerDraftsByTriggerId: {},
    },
  })
}

const renderTriggersPanel = () =>
  render(
    <>
      <TriggersSessionController />
      <TriggersPanel />
    </>,
  )

const openCreateTriggerForm = async (user: ReturnType<typeof userEvent.setup>) => {
  await user.click(screen.getByTestId('trigger-new-button'))
  return getCreateTriggerScope()
}

const getCreateTriggerScope = () => {
  const createCard = screen.getByTestId('trigger-create-button').closest('[data-slot="card"]')
  expect(createCard).not.toBeNull()
  return within(createCard as HTMLElement)
}

const makeTrigger = (overrides: Partial<Record<string, unknown>> = {}) => ({
  id: String(overrides.id ?? 'trigger-default'),
  name: String(overrides.name ?? 'Trigger'),
  enabled: overrides.enabled === false ? false : true,
  protected: overrides.protected === true,
  revision: 'revision-1',
  source_type: String(overrides.source_type ?? 'schedule'),
  created_at: '2026-03-22T00:00:00Z',
  updated_at: '2026-03-22T00:00:00Z',
  action: {
    flow_name: String(overrides.flow_name ?? TEST_TRIGGER_FLOW),
    project_path: Object.prototype.hasOwnProperty.call(overrides, 'project_path')
      ? overrides.project_path
      : null,
    static_context: {},
  },
  source: overrides.source ?? { kind: 'interval', interval_seconds: 300 },
  state: {
    last_fired_at: null,
    last_result: null,
    last_error: null,
    next_run_at: '2026-03-22T00:05:00Z',
    recent_history: [],
  },
  webhook_secret: overrides.webhook_secret ?? null,
})

describe('TriggersPanel', () => {
  beforeEach(() => {
    resetTriggerState()
    vi.stubGlobal('fetch', vi.fn())
  })

  afterEach(() => {
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('creates a schedule trigger and refreshes the list', async () => {
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        const payload = fetchMock.mock.calls.filter(
          ([request, requestInit]) => resolveRequestUrl(request as RequestInfo | URL).endsWith('/workspace/api/triggers') && (requestInit?.method ?? 'GET') === 'POST',
        ).length > 0
          ? [
            {
              id: 'trigger-created',
              name: 'Created schedule',
              enabled: true,
              protected: false,
              revision: 'revision-1',
              source_type: 'schedule',
              created_at: '2026-03-22T00:00:00Z',
              updated_at: '2026-03-22T00:00:00Z',
              action: { flow_name: TEST_TRIGGER_FLOW, project_path: null, static_context: {} },
              source: { kind: 'interval', interval_seconds: 300 },
              state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: '2026-03-22T00:05:00Z', recent_history: [] },
            },
          ]
          : []
        return jsonResponse(payload)
      }
      if (url.endsWith('/workspace/api/triggers') && method === 'POST') {
        return jsonResponse({
          id: 'trigger-created',
          name: 'Created schedule',
          enabled: true,
          protected: false,
          revision: 'revision-1',
          source_type: 'schedule',
          created_at: '2026-03-22T00:00:00Z',
          updated_at: '2026-03-22T00:00:00Z',
          action: { flow_name: TEST_TRIGGER_FLOW, project_path: null, static_context: {} },
          source: { kind: 'interval', interval_seconds: 300 },
          state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: '2026-03-22T00:05:00Z', recent_history: [] },
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    const createScope = await openCreateTriggerForm(user)
    expect(within(createScope.getByLabelText('Source Type')).queryByRole('option', { name: 'Workspace Event' })).not.toBeInTheDocument()
    await user.type(createScope.getByLabelText('Name'), 'Created schedule')
    await user.clear(createScope.getByLabelText('Target Flow'))
    await user.type(createScope.getByLabelText('Target Flow'), TEST_TRIGGER_FLOW)
    await user.click(screen.getByTestId('trigger-create-button'))

    await waitFor(() => {
      expect(screen.getByTestId('trigger-row-trigger-created')).toHaveTextContent('Created schedule')
    })
    expect(screen.queryByTestId('trigger-create-button')).not.toBeInTheDocument()
    expect(screen.getByDisplayValue('Created schedule')).toBeVisible()
  })

  it('shows shared webhook ingress details and regenerates webhook secrets', async () => {
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse([
          {
            id: 'trigger-webhook',
            name: 'Webhook trigger',
            enabled: true,
            protected: false,
            revision: 'revision-1',
            source_type: 'webhook',
            created_at: '2026-03-22T00:00:00Z',
            updated_at: '2026-03-22T00:00:00Z',
            action: { flow_name: 'webhook.dot', project_path: null, static_context: {} },
            source: { webhook_key: 'webhook-key-1' },
            state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: null, recent_history: [] },
          },
        ])
      }
      if (url.endsWith('/workspace/api/triggers/trigger-webhook') && method === 'PATCH') {
        return jsonResponse({
          id: 'trigger-webhook',
          name: 'Webhook trigger',
          enabled: true,
          protected: false,
          revision: 'revision-1',
          source_type: 'webhook',
          created_at: '2026-03-22T00:00:00Z',
          updated_at: '2026-03-22T00:01:00Z',
          action: { flow_name: 'webhook.dot', project_path: null, static_context: {} },
          source: { webhook_key: 'webhook-key-1' },
          state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: null, recent_history: [] },
          webhook_secret: 'new-webhook-secret',
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    expect(screen.getByTestId('triggers-project-context-chip')).toHaveTextContent('No active project')
    await user.click(await screen.findByTestId('trigger-row-trigger-webhook'))
    expect(screen.getByText(/POST JSON to/i)).toBeVisible()
    expect(screen.getAllByText(/webhook-key-1/).length).toBeGreaterThan(0)

    await user.click(screen.getByTestId('trigger-regenerate-secret-button'))

    await waitFor(() => {
      expect(screen.getByText(/new-webhook-secret/)).toBeVisible()
    })
  })

  it('serializes active, no-project, and custom execution targets when creating triggers', async () => {
    const postPayloads: Array<Record<string, unknown>> = []
    const createdTriggers: Array<ReturnType<typeof makeTrigger>> = []
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse(createdTriggers)
      }
      if (url.endsWith('/workspace/api/triggers') && method === 'POST') {
        const payload = JSON.parse(String(init?.body)) as Record<string, unknown>
        postPayloads.push(payload)
        const createdTrigger = makeTrigger({
          id: `trigger-created-${postPayloads.length}`,
          name: payload.name,
          revision: 'revision-1',
          source_type: payload.source_type,
          flow_name: (payload.action as Record<string, unknown>).flow_name,
          project_path: (payload.action as Record<string, unknown>).project_path ?? null,
          source: payload.source as Record<string, unknown>,
        })
        createdTriggers.splice(0, createdTriggers.length, createdTrigger)
        return jsonResponse(createdTrigger)
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/active-project')
      useStore.getState().setActiveProjectPath('/tmp/active-project')
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    let createScope = await openCreateTriggerForm(user)
    let nameInput = createScope.getByLabelText('Name')
    let targetFlowInput = createScope.getByLabelText('Target Flow')
    let executionTargetSelect = createScope.getByLabelText('Execution Target')
    const reopenCreateTriggerForm = async () => {
      await waitFor(() => {
        expect(screen.queryByTestId('trigger-create-button')).not.toBeInTheDocument()
      })
      createScope = await openCreateTriggerForm(user)
      nameInput = createScope.getByLabelText('Name')
      targetFlowInput = createScope.getByLabelText('Target Flow')
      executionTargetSelect = createScope.getByLabelText('Execution Target')
    }

    expect(executionTargetSelect).toHaveValue('active')
    expect(screen.getByText('Uses the current active project: /tmp/active-project')).toBeVisible()

    act(() => {
      useStore.getState().registerProject('/tmp/retargeted-project')
      useStore.getState().setActiveProjectPath('/tmp/retargeted-project')
    })

    await waitFor(() => {
      expect(executionTargetSelect).toHaveValue('active')
    })
    expect(screen.getByText('Uses the current active project: /tmp/retargeted-project')).toBeVisible()

    await user.type(nameInput, 'Active target trigger')
    await user.clear(targetFlowInput)
    await user.type(targetFlowInput, TEST_TRIGGER_FLOW)
    await user.click(screen.getByTestId('trigger-create-button'))

    await waitFor(() => {
      expect(postPayloads).toHaveLength(1)
    })
    expect(postPayloads[0]?.action).toMatchObject({ project_path: '/tmp/retargeted-project' })

    await reopenCreateTriggerForm()
    await user.type(nameInput, 'No project trigger')
    await user.clear(targetFlowInput)
    await user.type(targetFlowInput, TEST_TRIGGER_FLOW)
    await user.selectOptions(executionTargetSelect, 'none')
    expect(screen.queryByLabelText('Project Path')).not.toBeInTheDocument()

    act(() => {
      useStore.getState().registerProject('/tmp/ignored-project')
      useStore.getState().setActiveProjectPath('/tmp/ignored-project')
    })

    await waitFor(() => {
      expect(executionTargetSelect).toHaveValue('none')
    })
    await user.click(screen.getByTestId('trigger-create-button'))

    await waitFor(() => {
      expect(postPayloads).toHaveLength(2)
    })
    expect(postPayloads[1]?.action).toMatchObject({ project_path: null })

    await reopenCreateTriggerForm()
    await user.type(nameInput, 'Custom target trigger')
    await user.clear(targetFlowInput)
    await user.type(targetFlowInput, TEST_TRIGGER_FLOW)
    await user.selectOptions(executionTargetSelect, 'custom')
    await user.type(createScope.getByLabelText('Project Path'), '/tmp/custom-project')
    await user.click(screen.getByTestId('trigger-create-button'))

    await waitFor(() => {
      expect(postPayloads).toHaveLength(3)
    })
    expect(postPayloads[2]?.action).toMatchObject({ project_path: '/tmp/custom-project' })
  })

  it('hydrates existing target modes and filters to triggers that target the active project', async () => {
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse([
          makeTrigger({
            id: 'trigger-active-project',
            name: 'Active project trigger',
            project_path: '/tmp/active-project',
          }),
          makeTrigger({
            id: 'trigger-custom-project',
            name: 'Custom project trigger',
            project_path: '/tmp/custom-project',
          }),
        ])
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/active-project')
      useStore.getState().setActiveProjectPath('/tmp/active-project')
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    const customRow = await screen.findByTestId('trigger-row-trigger-custom-project')
    expect(customRow).toHaveTextContent('Project · custom-project')
    await user.click(customRow)
    const selectedTriggerCard = screen.getByTestId('trigger-save-button').closest('[data-slot="card"]')
    expect(selectedTriggerCard).not.toBeNull()
    const selectedTriggerScope = within(selectedTriggerCard as HTMLElement)

    await waitFor(() => {
      expect(selectedTriggerScope.getByDisplayValue('Other path')).toBeVisible()
    })
    expect(selectedTriggerScope.getByDisplayValue('/tmp/custom-project')).toBeVisible()
    expect(selectedTriggerScope.getByText('Target: Project · custom-project')).toBeVisible()

    await user.click(screen.getByTestId('triggers-filter-active-project'))

    await waitFor(() => {
      expect(screen.queryByText('Custom project trigger')).not.toBeInTheDocument()
    })
    expect(screen.getByTestId('trigger-row-trigger-active-project')).toHaveTextContent('Targets active project')
    expect(selectedTriggerScope.getByText('Target: Targets active project')).toBeVisible()
  })

  it('keeps protected trigger target and source settings locked while saving only allowed edits', async () => {
    const patchPayloads: Array<Record<string, unknown>> = []
    let protectedTrigger = makeTrigger({
      id: 'trigger-protected',
      name: 'Protected planning route',
      protected: true,
      revision: 'revision-1',
      source_type: 'schedule',
      flow_name: 'protected-plan.dot',
      project_path: '/tmp/protected-project',
      source: { kind: 'interval', interval_seconds: 300 },
    })
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse([protectedTrigger])
      }
      if (url.endsWith('/workspace/api/triggers/trigger-protected') && method === 'PATCH') {
        const payload = JSON.parse(String(init?.body)) as Record<string, unknown>
        patchPayloads.push(payload)
        const actionPayload = (payload.action as Record<string, unknown> | undefined) ?? {}
        protectedTrigger = {
          ...protectedTrigger,
          name: String(payload.name ?? protectedTrigger.name),
          enabled: payload.enabled === false ? false : protectedTrigger.enabled,
          updated_at: '2026-03-22T00:01:00Z',
          action: {
            ...protectedTrigger.action,
            flow_name: String(actionPayload.flow_name ?? protectedTrigger.action.flow_name),
          },
        }
        return jsonResponse(protectedTrigger)
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/active-project')
      useStore.getState().setActiveProjectPath('/tmp/active-project')
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    await user.click(await screen.findByTestId('trigger-row-trigger-protected'))
    const selectedTriggerCard = screen.getByTestId('trigger-save-button').closest('[data-slot="card"]')
    expect(selectedTriggerCard).not.toBeNull()
    const selectedTriggerScope = within(selectedTriggerCard as HTMLElement)

    expect(selectedTriggerScope.getByLabelText('Source Type')).toBeDisabled()
    expect(within(selectedTriggerScope.getByLabelText('Source Type')).queryByRole('option', { name: 'Workspace Event' })).not.toBeInTheDocument()
    expect(selectedTriggerScope.getByLabelText('Execution Target')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Project Path')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Schedule Kind')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Interval Seconds')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Static Context JSON')).toBeDisabled()

    await user.clear(selectedTriggerScope.getByLabelText('Name'))
    await user.type(selectedTriggerScope.getByLabelText('Name'), 'Protected planning route updated')
    await user.clear(selectedTriggerScope.getByLabelText('Target Flow'))
    await user.type(selectedTriggerScope.getByLabelText('Target Flow'), 'dispatch.dot')
    await user.click(selectedTriggerScope.getByRole('checkbox'))
    await user.click(selectedTriggerScope.getByTestId('trigger-save-button'))

    await waitFor(() => {
      expect(patchPayloads).toHaveLength(1)
    })

    const patchPayload = patchPayloads[0] ?? {}
    const actionPayload = (patchPayload.action as Record<string, unknown> | undefined) ?? {}
    expect(patchPayload).toMatchObject({
      name: 'Protected planning route updated',
      enabled: false,
      action: { flow_name: 'dispatch.dot' },
    })
    expect(patchPayload.source).toBeUndefined()
    expect(actionPayload.project_path).toBeUndefined()
    expect(actionPayload.static_context).toBeUndefined()

    await waitFor(() => {
      expect(selectedTriggerScope.getByText('Target: Project · protected-project')).toBeVisible()
    })
  })
  it('retains the draft revision after a live edit and conflict, blocks duplicate saves, and discards explicitly', async () => {
    let trigger = makeTrigger({ name: 'Original' })
    const patches: Array<Record<string, unknown>> = []
    let finishSave: ((response: Response) => void) | undefined
    vi.mocked(global.fetch).mockImplementation(async (_input, init) => {
      if (init?.method === 'PATCH') {
        patches.push(JSON.parse(String(init.body)))
        return new Promise<Response>((resolve) => { finishSave = resolve })
      }
      return jsonResponse([trigger])
    })
    renderTriggersPanel()
    const input = await screen.findByDisplayValue('Original')
    const user = userEvent.setup()
    await user.clear(input)
    await user.type(input, 'My draft')
    trigger = { ...trigger, name: 'External', revision: 'revision-2' }
    act(() => {
      useStore.getState().updateTriggersSession({ triggers: [trigger] })
    })
    expect(screen.getByDisplayValue('My draft')).toBeVisible()
    expect(screen.getByText(/changed elsewhere/)).toBeVisible()
    const save = screen.getByTestId('trigger-save-button')
    await user.click(save)
    expect(save).toBeDisabled()
    await user.click(save)
    expect(patches).toHaveLength(1)
    expect(patches[0].expected_revision).toBe('revision-1')
    await act(async () => {
      finishSave?.(new Response(JSON.stringify({ detail: 'Settings changed; reload before saving.' }), {
        status: 409, headers: { 'Content-Type': 'application/json' },
      }))
    })
    expect(screen.getByDisplayValue('My draft')).toBeVisible()
    expect(save).toBeEnabled()
    expect(useStore.getState().triggersSession.editTriggerDraftsByTriggerId['trigger-default'].expectedRevision).toBe('revision-1')
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(await screen.findByDisplayValue('External')).toBeVisible()
    expect(useStore.getState().triggersSession.editTriggerDraftsByTriggerId['trigger-default']).toBeUndefined()
  })

  it('hides the create form until New trigger and restores the selected trigger on cancel', async () => {
    vi.mocked(global.fetch).mockImplementation(async () => jsonResponse([makeTrigger({ name: 'Existing' })]))
    const user = userEvent.setup()
    renderTriggersPanel()

    expect(await screen.findByDisplayValue('Existing')).toBeVisible()
    expect(screen.queryByTestId('trigger-create-button')).not.toBeInTheDocument()

    const createScope = await openCreateTriggerForm(user)
    expect(screen.getByText('New trigger', { selector: '[data-slot="card-title"]' })).toBeVisible()
    expect(screen.queryByTestId('trigger-save-button')).not.toBeInTheDocument()
    await user.type(createScope.getByLabelText('Name'), 'Draft')
    expect(useStore.getState().triggersSession.createFormOpen).toBe(true)

    await user.click(screen.getByRole('button', { name: 'Cancel new trigger' }))
    expect(screen.queryByTestId('trigger-create-button')).not.toBeInTheDocument()
    expect(screen.getByDisplayValue('Existing')).toBeVisible()
    expect(useStore.getState().triggersSession.selectedTriggerId).toBe('trigger-default')

    await openCreateTriggerForm(user)
    expect(screen.getByDisplayValue('Draft')).toBeVisible()
  })

})
