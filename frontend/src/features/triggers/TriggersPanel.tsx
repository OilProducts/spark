import { completePreferenceInteraction } from '@/features/settings/services/clientPreferences'
import { Plus } from "lucide-react"
import { useEffect, useMemo } from "react"

import { TriggerEditor } from "./components/TriggerEditor"
import {
  formatTriggerTimestamp,
  SHARED_WEBHOOK_ENDPOINT,
  triggerTargetSummary,
  triggerTargetsActiveProject,
  triggerSourceSummary,
} from "./model/triggerForm"
import { useTriggersList } from "./hooks/useTriggersList"
import { useTriggerEditor } from "./hooks/useTriggerEditor"
import { useWebhookSecretRegeneration } from "./hooks/useWebhookSecretRegeneration"
import { useStore } from "@/store"
import { InlineError } from "@/components/app/inline-error"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
} from "@/components/ui/empty"
import { formatProjectPathLabel } from "@/lib/projectPaths"
export function TriggersPanel() {
  const activeProjectPath = useStore((state) => state.activeProjectPath)
  const triggersSession = useStore((state) => state.triggersSession)
  const updateTriggersSession = useStore((state) => state.updateTriggersSession)
  const revealedWebhookSecrets = triggersSession.revealedWebhookSecrets
  const scopeFilter = triggersSession.scopeFilter
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
    activeProjectPath,
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
  const filteredSystemTriggers = useMemo(
    () => scopeFilter === 'active' && activeProjectPath
      ? systemTriggers.filter((trigger) => triggerTargetsActiveProject(trigger, activeProjectPath))
      : systemTriggers,
    [activeProjectPath, scopeFilter, systemTriggers],
  )
  const filteredCustomTriggers = useMemo(
    () => scopeFilter === 'active' && activeProjectPath
      ? customTriggers.filter((trigger) => triggerTargetsActiveProject(trigger, activeProjectPath))
      : customTriggers,
    [activeProjectPath, customTriggers, scopeFilter],
  )
  const visibleTriggers = useMemo(
    () => [...filteredCustomTriggers, ...filteredSystemTriggers],
    [filteredCustomTriggers, filteredSystemTriggers],
  )
  const projectLabel = activeProjectPath
    ? formatProjectPathLabel(activeProjectPath)
    : 'No active project'

  useEffect(() => {
    if (scopeFilter === 'active' && !activeProjectPath) {
      updateTriggersSession({ scopeFilter: 'all' })
    }
  }, [activeProjectPath, scopeFilter, updateTriggersSession])

  useEffect(() => {
    if (selectedTriggerId && visibleTriggers.some((trigger) => trigger.id === selectedTriggerId)) {
      return
    }
    const firstVisibleTriggerId = visibleTriggers[0]?.id ?? null
    if (firstVisibleTriggerId !== selectedTriggerId) {
      setSelectedTriggerId(firstVisibleTriggerId)
    }
  }, [selectedTriggerId, setSelectedTriggerId, visibleTriggers])

  const createFormOpen = triggersSession.createFormOpen
  const hasDetail = createFormOpen || selectedTrigger !== null
  const triggerGroups = [
    { title: 'Custom', note: null, triggers: filteredCustomTriggers, loadingTestId: 'triggers-custom-list-loading', emptyText: 'No custom triggers in this scope yet.' },
    { title: 'System', note: 'Protected approval and review routing.', triggers: filteredSystemTriggers, loadingTestId: 'triggers-system-list-loading', emptyText: 'No protected triggers in this scope.' },
  ]

  return (
    <section data-testid="triggers-panel" className="flex-1 overflow-auto p-6">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
        <div className="flex flex-wrap items-start justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <h2 className="text-xl font-semibold tracking-tight text-foreground">Triggers</h2>
            <p className="text-sm text-muted-foreground">
              Automations that start flows on a schedule, on events, or from webhooks.
            </p>
            {activeProjectPath ? (
              <div className="flex flex-wrap items-center gap-2 pt-1">
                <Button
                  type="button"
                  data-testid="triggers-filter-all"
                  onClick={() => { updateTriggersSession({ scopeFilter: 'all' }); completePreferenceInteraction({ triggers_scope: 'all' }) }}
                  variant={scopeFilter === 'all' ? 'secondary' : 'outline'}
                  size="xs"
                >
                  All triggers
                </Button>
                <Button
                  type="button"
                  data-testid="triggers-filter-active-project"
                  onClick={() => { updateTriggersSession({ scopeFilter: 'active' }); completePreferenceInteraction({ triggers_scope: 'active' }) }}
                  variant={scopeFilter === 'active' ? 'secondary' : 'outline'}
                  size="xs"
                >
                  Targets active project
                </Button>
              </div>
            ) : null}
          </div>
          <div className="flex items-center gap-2">
            <Badge
              data-testid="triggers-project-context-chip"
              variant="outline"
              title={activeProjectPath || 'No active project'}
            >
              <span className="text-muted-foreground">Project:</span>
              <span className="max-w-40 truncate">{projectLabel}</span>
            </Badge>
            <Button
              type="button"
              data-testid="trigger-new-button"
              size="sm"
              onClick={() => updateTriggersSession({ createFormOpen: true })}
            >
              <Plus />
              New trigger
            </Button>
          </div>
        </div>

        {error ? (
          <InlineError>{error}</InlineError>
        ) : null}

        <div className={hasDetail ? 'grid items-start gap-6 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]' : 'grid gap-6'}>
          <Card className="gap-4 py-4">
            <CardHeader className="flex flex-row items-center justify-between gap-2 px-4">
              <CardTitle className="text-base">Triggers</CardTitle>
              <Button
                type="button"
                onClick={() => void refreshTriggers()}
                variant="outline"
                size="xs"
              >
                {loading ? 'Refreshing…' : 'Refresh'}
              </Button>
            </CardHeader>
            <CardContent className="space-y-6 px-4 pt-0">
              {triggerGroups.map((group) => (
                <div key={group.title} className="space-y-2">
                  <div>
                    <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{group.title}</h3>
                    {group.note ? <p className="text-xs text-muted-foreground">{group.note}</p> : null}
                  </div>
                  {status !== 'ready' && status !== 'error' ? (
                    <p data-testid={group.loadingTestId} className="text-sm text-muted-foreground" aria-live="polite">Restoring triggers…</p>
                  ) : null}
                  {group.triggers.map((trigger) => (
                    <Button
                      key={trigger.id}
                      type="button"
                      data-testid={`trigger-row-${trigger.id}`}
                      onClick={() => setSelectedTriggerId(trigger.id)}
                      variant="outline"
                      className={`h-auto w-full justify-start whitespace-normal rounded-md px-3 py-2 text-left ${selectedTriggerId === trigger.id ? 'border-foreground bg-muted/60' : 'border-border bg-background/70'}`}
                    >
                      <div className="w-full min-w-0">
                        <div className="flex items-center justify-between gap-2">
                          <span className="truncate text-sm font-medium">{trigger.name}</span>
                          <Badge
                            variant="outline"
                            className={trigger.enabled ? 'border-success/40 bg-success/10 text-success' : 'text-muted-foreground'}
                          >
                            {trigger.enabled ? 'Enabled' : 'Disabled'}
                          </Badge>
                        </div>
                        <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                          <span className="max-w-full truncate">{triggerSourceSummary(trigger)}</span>
                          <span aria-hidden="true">→</span>
                          <span className="max-w-full truncate font-mono">{trigger.action.flow_name}</span>
                          <Badge variant="outline">
                            {triggerTargetSummary(trigger, activeProjectPath)}
                          </Badge>
                        </div>
                      </div>
                    </Button>
                  ))}
                  {status === 'ready' && group.triggers.length === 0 ? (
                    <Empty className="px-3 py-4 text-xs text-muted-foreground">
                      <EmptyHeader>
                        <EmptyDescription>{group.emptyText}</EmptyDescription>
                      </EmptyHeader>
                    </Empty>
                  ) : null}
                </div>
              ))}
            </CardContent>
          </Card>

          {createFormOpen ? (
            <Card className="gap-4 py-4">
              <CardHeader className="gap-1 px-4">
                <CardTitle className="text-base">New trigger</CardTitle>
              </CardHeader>
              <CardContent className="px-4 pt-0">
                <TriggerEditor
                  form={newTriggerForm}
                  onChange={setNewTriggerForm}
                  mode="create"
                  protectedTrigger={false}
                  activeProjectPath={activeProjectPath}
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
                  <CardTitle className="truncate text-base">{selectedTrigger.name}</CardTitle>
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
                    variant="outline"
                    size="xs"
                    className="border-destructive/40 text-destructive hover:bg-destructive/10"
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
                  activeProjectPath={activeProjectPath}
                />

                {selectedTrigger.source_type === 'webhook' ? (
                  <div className="mt-4 space-y-2 rounded-md border border-border bg-background/70 p-3 text-sm">
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
                  <div className="rounded-md border border-border bg-background/70 p-3 text-sm">
                    <div className="font-medium text-foreground">Runtime</div>
                    <div className="mt-2 text-muted-foreground">Target: {triggerTargetSummary(selectedTrigger, activeProjectPath)}</div>
                    <div className="mt-2 text-muted-foreground">Last fired: {formatTriggerTimestamp(selectedTrigger.state.last_fired_at)}</div>
                    <div className="text-muted-foreground">Next run: {formatTriggerTimestamp(selectedTrigger.state.next_run_at)}</div>
                    <div className="text-muted-foreground">Last result: {selectedTrigger.state.last_result ?? 'Never'}</div>
                    {selectedTrigger.state.last_error ? (
                      <div className="mt-2 text-destructive">{selectedTrigger.state.last_error}</div>
                    ) : null}
                  </div>
                  <div className="rounded-md border border-border bg-background/70 p-3 text-sm">
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
  )
}
