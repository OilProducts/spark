import { Plus } from "lucide-react"
import { useEffect, useMemo } from "react"

import { TriggerEditor } from "./components/TriggerEditor"
import {
  createEmptyTriggerForm,
  formatTriggerTimestamp,
  SHARED_WEBHOOK_ENDPOINT,
  triggerProjectSummary,
  triggerSourceSummary,
} from "./model/triggerForm"
import { useTriggersList } from "./hooks/useTriggersList"
import { useTriggerEditor } from "./hooks/useTriggerEditor"
import { useWebhookSecretRegeneration } from "./hooks/useWebhookSecretRegeneration"
import { useStore } from "@/store"
import { triggerKey } from "@/lib/workspaceClient"
import { defaultProjectChoice, isUnregisteredTarget } from "@/features/projects/model/projectChoices"
import { ViewLayout } from "@/components/app/view-layout"
import { InlineError } from "@/components/app/inline-error"
import { SectionHeading } from "@/components/app/section-heading"
import { ListGroupLabel, SidePanelRow } from "@/components/app/side-panel-row"
import { Well } from "@/components/app/well"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
export function TriggersPanel() {
  const projectRegistry = useStore((state) => state.projectRegistry)
  const activeProjectPath = useStore((state) => state.activeProjectPath)
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
  const revealWebhookSecret = (key: string, secret: string) => {
    updateTriggersSession({
      revealedWebhookSecrets: {
        ...revealedWebhookSecrets,
        [key]: secret,
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
    if (selectedTriggerId && visibleTriggers.some((trigger) => triggerKey(trigger) === selectedTriggerId)) {
      return
    }
    const firstVisibleTriggerId = visibleTriggers[0] ? triggerKey(visibleTriggers[0]) : null
    if (firstVisibleTriggerId !== selectedTriggerId) {
      setSelectedTriggerId(firstVisibleTriggerId)
    }
  }, [selectedTriggerId, setSelectedTriggerId, status, visibleTriggers])

  const createFormOpen = triggersSession.createFormOpen
  const openCreateForm = () => {
    // An untouched draft starts in the active project, else the last-used one.
    if (!newTriggerForm.name.trim() && !newTriggerForm.projectPath) {
      setTriggersSessionNewDraft({ form: createEmptyTriggerForm(defaultProjectChoice(projectRegistry, activeProjectPath)) })
    }
    updateTriggersSession({ createFormOpen: true })
  }
  const triggerGroups = [
    { title: 'Custom', note: null, triggers: customTriggers, loadingTestId: 'triggers-custom-list-loading', emptyText: 'No custom triggers yet.' },
    { title: 'System', note: 'Protected approval and review routing.', triggers: systemTriggers, loadingTestId: 'triggers-system-list-loading', emptyText: 'No protected triggers.' },
  ]

  const panel = (
    <div className="space-y-4">
      {triggerGroups.map((group) => (
        <div key={group.title} className="space-y-0.5">
          <ListGroupLabel>
            <h3>{group.title}</h3>
            {group.note ? <p className="font-normal">{group.note}</p> : null}
          </ListGroupLabel>
          {status !== 'ready' && status !== 'error' ? (
            <p data-testid={group.loadingTestId} className="px-3 text-sm text-muted-foreground" aria-live="polite">Restoring triggers…</p>
          ) : null}
          {group.triggers.map((trigger) => {
            const unregistered = isUnregisteredTarget(projectRegistry, trigger.project_path)
            return (
              <SidePanelRow
                key={triggerKey(trigger)}
                data-testid={`trigger-row-${trigger.id}`}
                data-unregistered-target={unregistered ? 'true' : undefined}
                selected={selectedTriggerId === triggerKey(trigger) && !createFormOpen}
                // A disabled trigger is the exception: hollow dot plus the word.
                tone="neutral"
                hollow={!trigger.enabled}
                title={trigger.name}
                meta={(
                  <>
                    <span
                      data-testid="trigger-row-project"
                      title={trigger.project_path}
                      className={unregistered ? 'text-warning' : undefined}
                    >
                      {triggerProjectSummary(trigger, projectRegistry)}
                    </span>
                    {' · '}{triggerSourceSummary(trigger)}
                    {trigger.enabled ? null : ' · Disabled'}
                  </>
                )}
                onClick={() => {
                  setSelectedTriggerId(triggerKey(trigger))
                  updateTriggersSession({ createFormOpen: false })
                }}
              />
            )
          })}
          {status === 'ready' && group.triggers.length === 0 ? (
            <p className="px-3 py-1 text-xs text-muted-foreground">{group.emptyText}</p>
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
          <Button type="button" data-testid="trigger-new-button" variant="outline" size="xs" onClick={openCreateForm}>
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
              <CardHeader className="gap-1 px-0">
                <CardTitle role="heading" aria-level={3} className="text-xl font-semibold">New trigger</CardTitle>
              </CardHeader>
              <CardContent className="px-0 pt-0">
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
                    variant="ghost"
                    disabled={pending}
                    onClick={() => updateTriggersSession({ createFormOpen: false })}
                  >
                    Cancel
                  </Button>
                  <Button
                    type="button"
                    data-testid="trigger-create-button"
                    disabled={pending || isRegenerating || !newTriggerForm.projectPath}
                    onClick={() => void onCreateTrigger()}
                  >
                    Create trigger
                  </Button>
                </div>
              </CardContent>
            </Card>
          ) : selectedTrigger && editTriggerForm ? (
            <Card className="gap-4 py-4">
              <CardHeader className="flex flex-row items-start justify-between gap-2 px-0">
                <div className="min-w-0">
                  <CardTitle role="heading" aria-level={3} className="truncate text-xl font-semibold">{selectedTrigger.name}</CardTitle>
                  <div className="mt-1 text-xs text-muted-foreground">
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

              <CardContent className="px-0 pt-0">
                <TriggerEditor
                  form={editTriggerForm}
                  onChange={setEditTriggerForm}
                  mode="edit"
                  protectedTrigger={selectedTrigger.protected}
                />

                {selectedTrigger.source_type === 'webhook' ? (
                  <Well className="mt-4 space-y-2 text-sm">
                    <SectionHeading as="h4">Shared webhook ingress</SectionHeading>
                    <div className="text-muted-foreground">POST JSON to <code>{SHARED_WEBHOOK_ENDPOINT}</code> with:</div>
                    <div className="font-mono text-xs text-foreground">
                      X-Spark-Webhook-Key: {String(selectedTrigger.source.webhook_key ?? '')}
                    </div>
                    <div className="font-mono text-xs text-foreground">
                      X-Spark-Webhook-Secret: {revealedWebhookSecrets[triggerKey(selectedTrigger)] ?? 'Hidden after creation'}
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
                  </Well>
                ) : null}

                <div className="mt-4 grid gap-3 lg:grid-cols-2">
                  <Well className="text-sm">
                    <SectionHeading as="h4">Runtime</SectionHeading>
                    <div className="mt-2 text-muted-foreground">{triggerProjectSummary(selectedTrigger, projectRegistry)}</div>
                    <div className="mt-2 text-muted-foreground">Last fired: {formatTriggerTimestamp(selectedTrigger.state.last_fired_at)}</div>
                    <div className="text-muted-foreground">Next run: {formatTriggerTimestamp(selectedTrigger.state.next_run_at)}</div>
                    <div className="text-muted-foreground">Last result: {selectedTrigger.state.last_result ?? 'Never'}</div>
                    {selectedTrigger.state.last_error ? (
                      <div className="mt-2 text-destructive">{selectedTrigger.state.last_error}</div>
                    ) : null}
                  </Well>
                  <Well className="text-sm">
                    <SectionHeading as="h4">Recent history</SectionHeading>
                    <div className="mt-2 space-y-2">
                      {selectedTrigger.state.recent_history.slice(0, 5).map((entry) => (
                        <div key={`${entry.timestamp}-${entry.status}`}>
                          <div className="text-xs text-foreground">{entry.status}</div>
                          <div className="text-xs text-muted-foreground">{formatTriggerTimestamp(entry.timestamp)}</div>
                          <div className="text-xs text-muted-foreground">{entry.message}</div>
                        </div>
                      ))}
                      {selectedTrigger.state.recent_history.length === 0 ? (
                        <p className="text-xs text-muted-foreground">No trigger history yet.</p>
                      ) : null}
                    </div>
                  </Well>
                </div>

                {externalChange ? <p role="status">This trigger changed elsewhere. Your draft is retained; discard to reload.</p> : null}
                <div className="mt-4 flex justify-end gap-2">
                  <Button aria-label="Discard selected trigger changes" type="button" variant="ghost" disabled={!dirty || pending || isRegenerating} onClick={discard}>Discard</Button>
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
