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
import { Alert, AlertDescription } from "@/components/ui/alert"
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
  const visibleTriggerIds = useMemo(
    () => new Set([...filteredSystemTriggers, ...filteredCustomTriggers].map((trigger) => trigger.id)),
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
    if (!selectedTriggerId) {
      const firstVisibleTrigger = [...filteredSystemTriggers, ...filteredCustomTriggers][0]
      if (firstVisibleTrigger) {
        setSelectedTriggerId(firstVisibleTrigger.id)
      }
      return
    }
    if (visibleTriggerIds.has(selectedTriggerId)) {
      return
    }
    const firstVisibleTrigger = [...filteredSystemTriggers, ...filteredCustomTriggers][0]
    setSelectedTriggerId(firstVisibleTrigger?.id ?? null)
  }, [filteredCustomTriggers, filteredSystemTriggers, selectedTriggerId, setSelectedTriggerId, visibleTriggerIds])

  return (
    <section data-testid="triggers-panel" className="flex-1 overflow-auto p-6">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 space-y-1">
            <h2 className="text-sm font-semibold text-foreground">Triggers</h2>
            <p className="text-xs leading-5 text-muted-foreground">
              Manage system routing, schedules, polling, flow-event automation, and shared webhook ingress.
            </p>
          </div>
          <Badge
            data-testid="triggers-project-context-chip"
            variant="outline"
            title={activeProjectPath || 'No active project'}
          >
            <span className="text-muted-foreground">Project:</span>
            <span className="max-w-40 truncate">{projectLabel}</span>
          </Badge>
        </div>

        {activeProjectPath ? (
          <div className="flex flex-wrap items-center gap-2">
            <Button
              type="button"
              data-testid="triggers-filter-all"
              onClick={() => updateTriggersSession({ scopeFilter: 'all' })}
              variant={scopeFilter === 'all' ? 'secondary' : 'outline'}
              size="xs"
            >
              All triggers
            </Button>
            <Button
              type="button"
              data-testid="triggers-filter-active-project"
              onClick={() => updateTriggersSession({ scopeFilter: 'active' })}
              variant={scopeFilter === 'active' ? 'secondary' : 'outline'}
              size="xs"
            >
              Targets active project
            </Button>
          </div>
        ) : null}

        {error ? (
          <Alert className="border-destructive/40 bg-destructive/10 px-3 py-2 text-destructive">
            <AlertDescription className="text-inherit">{error}</AlertDescription>
          </Alert>
        ) : null}

        <div className="grid gap-6 lg:grid-cols-[minmax(20rem,26rem)_minmax(0,1fr)]">
          <div className="space-y-4">
            <Card className="gap-4 py-4">
              <CardHeader className="flex flex-row items-center justify-between gap-2 px-4">
                <div>
                  <CardTitle className="text-sm">System triggers</CardTitle>
                  <div className="text-sm text-muted-foreground">Protected approval and review routing.</div>
                </div>
                <Button
                  type="button"
                  onClick={() => void refreshTriggers()}
                  variant="outline"
                  size="xs"
                >
                  {loading ? 'Refreshing…' : 'Refresh'}
                </Button>
              </CardHeader>
              <CardContent className="space-y-2 px-4 pt-0">
                {status !== 'ready' && status !== 'error' ? (
                  <Alert
                    data-testid="triggers-system-list-loading"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                  >
                    <AlertDescription className="text-inherit">Restoring triggers…</AlertDescription>
                  </Alert>
                ) : null}
                {filteredSystemTriggers.map((trigger) => (
                  <Button
                    key={trigger.id}
                    type="button"
                    data-testid={`trigger-row-${trigger.id}`}
                    onClick={() => setSelectedTriggerId(trigger.id)}
                    variant="outline"
                    className={`h-auto w-full justify-start rounded-md px-3 py-2 text-left ${selectedTriggerId === trigger.id ? 'border-foreground bg-muted/60' : 'border-border bg-background/70'}`}
                    >
                    <div className="w-full">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-sm font-medium">{trigger.name}</span>
                        <span className="text-[11px] text-muted-foreground">{trigger.enabled ? 'Enabled' : 'Disabled'}</span>
                      </div>
                      <div className="mt-1 flex flex-wrap items-center gap-2">
                        <Badge variant="outline" className="text-[11px]">
                          {triggerTargetSummary(trigger, activeProjectPath)}
                        </Badge>
                        <span className="text-xs text-muted-foreground">{triggerSourceSummary(trigger)}</span>
                      </div>
                    </div>
                  </Button>
                ))}
                {status === 'ready' && filteredSystemTriggers.length === 0 ? (
                  <Empty className="px-3 py-4 text-xs text-muted-foreground">
                    <EmptyHeader>
                      <EmptyDescription>No protected triggers in this scope.</EmptyDescription>
                    </EmptyHeader>
                  </Empty>
                ) : null}
              </CardContent>
            </Card>

            <Card className="gap-4 py-4">
              <CardHeader className="gap-1 px-4">
                <CardTitle className="text-sm">Custom triggers</CardTitle>
              </CardHeader>
              <CardContent className="space-y-2 px-4 pt-0">
                {status !== 'ready' && status !== 'error' ? (
                  <Alert
                    data-testid="triggers-custom-list-loading"
                    className="border-border/70 bg-muted/20 px-3 py-2 text-muted-foreground"
                  >
                    <AlertDescription className="text-inherit">Restoring triggers…</AlertDescription>
                  </Alert>
                ) : null}
                {filteredCustomTriggers.map((trigger) => (
                  <Button
                    key={trigger.id}
                    type="button"
                    onClick={() => setSelectedTriggerId(trigger.id)}
                    variant="outline"
                    className={`h-auto w-full justify-start rounded-md px-3 py-2 text-left ${selectedTriggerId === trigger.id ? 'border-foreground bg-muted/60' : 'border-border bg-background/70'}`}
                    >
                    <div className="w-full">
                      <div className="flex items-center justify-between gap-2">
                        <span className="text-sm font-medium">{trigger.name}</span>
                        <span className="text-[11px] text-muted-foreground">{trigger.enabled ? 'Enabled' : 'Disabled'}</span>
                      </div>
                      <div className="mt-1 flex flex-wrap items-center gap-2">
                        <Badge variant="outline" className="text-[11px]">
                          {triggerTargetSummary(trigger, activeProjectPath)}
                        </Badge>
                        <span className="text-xs text-muted-foreground">{triggerSourceSummary(trigger)}</span>
                      </div>
                    </div>
                  </Button>
                ))}
                {status === 'ready' && filteredCustomTriggers.length === 0 ? (
                  <Empty className="px-3 py-4 text-xs text-muted-foreground">
                    <EmptyHeader>
                      <EmptyDescription>No custom triggers in this scope yet.</EmptyDescription>
                    </EmptyHeader>
                  </Empty>
                ) : null}
              </CardContent>
            </Card>
          </div>

          <div className="space-y-6">
            <Card className="gap-4 py-4">
              <CardHeader className="gap-1 px-4">
                <CardTitle className="text-sm">Create trigger</CardTitle>
              </CardHeader>
              <CardContent className="px-4 pt-0">
              <TriggerEditor
                form={newTriggerForm}
                onChange={setNewTriggerForm}
                mode="create"
                protectedTrigger={false}
                activeProjectPath={activeProjectPath}
              />
              <div className="mt-4 flex justify-end">
                <Button
                  type="button"
                  data-testid="trigger-create-button"
                  onClick={() => void onCreateTrigger()}
                >
                  Create trigger
                </Button>
              </div>
              </CardContent>
            </Card>

            <Card className="gap-4 py-4">
              <CardHeader className="flex flex-row items-center justify-between gap-2 px-4">
                <div>
                  <CardTitle className="text-sm">Selected trigger</CardTitle>
                  <div className="text-sm text-muted-foreground">
                    {selectedTrigger ? selectedTrigger.id : 'Select a trigger to inspect and edit it.'}
                  </div>
                </div>
                {selectedTrigger && !selectedTrigger.protected ? (
                  <Button
                    type="button"
                    data-testid="trigger-delete-button"
                    onClick={() => void onDeleteSelectedTrigger()}
                    variant="outline"
                    size="xs"
                    className="border-destructive/40 text-destructive hover:bg-destructive/10"
                  >
                    Delete
                  </Button>
                ) : null}
              </CardHeader>

              {selectedTrigger && editTriggerForm ? (
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
                          <div className="text-xs text-muted-foreground">No trigger history yet.</div>
                        ) : null}
                      </div>
                    </div>
                  </div>

                  <div className="mt-4 flex justify-end">
                    <Button
                      type="button"
                      data-testid="trigger-save-button"
                      onClick={() => void onSaveSelectedTrigger()}
                    >
                      Save trigger
                    </Button>
                  </div>
                </CardContent>
              ) : null}
            </Card>
          </div>
        </div>
      </div>
    </section>
  )
}
