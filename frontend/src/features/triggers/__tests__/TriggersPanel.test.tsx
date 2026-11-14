import { TriggersSessionController } from '@/app/AppSessionControllers'
import { TriggersPanel } from '@/features/triggers/TriggersPanel'
import { browserTimeZone, createEmptyTriggerForm } from '@/features/triggers/model/triggerForm'
import { triggerKey } from '@/lib/workspaceClient'
import { useStore } from '@/store'
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

const jsonResponse = (payload: unknown) =>
  new Response(JSON.stringify(payload), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  })
const TEST_TRIGGER_FLOW = 'test-planning.dot'
// The flows every test project can run, as /workspace/api/flows lists them.
const PROJECT_FLOWS = [
  { name: TEST_TRIGGER_FLOW, title: 'Test planning', description: '' },
  { name: 'dispatch.dot', title: 'Dispatch', description: '' },
]
const isFlowList = (url: string) => url.startsWith('/workspace/api/flows')

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
      revealedWebhookSecrets: {},
      createFormOpen: false,
      newTriggerDraft: {
        form: createEmptyTriggerForm(null),
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
  project_path: String(overrides.project_path ?? '/project'),
  name: String(overrides.name ?? 'Trigger'),
  enabled: overrides.enabled === false ? false : true,
  protected: overrides.protected === true,
  revision: 'revision-1',
  source_type: String(overrides.source_type ?? 'schedule'),
  created_at: '2026-03-22T00:00:00Z',
  updated_at: '2026-03-22T00:00:00Z',
  action: {
    flow_name: String(overrides.flow_name ?? TEST_TRIGGER_FLOW),
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

  it('keeps a restored selection while the list loads and falls back once a missing one is known', async () => {
    let respond: (triggers: unknown[]) => void = () => undefined
    vi.mocked(fetch).mockImplementation(() => new Promise((resolve) => { respond = (triggers) => resolve(jsonResponse(triggers)) }))
    act(() => useStore.getState().updateTriggersSession({ selectedTriggerId: triggerKey({ project_path: '/project', id: 'second' }) }))
    renderTriggersPanel()
    await waitFor(() => expect(fetch).toHaveBeenCalled())
    expect(useStore.getState().triggersSession.selectedTriggerId).toBe(triggerKey({ project_path: '/project', id: 'second' }))
    await act(async () => respond([makeTrigger({ id: 'first', name: 'First' }), makeTrigger({ id: 'second', name: 'Second' })]))
    expect(await screen.findByTestId('trigger-row-second')).toHaveAttribute('aria-current', 'true')

    // A restored trigger that no longer exists falls back to the first.
    act(() => useStore.getState().updateTriggersSession({ triggers: [makeTrigger({ id: 'first', name: 'First' })] }))
    await waitFor(() => expect(screen.getByTestId('trigger-row-first')).toHaveAttribute('aria-current', 'true'))
  })

  it('says where people look that the server left a trigger inactive, and why', async () => {
    const failed = makeTrigger({ id: 'failed', enabled: false, flow_name: 'missing' })
    vi.mocked(fetch).mockImplementation(async (input) => isFlowList(resolveRequestUrl(input))
      ? jsonResponse(PROJECT_FLOWS)
      : jsonResponse([{ ...failed, state: { ...failed.state, activation_error: 'Unknown flow: missing' } }]))
    renderTriggersPanel()
    const row = await screen.findByTestId('trigger-row-failed')
    expect(row).toHaveTextContent(/^TriggerNot running · /)
    expect(row).not.toHaveTextContent('Disabled')
    expect(row.querySelector('[data-slot="status-dot"]')).toHaveAttribute('data-tone', 'danger')
    const alert = await screen.findByTestId('trigger-activation-error')
    expect(alert).toHaveTextContent("Not running: Spark couldn't turn this trigger on")
    expect(alert).toHaveTextContent('Unknown flow: missing')
    // The flow it names is still shown, marked as not found.
    expect(await screen.findByRole('option', { name: 'missing (not found)' })).toBeInTheDocument()
    expect(screen.getByLabelText('Flow to start')).toHaveValue('missing')
    expect(screen.getByLabelText('Flow to start')).toHaveAccessibleDescription("This flow isn't in the project. Choose another.")
  })

  it('lists definitions that cannot run beside the triggers', async () => {
    vi.mocked(fetch).mockImplementation(async (input) => resolveRequestUrl(input).includes('/trigger-issues')
      ? jsonResponse([{ project_path: '/project', trigger_id: 'broken', error: 'Invalid trigger definition' }])
      : jsonResponse([makeTrigger({ id: 'fine' })]))
    renderTriggersPanel()
    const issues = await screen.findByTestId('trigger-issues')
    expect(issues).toHaveTextContent("Some triggers can't be loaded")
    expect(issues).toHaveTextContent('broken in /project: Invalid trigger definition')
  })

  it('creates a mission action from the project open mission picker', async () => {
    useStore.setState({ projectRegistry: { '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-03-22T00:00:00Z' } } })
    let saved: Record<string, unknown> | null = null
    vi.mocked(fetch).mockImplementation(async (input, init) => {
      const url = resolveRequestUrl(input)
      if (url.includes('/missions?')) return jsonResponse({ missions: [{ id: 'open', fields: { title: 'Open mission' } }, { id: 'closed', fields: { title: 'Closed mission' }, closed: { status: 'done' } }] })
      if (init?.method === 'POST') saved = JSON.parse(String(init.body))
      const trigger = { ...makeTrigger({ id: 'mission-trigger' }), ...saved }
      return jsonResponse(init?.method === 'POST' ? trigger : saved ? [trigger] : [])
    })
    const user = userEvent.setup()
    renderTriggersPanel()
    const form = await openCreateTriggerForm(user)
    await user.type(form.getByLabelText('Name'), 'Mission watcher')
    await user.selectOptions(form.getByLabelText('Action'), 'mission')
    await form.findByRole('option', { name: 'Open mission' })
    expect(form.queryByRole('option', { name: 'Closed mission' })).not.toBeInTheDocument()
    expect(form.queryByLabelText('Target flow')).not.toBeInTheDocument()
    await user.selectOptions(form.getByLabelText('Mission'), 'open')
    await user.click(screen.getByTestId('trigger-create-button'))
    await waitFor(() => expect(saved).toMatchObject({ project_path: '/project', action: { mode: 'mission', mission_id: 'open' } }))
    expect((saved as unknown as { action: object }).action).not.toHaveProperty('flow_name')
    expect((saved as unknown as { action: object }).action).not.toHaveProperty('project_path')
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
              project_path: '/project',
              name: 'Created schedule',
              enabled: true,
              protected: false,
              revision: 'revision-1',
              source_type: 'schedule',
              created_at: '2026-03-22T00:00:00Z',
              updated_at: '2026-03-22T00:00:00Z',
              action: { flow_name: TEST_TRIGGER_FLOW, static_context: {} },
              source: { kind: 'interval', interval_seconds: 300 },
              state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: '2026-03-22T00:05:00Z', recent_history: [] },
            },
          ]
          : []
        return jsonResponse(payload)
      }
      if (isFlowList(url)) return jsonResponse(PROJECT_FLOWS)
      if (url.endsWith('/workspace/api/triggers') && method === 'POST') {
        return jsonResponse({
          id: 'trigger-created',
          project_path: '/project',
          name: 'Created schedule',
          enabled: true,
          protected: false,
          revision: 'revision-1',
          source_type: 'schedule',
          created_at: '2026-03-22T00:00:00Z',
          updated_at: '2026-03-22T00:00:00Z',
          action: { flow_name: TEST_TRIGGER_FLOW, static_context: {} },
          source: { kind: 'interval', interval_seconds: 300 },
          state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: '2026-03-22T00:05:00Z', recent_history: [] },
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    useStore.setState({ projectRegistry: { '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-03-22T00:00:00Z' } } })
    const user = userEvent.setup()
    renderTriggersPanel()

    const createScope = await openCreateTriggerForm(user)
    expect(createScope.getByLabelText('Every')).toHaveAccessibleDescription(/^First run in 5 minutes \((Today|Tomorrow) at .+\), then every 5 minutes\.$/)
    expect(within(createScope.getByLabelText('Starts when')).queryByRole('option', { name: 'Workspace Event' })).not.toBeInTheDocument()
    await user.type(createScope.getByLabelText('Name'), 'Created schedule')
    await user.selectOptions(createScope.getByLabelText('Flow to start'), await createScope.findByRole('option', { name: 'Test planning' }))
    expect(createScope.getByText(TEST_TRIGGER_FLOW)).toBeVisible()
    await user.click(screen.getByTestId('trigger-create-button'))

    await waitFor(() => {
      expect(screen.getByTestId('trigger-row-trigger-created')).toHaveTextContent('Created schedule')
    })
    expect(screen.queryByTestId('trigger-create-button')).not.toBeInTheDocument()
    expect(screen.getByDisplayValue('Created schedule')).toBeVisible()
  })

  it('words the status card in plain time and result words', async () => {
    const fired = new Date().toISOString()
    const trigger = makeTrigger({ id: 'ran' })
    vi.mocked(fetch).mockImplementation(async (input) => isFlowList(resolveRequestUrl(input))
      ? jsonResponse(PROJECT_FLOWS)
      : jsonResponse([{ ...trigger, state: { ...trigger.state, last_fired_at: fired, last_result: 'success', recent_history: [{ timestamp: fired, status: 'failed', message: 'Boom' }] } }]))
    renderTriggersPanel()
    expect(await screen.findByText(/^Last triggered: Today at \d{1,2}:\d{2}\s[AP]M$/)).toBeVisible()
    expect(screen.getByText('Last result: Succeeded')).toBeVisible()
    expect(screen.getByText('Failed')).toBeVisible()
    // Editing: the status card shows the next run, so the interval has no first-run hint.
    expect(screen.getByLabelText('Every')).not.toHaveAccessibleDescription()
  })

  it('reports a firing that started a run by that run\'s state, linked to the run', async () => {
    const fired = new Date().toISOString()
    const trigger = makeTrigger({ id: 'sweep', name: 'Tests sweep' })
    useStore.setState((state) => ({ runsListSession: { ...state.runsListSession, runs: [
      { run_id: 'run-failed', flow_name: 'nightly-tests.dot', status: 'failed', started_at: fired, working_directory: '/project', model: '' },
    ] } }))
    vi.mocked(fetch).mockImplementation(async (input) => isFlowList(resolveRequestUrl(input))
      ? jsonResponse(PROJECT_FLOWS)
      : jsonResponse([{ ...trigger, state: { ...trigger.state, last_fired_at: fired, last_result: 'success', recent_history: [
        { timestamp: fired, status: 'success', message: 'Trigger fired successfully.', run_id: 'run-failed' },
        { timestamp: fired, status: 'success', message: 'Trigger fired successfully.', run_id: 'run-gone' },
      ] } }]))
    renderTriggersPanel()
    expect(await screen.findByTestId('trigger-last-result')).toHaveTextContent('Last result: Failed · Nightly Tests')
    const [latest, older] = screen.getAllByTestId('trigger-history-entry')
    expect(latest).toHaveTextContent('Failed · Nightly Tests')
    expect(latest).not.toHaveTextContent('fired successfully')
    expect(older).toHaveTextContent('Started a run')
    await userEvent.setup().click(within(latest).getByRole('link', { name: 'Nightly Tests' }))
    expect(useStore.getState().viewMode).toBe('runs')
    expect(useStore.getState().runsListSession.selectedRunId).toBe('run-failed')
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
            project_path: '/project',
            name: 'Webhook trigger',
            enabled: true,
            protected: false,
            revision: 'revision-1',
            source_type: 'webhook',
            created_at: '2026-03-22T00:00:00Z',
            updated_at: '2026-03-22T00:00:00Z',
            action: { flow_name: 'webhook.dot', static_context: {} },
            source: { webhook_key: 'webhook-key-1' },
            state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: null, recent_history: [] },
          },
        ])
      }
      if (url.endsWith('/workspace/api/triggers/trigger-webhook?project_path=%2Fproject') && method === 'PATCH') {
        return jsonResponse({
          id: 'trigger-webhook',
          project_path: '/project',
          name: 'Webhook trigger',
          enabled: true,
          protected: false,
          revision: 'revision-1',
          source_type: 'webhook',
          created_at: '2026-03-22T00:00:00Z',
          updated_at: '2026-03-22T00:01:00Z',
          action: { flow_name: 'webhook.dot', static_context: {} },
          source: { webhook_key: 'webhook-key-1' },
          state: { last_fired_at: null, last_result: null, last_error: null, next_run_at: null, recent_history: [] },
          webhook_secret: 'new-webhook-secret',
        })
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    await user.click(await screen.findByTestId('trigger-row-trigger-webhook'))
    expect(screen.getByText(/Send a POST request/i)).toBeVisible()
    expect(screen.getAllByText(/webhook-key-1/).length).toBeGreaterThan(0)

    await user.click(screen.getByTestId('trigger-regenerate-secret-button'))

    await waitFor(() => {
      expect(screen.getByText(/new-webhook-secret/)).toBeVisible()
    })
  })

  it('shows errors at the field in plain words, and saves an interval in a unit as seconds', async () => {
    const posts: Array<Record<string, unknown>> = []
    vi.mocked(global.fetch).mockImplementation(async (input, init) => {
      const url = resolveRequestUrl(input as RequestInfo | URL)
      if (isFlowList(url)) return jsonResponse(PROJECT_FLOWS)
      if (init?.method === 'POST') {
        posts.push(JSON.parse(String(init.body)))
        return jsonResponse(makeTrigger({ id: 'created' }))
      }
      return jsonResponse(posts.length ? [makeTrigger({ id: 'created' })] : [])
    })
    useStore.setState({ projectRegistry: { '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-03-22T00:00:00Z' } } })
    const user = userEvent.setup()
    renderTriggersPanel()
    const form = await openCreateTriggerForm(user)
    await user.type(form.getByLabelText('Name'), 'Hourly')
    const context = form.getByLabelText('Extra data for the run (optional)')
    await user.type(context, '{{branch: main}')
    await user.click(screen.getByTestId('trigger-create-button'))

    // Refused before sending, with each problem beside its field and no parser message.
    expect(posts).toHaveLength(0)
    expect(screen.queryByRole('alert')).not.toBeInTheDocument()
    const flow = form.getByLabelText('Flow to start')
    expect(flow).toHaveAttribute('aria-invalid', 'true')
    expect(flow).toHaveFocus()
    expect(flow).toHaveAccessibleDescription(/Choose the flow to start\./)
    expect(context).toHaveAttribute('aria-invalid', 'true')
    expect(context).toHaveAccessibleDescription(/This isn't a valid JSON object\./)
    expect(context).not.toHaveAccessibleDescription(/Unexpected token|position/)

    // Errors follow the edits once shown.
    await user.selectOptions(flow, await form.findByRole('option', { name: 'Test planning' }))
    expect(flow).not.toHaveAttribute('aria-invalid')
    await user.clear(context)
    await user.type(context, '{{"branch": "main"}')
    expect(context).not.toHaveAttribute('aria-invalid')
    await user.clear(form.getByLabelText('Every'))
    await user.type(form.getByLabelText('Every'), '2')
    await user.selectOptions(form.getByLabelText('Every unit'), 'hours')
    await user.click(screen.getByTestId('trigger-create-button'))
    await waitFor(() => expect(posts).toHaveLength(1))
    expect(posts[0]).toMatchObject({
      action: { flow_name: TEST_TRIGGER_FLOW, static_context: { branch: 'main' } },
      source: { kind: 'interval', interval_seconds: 7200 },
    })
  })

  it('asks for the flow\'s launch inputs, refuses a missing required one, and saves them under their keys', async () => {
    const releaseCheck = { name: 'release-check', title: 'Release Check', description: '' }
    const posts: Array<Record<string, unknown>> = []
    const saved = { ...makeTrigger({ id: 'nightly', name: 'Nightly', flow_name: 'release-check' }), action: { flow_name: 'release-check', static_context: { 'context.request.version': '1.2.0', branch: 'main' } } }
    vi.mocked(global.fetch).mockImplementation(async (input, init) => {
      const url = resolveRequestUrl(input as RequestInfo | URL)
      if (url.startsWith('/workspace/api/flows/release-check')) {
        return jsonResponse({ ...releaseCheck, inputs: [{ key: 'context.request.version', label: 'Version', type: 'string', description: 'The version to release.', required: true }] })
      }
      if (isFlowList(url)) return jsonResponse([...PROJECT_FLOWS, releaseCheck])
      if (init?.method === 'POST') {
        posts.push(JSON.parse(String(init.body)))
        return jsonResponse(saved)
      }
      return jsonResponse(posts.length ? [saved] : [])
    })
    useStore.setState({ projectRegistry: { '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-03-22T00:00:00Z' } } })
    const user = userEvent.setup()
    renderTriggersPanel()
    const form = await openCreateTriggerForm(user)
    await user.type(form.getByLabelText('Name'), 'Nightly')
    await user.selectOptions(form.getByLabelText('Flow to start'), await form.findByRole('option', { name: 'Release Check' }))
    const version = await form.findByLabelText(/^Version/)
    expect(form.getByText('What each run needs')).toBeVisible()
    expect(version).toHaveAccessibleDescription('The version to release.')

    await user.click(screen.getByTestId('trigger-create-button'))
    expect(posts).toHaveLength(0)
    expect(version).toHaveAttribute('aria-invalid', 'true')
    expect(version).toHaveFocus()
    expect(version).toHaveAccessibleDescription(/Version is required\./)

    await user.type(version, '1.2.0')
    await user.type(form.getByLabelText('Extra data for the run (optional)'), '{{"branch": "main"}')
    await user.click(screen.getByTestId('trigger-create-button'))
    await waitFor(() => expect(posts).toHaveLength(1))
    expect(posts[0]).toMatchObject({ action: { flow_name: 'release-check', static_context: { 'context.request.version': '1.2.0', branch: 'main' } } })

    // Opened again, the input is back in its field and only the rest is extra data.
    await waitFor(() => expect(screen.getByLabelText(/^Version/)).toHaveValue('1.2.0'))
    expect(JSON.parse((screen.getByLabelText('Extra data for the run (optional)') as HTMLTextAreaElement).value)).toEqual({ branch: 'main' })
  })

  it('waits for the flow\'s launch inputs before a trigger that starts it can be created', async () => {
    const releaseCheck = { name: 'release-check', title: 'Release Check', description: '' }
    let release: () => void = () => undefined
    vi.mocked(global.fetch).mockImplementation(async (input) => {
      const url = resolveRequestUrl(input as RequestInfo | URL)
      if (url.startsWith('/workspace/api/flows/release-check')) {
        return new Promise((resolve) => {
          release = () => resolve(jsonResponse({ ...releaseCheck, inputs: [{ key: 'context.request.version', label: 'Version', type: 'string', required: true }] }))
        })
      }
      if (isFlowList(url)) return jsonResponse([...PROJECT_FLOWS, releaseCheck])
      return jsonResponse([])
    })
    useStore.setState({ projectRegistry: { '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-03-22T00:00:00Z' } } })
    const user = userEvent.setup()
    renderTriggersPanel()
    const form = await openCreateTriggerForm(user)
    await user.type(form.getByLabelText('Name'), 'Nightly')
    await user.selectOptions(form.getByLabelText('Flow to start'), await form.findByRole('option', { name: 'Release Check' }))

    // Its required inputs aren't known yet, so they can't be checked.
    expect(screen.getByTestId('trigger-create-button')).toBeDisabled()
    expect(form.getByRole('status')).toHaveTextContent('Reading the flow’s launch inputs…')
    await act(async () => release())
    expect(await form.findByLabelText(/^Version/)).toBeVisible()
    expect(screen.getByTestId('trigger-create-button')).toBeEnabled()
  })

  it('marks a trigger whose last firing failed in the list, with the reason on hover', async () => {
    const failing = makeTrigger({ id: 'nightly', name: 'Nightly' })
    failing.state = { ...failing.state, last_result: 'failed', last_error: 'Release version is required.' }
    vi.mocked(global.fetch).mockImplementation(async (input) => (
      isFlowList(resolveRequestUrl(input as RequestInfo | URL)) ? jsonResponse(PROJECT_FLOWS) : jsonResponse([failing, makeTrigger({ id: 'fine', name: 'Fine' })])
    ))
    renderTriggersPanel()
    const problem = await within(await screen.findByTestId('trigger-row-nightly')).findByTestId('trigger-row-problem')
    expect(problem).toHaveTextContent('Last firing failed')
    expect(problem).toHaveAttribute('title', 'Release version is required.')
    expect(within(screen.getByTestId('trigger-row-fine')).queryByTestId('trigger-row-problem')).not.toBeInTheDocument()
  })

  it('saves weekly days from toggles and a time in the browser\'s zone, and refuses a week with no day', async () => {
    const posts: Array<Record<string, unknown>> = []
    vi.mocked(global.fetch).mockImplementation(async (input, init) => {
      const url = resolveRequestUrl(input as RequestInfo | URL)
      if (isFlowList(url)) return jsonResponse(PROJECT_FLOWS)
      if (init?.method === 'POST') {
        posts.push(JSON.parse(String(init.body)))
        return jsonResponse(makeTrigger({ id: 'created' }))
      }
      return jsonResponse(posts.length ? [makeTrigger({ id: 'created' })] : [])
    })
    useStore.setState({ projectRegistry: { '/project': { directoryPath: '/project', isFavorite: false, lastAccessedAt: '2026-03-22T00:00:00Z' } } })
    const user = userEvent.setup()
    renderTriggersPanel()
    const form = await openCreateTriggerForm(user)
    await user.type(form.getByLabelText('Name'), 'Standup')
    await user.selectOptions(form.getByLabelText('Flow to start'), await form.findByRole('option', { name: 'Test planning' }))
    await user.selectOptions(form.getByLabelText('Repeats'), 'weekly')
    const days = form.getByRole('group', { name: 'Days' })
    expect(within(days).getAllByRole('button').map((day) => day.textContent)).toEqual(['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'])
    expect(within(days).getByRole('button', { name: 'Monday' })).toHaveAttribute('aria-pressed', 'true')
    expect(within(days).getByRole('button', { name: 'Wednesday' })).toHaveAttribute('aria-pressed', 'false')
    expect(form.getByLabelText('Time')).toHaveAccessibleDescription(`In your time zone, ${browserTimeZone()}.`)
    // Days take several presses, unlike the one-choice controls beside them.
    expect(days).toHaveAccessibleDescription('Pick one or more.')

    // Every day off: refused, and focus moves to the days.
    await user.click(within(days).getByRole('button', { name: 'Monday' }))
    await user.click(within(days).getByRole('button', { name: 'Friday' }))
    await user.click(screen.getByTestId('trigger-create-button'))
    expect(posts).toHaveLength(0)
    expect(days).toHaveFocus()
    expect(days).toHaveAccessibleDescription('Pick one or more. Pick at least one day.')

    await user.click(within(days).getByRole('button', { name: 'Friday' }))
    await user.click(within(days).getByRole('button', { name: 'Wednesday' }))
    fireEvent.change(form.getByLabelText('Time'), { target: { value: '07:30' } })
    expect(days).not.toHaveAttribute('aria-invalid')
    await user.click(screen.getByTestId('trigger-create-button'))
    await waitFor(() => expect(posts).toHaveLength(1))
    expect(posts[0].source).toEqual({ kind: 'weekly', weekdays: ['wed', 'fri'], hour: 7, minute: 30, timezone: browserTimeZone() })
  })

  it('says which zone a saved weekly trigger runs in when it is not the browser\'s, and lists its days and time', async () => {
    const zone = browserTimeZone() === 'Asia/Tokyo' ? 'Europe/Paris' : 'Asia/Tokyo'
    vi.mocked(fetch).mockImplementation(async (input) => isFlowList(resolveRequestUrl(input))
      ? jsonResponse(PROJECT_FLOWS)
      : jsonResponse([makeTrigger({ id: 'weekly', source: { kind: 'weekly', weekdays: ['mon', 'thu'], hour: 9, minute: 0, timezone: zone } })]))
    renderTriggersPanel()
    expect((await screen.findByTestId('trigger-row-weekly')).textContent?.replace(/\s/g, ' ')).toContain(`Mon, Thu at 9:00 AM (${zone})`)
    const time = await screen.findByLabelText('Time')
    expect(time).toHaveValue('09:00')
    expect(time).toHaveAccessibleDescription(`In ${zone}, not your time zone.`)
  })

  it('opens on the list on a phone; a picked trigger shows its form until All triggers returns to its row', async () => {
    window.innerWidth = 390
    try {
      vi.mocked(fetch).mockImplementation(async (input) => isFlowList(resolveRequestUrl(input))
        ? jsonResponse(PROJECT_FLOWS)
        : jsonResponse([makeTrigger({ id: 'first', name: 'First' }), makeTrigger({ id: 'second', name: 'Second' })]))
      const user = userEvent.setup()
      renderTriggersPanel()
      const second = await screen.findByTestId('trigger-row-second')
      await waitFor(() => expect(useStore.getState().triggersSession.status).toBe('ready'))
      expect(useStore.getState().triggersSession.selectedTriggerId).toBeNull()
      expect(screen.queryByTestId('trigger-save-button')).not.toBeInTheDocument()
      // The list alone shows: no empty detail pane under it.
      expect(screen.queryByTestId('view-main')).not.toBeInTheDocument()
      // No protected triggers, so no System section.
      expect(screen.queryByRole('heading', { name: 'System' })).not.toBeInTheDocument()

      await user.click(second)
      expect(await screen.findByDisplayValue('Second')).toBeVisible()
      await user.click(screen.getByRole('button', { name: 'All triggers' }))
      expect(useStore.getState().triggersSession.selectedTriggerId).toBeNull()
      expect(screen.queryByTestId('view-main')).not.toBeInTheDocument()
      expect(second).toHaveFocus()
    } finally {
      window.innerWidth = 1024
    }
  })

  it('creates triggers in a registered project, defaulting to the one worked in most recently, as Run a flow does', async () => {
    const postPayloads: Array<Record<string, unknown>> = []
    const createdTriggers: Array<ReturnType<typeof makeTrigger>> = []
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse(createdTriggers)
      }
      if (isFlowList(url)) return jsonResponse(PROJECT_FLOWS)
      if (url.endsWith('/workspace/api/triggers') && method === 'POST') {
        const payload = JSON.parse(String(init?.body)) as Record<string, unknown>
        postPayloads.push(payload)
        const createdTrigger = makeTrigger({
          id: `trigger-created-${postPayloads.length}`,
          name: payload.name,
          revision: 'revision-1',
          source_type: payload.source_type,
          flow_name: (payload.action as Record<string, unknown>).flow_name,
          project_path: payload.project_path,
          source: payload.source as Record<string, unknown>,
        })
        createdTriggers.splice(0, createdTriggers.length, createdTrigger)
        return jsonResponse(createdTrigger)
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    useStore.setState({
      activeProjectPath: '/tmp/active-project',
      projectRegistry: {
        '/tmp/last-used-project': { directoryPath: '/tmp/last-used-project', isFavorite: false, lastAccessedAt: '2026-03-22T00:00:00Z' },
        '/tmp/active-project': { directoryPath: '/tmp/active-project', isFavorite: false, lastAccessedAt: '2026-03-01T00:00:00Z' },
        '/tmp/other-project': { directoryPath: '/tmp/other-project', isFavorite: false, lastAccessedAt: '2026-02-01T00:00:00Z' },
      },
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    let createScope = await openCreateTriggerForm(user)
    // No execution target or free-form path: only a registered project.
    expect(createScope.queryByLabelText('Execution target')).not.toBeInTheDocument()
    expect(createScope.queryByLabelText('Project path')).not.toBeInTheDocument()
    expect(createScope.getByLabelText('Project')).toHaveValue('/tmp/last-used-project')
    await user.type(createScope.getByLabelText('Name'), 'Last used project trigger')
    await user.selectOptions(createScope.getByLabelText('Flow to start'), await createScope.findByRole('option', { name: 'Test planning' }))
    await user.click(screen.getByTestId('trigger-create-button'))
    await waitFor(() => expect(postPayloads).toHaveLength(1))
    expect(postPayloads[0]).toMatchObject({ project_path: '/tmp/last-used-project' })
    expect(postPayloads[0]?.action).not.toHaveProperty('project_path')

    await waitFor(() => expect(screen.queryByTestId('trigger-create-button')).not.toBeInTheDocument())
    createScope = await openCreateTriggerForm(user)
    await user.selectOptions(createScope.getByLabelText('Project'), '/tmp/other-project')
    await user.type(createScope.getByLabelText('Name'), 'Other project trigger')
    await user.selectOptions(createScope.getByLabelText('Flow to start'), await createScope.findByRole('option', { name: 'Test planning' }))
    await user.click(screen.getByTestId('trigger-create-button'))
    await waitFor(() => expect(postPayloads).toHaveLength(2))
    expect(postPayloads[1]).toMatchObject({ project_path: '/tmp/other-project' })
  })

  it('cannot create a trigger without a registered project', async () => {
    vi.mocked(global.fetch).mockImplementation(async () => jsonResponse([]))
    const user = userEvent.setup()
    renderTriggersPanel()
    await openCreateTriggerForm(user)
    expect(screen.getByTestId('trigger-create-button')).toBeDisabled()
  })

  it('shows each trigger project and keeps it fixed when editing', async () => {
    const fetchMock = vi.mocked(global.fetch)
    fetchMock.mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = resolveRequestUrl(input)
      const method = init?.method ?? 'GET'
      if (url.endsWith('/workspace/api/triggers') && method === 'GET') {
        return jsonResponse([
          makeTrigger({
            id: 'trigger-registered-project',
            name: 'Registered project trigger',
            project_path: '/tmp/registered-project',
          }),
          makeTrigger({
            id: 'trigger-other-project',
            name: 'Other project trigger',
            project_path: '/tmp/other-project',
            enabled: false,
          }),
        ])
      }
      throw new Error(`Unhandled request: ${method} ${url}`)
    })

    act(() => {
      useStore.getState().registerProject('/tmp/registered-project')
    })

    const user = userEvent.setup()
    renderTriggersPanel()

    const registeredRow = await screen.findByTestId('trigger-row-trigger-registered-project')
    expect(screen.getByTestId('trigger-row-trigger-registered-project').querySelector('[data-testid="trigger-row-project"]')).toHaveTextContent(/^registered-project$/)
    expect(registeredRow).not.toHaveTextContent('Disabled')
    expect(registeredRow).not.toHaveAttribute('data-unregistered-target')
    const otherRow = screen.getByTestId('trigger-row-trigger-other-project')
    // Only the disabled row carries the word; its dot is hollow.
    expect(otherRow).toHaveTextContent('Disabled · Unregistered project · other-project · Every 5 minutes')
    expect(otherRow.querySelector('[data-slot="status-dot"]')).toHaveClass('border')
    expect(otherRow).toHaveAttribute('data-unregistered-target', 'true')

    await user.click(registeredRow)
    const selectedTriggerScope = within(screen.getByTestId('trigger-save-button').closest('[data-slot="card"]') as HTMLElement)
    await waitFor(() => {
      expect(selectedTriggerScope.getByLabelText('Project')).toHaveValue('/tmp/registered-project')
    })
    expect(selectedTriggerScope.getByLabelText('Project')).toBeDisabled()
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
      if (isFlowList(url)) return jsonResponse(PROJECT_FLOWS)
      if (url.endsWith('/workspace/api/triggers/trigger-protected?project_path=%2Ftmp%2Fprotected-project') && method === 'PATCH') {
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

    const user = userEvent.setup()
    renderTriggersPanel()

    await user.click(await screen.findByTestId('trigger-row-trigger-protected'))
    expect(screen.getByRole('heading', { name: 'System' })).toBeVisible()
    const selectedTriggerCard = screen.getByTestId('trigger-save-button').closest('[data-slot="card"]')
    expect(selectedTriggerCard).not.toBeNull()
    const selectedTriggerScope = within(selectedTriggerCard as HTMLElement)

    expect(selectedTriggerScope.getByLabelText('Starts when')).toBeDisabled()
    expect(within(selectedTriggerScope.getByLabelText('Starts when')).queryByRole('option', { name: 'Workspace Event' })).not.toBeInTheDocument()
    expect(selectedTriggerScope.getByLabelText('Project')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Repeats')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Every')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Every unit')).toBeDisabled()
    expect(selectedTriggerScope.getByLabelText('Extra data for the run (optional)')).toBeDisabled()

    await user.clear(selectedTriggerScope.getByLabelText('Name'))
    await user.type(selectedTriggerScope.getByLabelText('Name'), 'Protected planning route updated')
    await user.selectOptions(selectedTriggerScope.getByLabelText('Flow to start'), await selectedTriggerScope.findByRole('option', { name: 'Dispatch' }))
    await user.click(selectedTriggerScope.getByRole('switch', { name: 'Enabled' }))
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
      expect(selectedTriggerScope.getByText('Unregistered project · protected-project')).toBeVisible()
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
    expect(useStore.getState().triggersSession.editTriggerDraftsByTriggerId[triggerKey(trigger)].expectedRevision).toBe('revision-1')
    await user.click(screen.getByRole('button', { name: /^Discard/ }))
    expect(await screen.findByDisplayValue('External')).toBeVisible()
    expect(useStore.getState().triggersSession.editTriggerDraftsByTriggerId[triggerKey(trigger)]).toBeUndefined()
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

    await user.click(screen.getByRole('button', { name: 'Close new trigger' }))
    expect(screen.queryByTestId('trigger-create-button')).not.toBeInTheDocument()
    expect(screen.getByDisplayValue('Existing')).toBeVisible()
    expect(useStore.getState().triggersSession.selectedTriggerId).toBe(triggerKey({ project_path: '/project', id: 'trigger-default' }))

    await openCreateTriggerForm(user)
    expect(screen.getByDisplayValue('Draft')).toBeVisible()
  })


  describe('the same id in two projects', () => {
    const projectA = '/tmp/project-a'
    const projectB = '/tmp/project-b'
    const rowFor = (name: string) => screen.getByRole('button', { name: new RegExp(name) })
    const editorScope = () => within(screen.getByTestId('trigger-save-button').closest('[data-slot="card"]') as HTMLElement)
    const twins = () => [
      makeTrigger({ id: 'daily', name: 'Daily A', project_path: projectA, source_type: 'webhook', source: { webhook_key: 'key-a' } }),
      makeTrigger({ id: 'daily', name: 'Daily B', project_path: projectB, source_type: 'webhook', source: { webhook_key: 'key-b' } }),
    ]

    it('selects, edits and deletes each one independently, scoped to its project', async () => {
      let triggers = twins()
      const requests: Array<{ method: string; url: string; body?: Record<string, unknown> }> = []
      vi.mocked(global.fetch).mockImplementation(async (input, init) => {
        const url = resolveRequestUrl(input as RequestInfo | URL)
        const method = init?.method ?? 'GET'
        if (method === 'GET') return jsonResponse(triggers)
        const body = init?.body ? JSON.parse(String(init.body)) as Record<string, unknown> : undefined
        requests.push({ method, url, body })
        const project = new URL(url, 'http://local').searchParams.get('project_path')
        const target = triggers.find((trigger) => trigger.project_path === project)!
        if (method === 'DELETE') {
          triggers = triggers.filter((trigger) => trigger !== target)
          return jsonResponse({ status: 'deleted', id: 'daily', project_path: project })
        }
        const updated = { ...target, name: String(body?.name ?? target.name), revision: 'revision-2' }
        triggers = triggers.map((trigger) => trigger === target ? updated : trigger)
        return jsonResponse(updated)
      })
      const user = userEvent.setup()
      renderTriggersPanel()

      await waitFor(() => expect(screen.getAllByTestId('trigger-row-daily')).toHaveLength(2))
      await user.click(rowFor('Daily B'))
      expect(rowFor('Daily B')).toHaveAttribute('aria-current', 'true')
      expect(rowFor('Daily A')).not.toHaveAttribute('aria-current', 'true')
      expect(editorScope().getByLabelText('Name')).toHaveValue('Daily B')

      await user.clear(editorScope().getByLabelText('Name'))
      await user.type(editorScope().getByLabelText('Name'), 'Daily B renamed')
      await user.click(screen.getByTestId('trigger-save-button'))
      await waitFor(() => expect(requests).toHaveLength(1))
      expect(requests[0]).toMatchObject({ method: 'PATCH', body: { name: 'Daily B renamed' } })
      expect(requests[0].url).toContain(`project_path=${encodeURIComponent(projectB)}`)
      await waitFor(() => expect(rowFor('Daily B renamed')).toBeVisible())
      expect(rowFor('Daily A')).toBeVisible()

      await user.click(rowFor('Daily A'))
      expect(editorScope().getByLabelText('Name')).toHaveValue('Daily A')
      vi.spyOn(window, 'confirm').mockReturnValue(true)
      await user.click(screen.getByTestId('trigger-delete-button'))
      await waitFor(() => expect(requests.map((request) => request.method)).toEqual(['PATCH', 'DELETE']))
      expect(requests[1].url).toContain(`project_path=${encodeURIComponent(projectA)}`)
      await waitFor(() => expect(screen.getAllByTestId('trigger-row-daily')).toHaveLength(1))
      expect(rowFor('Daily B renamed')).toBeVisible()
    })

    it('keeps separate drafts and secrets, and applies one project\'s live events without touching the other', async () => {
      vi.mocked(global.fetch).mockImplementation(async () => jsonResponse(twins()))
      const user = userEvent.setup()
      renderTriggersPanel()
      await waitFor(() => expect(screen.getAllByTestId('trigger-row-daily')).toHaveLength(2))
      const [a, b] = twins()
      act(() => useStore.getState().updateTriggersSession({
        revealedWebhookSecrets: { [triggerKey(a)]: 'secret-a', [triggerKey(b)]: 'secret-b' },
      }))

      await user.click(rowFor('Daily A'))
      await user.type(editorScope().getByLabelText('Name'), ' draft')
      await user.click(rowFor('Daily B'))
      expect(editorScope().getByLabelText('Name')).toHaveValue('Daily B')
      expect(screen.getByText(/secret-b/)).toBeVisible()
      expect(screen.queryByText(/secret-a/)).not.toBeInTheDocument()
      await user.click(rowFor('Daily A'))
      expect(editorScope().getByLabelText('Name')).toHaveValue('Daily A draft')
      expect(screen.getByText(/secret-a/)).toBeVisible()
      const drafts = useStore.getState().triggersSession.editTriggerDraftsByTriggerId
      expect(Object.keys(drafts)).toEqual([triggerKey(a)])

      const live = (type: string, trigger: unknown) => act(() => {
        window.dispatchEvent(new CustomEvent('spark:trigger-live-event', { detail: { type, payload: { trigger } } }))
      })
      live('trigger.upsert', { ...b, name: 'Daily B live', revision: 'revision-2' })
      expect(rowFor('Daily B live')).toBeVisible()
      expect(useStore.getState().triggersSession.triggers.find((trigger) => trigger.project_path === projectA)).toMatchObject({ name: 'Daily A', revision: 'revision-1' })
      expect(rowFor('Daily A')).toHaveAttribute('aria-current', 'true')

      live('trigger.delete', { status: 'deleted', id: 'daily', project_path: projectB })
      expect(screen.getAllByTestId('trigger-row-daily')).toHaveLength(1)
      expect(rowFor('Daily A')).toHaveAttribute('aria-current', 'true')
      expect(editorScope().getByLabelText('Name')).toHaveValue('Daily A draft')
    })
  })
})
