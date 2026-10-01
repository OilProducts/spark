import { Plus } from "lucide-react"
import { useEffect, useMemo } from "react"

import { TriggerEditor } from "./components/TriggerEditor"
import {
  createEmptyTriggerForm,
  formatTriggerTimestamp,
  SHARED_WEBHOOK_ENDPOINT,
  triggerTargetSummary,
  triggerSourceSummary,
} from "./model/triggerForm"
import { useTriggersList } from "./hooks/useTriggersList"
import { useTriggerEditor } from "./hooks/useTriggerEditor"
import { useWebhookSecretRegeneration } from "./hooks/useWebhookSecretRegeneration"
import { useStore } from "@/store"
import { defaultProjectChoice, isUnregisteredTarget } from "@/features/projects/model/projectChoices"
import { ViewLayout } from "@/components/app/view-layout"
import { InlineError } from "@/components/app/inline-error"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
} from "@/components/ui/empty"
export function TriggersPanel() {
  const projectRegistry = useStore((state) => state.projectRegistry)
  const triggersSession = useStore((state) => state.triggersSession)
  const updateTriggersSession = useStore((state) => state.updateTriggersSession)
  const setTriggersSessionNewDraft = useStore((state) => state.setTriggersSessionNewDraft)
  const revealedWebhookSecrets = triggersSession.revealedWebhookSecrets
  const {
    customTriggers,
    error,
    loading,
    refreshTriggers,
    selectedTrigger,
    selectedTriggerId,
    status,
    setError,
    setSelectedTriggerId,
    systemTriggers,
  } = useTriggersList({ manageSync: false })
  const revealWebhookSecret = (triggerId: string, secret: string) => {
    updateTriggersSession({
      revealedWebhookSecrets: {
        ...revealedWebhookSecrets,
        [triggerId]: secret,
      },
    })
  }
  const {
    pending,
    dirty,
    externalChange,
    discard,
    editTriggerForm,
    newTriggerForm,
    onCreateTrigger,
    onDeleteSelectedTrigger,
    onSaveSelectedTrigger,
    setEditTriggerForm,
    setNewTriggerForm,
  } = useTriggerEditor({
    refreshTriggers,
    selectedTrigger,
    setError,
    revealWebhookSecret,
    setSelectedTriggerId,
  })
  const { isRegenerating, onRegenerateWebhookSecret } = useWebhookSecretRegeneration({
    refreshTriggers,
    selectedTrigger,
    setError,
    revealWebhookSecret,
  })
  const visibleTriggers = useMemo(
    () => [...customTriggers, ...systemTriggers],
    [customTriggers, systemTriggers],
  )

  useEffect(() => {
    // Keep a restored selection until the list has loaded.
    if (status !== 'ready') {
      return
    }
    if (selectedTriggerId && visibleTriggers.some((trigger) => trigger.id === selectedTriggerId)) {
      return
    }
    const firstVisibleTriggerId = visibleTriggers[0]?.id ?? null
    if (firstVisibleTriggerId !== selectedTriggerId) {
      setSelectedTriggerId(firstVisibleTriggerId)
    }
  }, [selectedTriggerId, setSelectedTriggerId, status, visibleTriggers])

  const createFormOpen = triggersSession.createFormOpen
  const openCreateForm = () => {
    // An untouched draft starts in the last-used project.
    if (!newTriggerForm.name.trim() && !newTriggerForm.projectPath) {
      setTriggersSessionNewDraft({ form: createEmptyTriggerForm(defaultProjectChoice(projectRegistry)) })
    }
    updateTriggersSession({ createFormOpen: true })
  }
  const triggerGroups = [
    { title: 'Custom', note: null, triggers: customTriggers, loadingTestId: 'triggers-custom-list-loading', emptyText: 'No custom triggers yet.' },
    { title: 'System', note: 'Protected approval and review routing.', triggers: systemTriggers, loadingTestId: 'triggers-system-list-loading', emptyText: 'No protected triggers.' },
  ]

  const panel = (
    <div className="space-y-5 px-2">
      {triggerGroups.map((group) => (
        <div key={group.title} className="space-y-1">
          <div className="px-2">
            <h3 className="text-xs font-medium uppercase tracking-wide text-muted-foreground">{group.title}</h3>
            {group.note ? <p className="text-xs text-muted-foreground">{group.note}</p> : null}
          </div>
          {status !== 'ready' && status !== 'error' ? (
            <p data-testid={group.loadingTestId} className="px-2 text-sm text-muted-foreground" aria-live="polite">Restoring triggers…</p>
          ) : null}
          {group.triggers.map((trigger) => {
            const unregistered = isUnregisteredTarget(projectRegistry, trigger.action.project_path)
            return (
              <button
                key={trigger.id}
                type="button"
                data-testid={`trigger-row-${trigger.id}`}
                data-unregistered-target={unregistered ? 'true' : undefined}
                aria-current={selectedTriggerId === trigger.id && !createFormOpen ? 'true' : undefined}
                onClick={() => {
                  setSelectedTriggerId(trigger.id)
                  updateTriggersSession({ createFormOpen: false })
                }}
                className={`block w-full px-2 py-1.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring ${selectedTriggerId === trigger.id && !createFormOpen ? 'text-primary shadow-[inset_2px_0_0_hsl(var(--primary))]' : 'hover:text-primary'}`}
              >
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate text-sm">{trigger.name}</span>
                  <span className={`shrink-0 text-xs ${trigger.enabled ? 'text-success' : 'text-muted-foreground'}`}>{trigger.enabled ? 'Enabled' : 'Disabled'}</span>
                </div>
                <div className="mt-0.5 truncate text-xs text-muted-foreground">
                  <span
                    data-testid="trigger-row-target"
                    title={trigger.action.project_path ?? undefined}
                    className={unregistered ? 'text-warning' : undefined}
                  >
                    {triggerTargetSummary(trigger, projectRegistry)}
                  </span>
                  {' · '}{triggerSourceSummary(trigger)}
                </div>
              </button>
            )
          })}
          {status === 'ready' && group.triggers.length === 0 ? (
            <Empty className="px-3 py-3 text-xs text-muted-foreground">
              <EmptyHeader>
                <EmptyDescription>{group.emptyText}</EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : null}
        </div>
      ))}
    </div>
  )

  return (
    <ViewLayout
      view="triggers"
      title="Triggers"
      actions={(
        <>
          <Button type="button" variant="ghost" size="xs" onClick={() => void refreshTriggers()}>
            {loading ? 'Refreshing…' : 'Refresh'}
          </Button>
          <Button type="button" data-testid="trigger-new-button" variant="ghost" size="xs" onClick={openCreateForm}>
            <Plus />
            New
          </Button>
        </>
      )}
      panel={panel}
    >
    <section data-testid="triggers-panel" className="h-full overflow-auto p-6">
      <div className="mx-auto flex w-full max-w-4xl flex-col gap-6">
        <p className="text-sm text-muted-foreground">
          Automations that start flows on a schedule, on events, or from webhooks.
        </p>

        {error ? (
          <InlineError>{error}</InlineError>
        ) : null}

        <div className="grid gap-6">
          {createFormOpen ? (
            <Card className="gap-4 py-4">
              <CardHeader className="gap-1 px-4">
                <CardTitle className="text-lg font-light">New trigger</CardTitle>
              </CardHeader>
              <CardContent className="px-4 pt-0">
                <TriggerEditor
                  form={newTriggerForm}
                  onChange={setNewTriggerForm}
                  mode="create"
                  protectedTrigger={false}
                />
                <div className="mt-4 flex justify-end gap-2">
                  <Button
                    type="button"
                    aria-label="Cancel new trigger"
                    data-testid="trigger-create-cancel-button"
                    variant="outline"
                    disabled={pending}
                    onClick={() => updateTriggersSession({ createFormOpen: false })}
                  >
                    Cancel
                  </Button>
                  <Button
                    type="button"
                    data-testid="trigger-create-button"
                    disabled={pending || isRegenerating}
                    onClick={() => void onCreateTrigger()}
                  >
                    Create trigger
                  </Button>
                </div>
              </CardContent>
            </Card>
          ) : selectedTrigger && editTriggerForm ? (
            <Card className="gap-4 py-4">
              <CardHeader className="flex flex-row items-start justify-between gap-2 px-4">
                <div className="min-w-0">
                  <CardTitle className="truncate text-lg font-light">{selectedTrigger.name}</CardTitle>
                  <div className="text-xs text-muted-foreground">
                    {selectedTrigger.protected ? 'System trigger' : 'Custom trigger'}
                  </div>
                </div>
                {!selectedTrigger.protected ? (
                  <Button
                    type="button"
                    data-testid="trigger-delete-button"
                    disabled={pending || isRegenerating}
                    onClick={() => void onDeleteSelectedTrigger()}
                    variant="destructive"
                    size="xs"
                  >
                    Delete
                  </Button>
                ) : null}
              </CardHeader>

              <CardContent className="px-4 pt-0">
                <TriggerEditor
                  form={editTriggerForm}
                  onChange={setEditTriggerForm}
                  mode="edit"
                  protectedTrigger={selectedTrigger.protected}
                />

                {selectedTrigger.source_type === 'webhook' ? (
                  <div className="mt-4 space-y-2 rounded-md border border-border p-3 text-sm">
                    <div className="font-medium text-foreground">Shared webhook ingress</div>
                    <div className="text-muted-foreground">POST JSON to <code>{SHARED_WEBHOOK_ENDPOINT}</code> with:</div>
                    <div className="font-mono text-xs text-foreground">
                      X-Spark-Webhook-Key: {String(selectedTrigger.source.webhook_key ?? '')}
                    </div>
                    <div className="font-mono text-xs text-foreground">
                      X-Spark-Webhook-Secret: {revealedWebhookSecrets[selectedTrigger.id] ?? 'Hidden after creation'}
                    </div>
                    <Button
                      type="button"
                      data-testid="trigger-regenerate-secret-button"
                      onClick={() => void onRegenerateWebhookSecret()}
                      variant="outline"
                      size="xs"
                    >
                      {isRegenerating ? 'Regenerating…' : 'Regenerate secret'}
                    </Button>
                  </div>
                ) : null}

                <div className="mt-4 grid gap-3 lg:grid-cols-2">
                  <div className="rounded-md border border-border p-3 text-sm">
                    <div className="font-medium text-foreground">Runtime</div>
                    <div className="mt-2 text-muted-foreground">Target: {triggerTargetSummary(selectedTrigger, projectRegistry)}</div>
                    <div className="mt-2 text-muted-foreground">Last fired: {formatTriggerTimestamp(selectedTrigger.state.last_fired_at)}</div>
                    <div className="text-muted-foreground">Next run: {formatTriggerTimestamp(selectedTrigger.state.next_run_at)}</div>
                    <div className="text-muted-foreground">Last result: {selectedTrigger.state.last_result ?? 'Never'}</div>
                    {selectedTrigger.state.last_error ? (
                      <div className="mt-2 text-destructive">{selectedTrigger.state.last_error}</div>
                    ) : null}
                  </div>
                  <div className="rounded-md border border-border p-3 text-sm">
                    <div className="font-medium text-foreground">Recent history</div>
                    <div className="mt-2 space-y-2">
                      {selectedTrigger.state.recent_history.slice(0, 5).map((entry) => (
                        <div key={`${entry.timestamp}-${entry.status}`} className="rounded border border-border/70 px-2 py-1">
                          <div className="text-xs text-foreground">{entry.status}</div>
                          <div className="text-xs text-muted-foreground">{formatTriggerTimestamp(entry.timestamp)}</div>
                          <div className="text-xs text-muted-foreground">{entry.message}</div>
                        </div>
                      ))}
                      {selectedTrigger.state.recent_history.length === 0 ? (
                        <Empty className="px-3 py-4 text-xs text-muted-foreground">
                          <EmptyHeader>
                            <EmptyDescription>No trigger history yet.</EmptyDescription>
                          </EmptyHeader>
                        </Empty>
                      ) : null}
                    </div>
                  </div>
                </div>

                {externalChange ? <p role="status">This trigger changed elsewhere. Your draft is retained; discard to reload.</p> : null}
                <div className="mt-4 flex justify-end gap-2">
                  <Button aria-label="Discard selected trigger changes" type="button" variant="outline" disabled={!dirty || pending || isRegenerating} onClick={discard}>Discard</Button>
                  <Button aria-label="Save selected trigger"
                    type="button"
                    data-testid="trigger-save-button"
                    disabled={pending || isRegenerating}
                    onClick={() => void onSaveSelectedTrigger()}
                  >
                    Save
                  </Button>
                </div>
              </CardContent>
            </Card>
          ) : null}
        </div>
      </div>
    </section>
    </ViewLayout>
  )
}
