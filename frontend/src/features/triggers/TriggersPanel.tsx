import { useState } from "react"

import { TriggerEditor } from "@/components/triggers/TriggerEditor"
import {
  formatTriggerTimestamp,
  SHARED_WEBHOOK_ENDPOINT,
  triggerSourceSummary,
} from "@/components/triggers/triggerForm"
import { useTriggersList } from "@/components/triggers/hooks/useTriggersList"
import { useTriggerEditor } from "@/components/triggers/hooks/useTriggerEditor"
import { useWebhookSecretRegeneration } from "@/components/triggers/hooks/useWebhookSecretRegeneration"

export function TriggersPanel() {
  const [revealedWebhookSecrets, setRevealedWebhookSecrets] = useState<Record<string, string>>({})
  const {
    customTriggers,
    error,
    loading,
    refreshTriggers,
    selectedTrigger,
    selectedTriggerId,
    setError,
    setSelectedTriggerId,
    systemTriggers,
  } = useTriggersList()
  const {
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
    setRevealedWebhookSecrets,
    setSelectedTriggerId,
  })
  const { isRegenerating, onRegenerateWebhookSecret } = useWebhookSecretRegeneration({
    refreshTriggers,
    selectedTrigger,
    setError,
    setRevealedWebhookSecrets,
  })

  return (
    <section data-testid="triggers-panel" className="flex-1 overflow-auto p-6">
      <div className="mx-auto flex w-full max-w-6xl flex-col gap-6">
        <div className="space-y-1">
          <h2 className="text-lg font-semibold">Triggers</h2>
          <p className="text-sm text-muted-foreground">
            Manage system routing, schedules, polling, flow-event automation, and shared webhook ingress.
          </p>
        </div>

        {error ? (
          <div className="rounded-md border border-destructive/30 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {error}
          </div>
        ) : null}

        <div className="grid gap-6 lg:grid-cols-[minmax(20rem,26rem)_minmax(0,1fr)]">
          <div className="space-y-4">
            <div className="rounded-md border border-border bg-card p-4 shadow-sm">
              <div className="flex items-center justify-between gap-2">
                <div>
                  <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">System triggers</div>
                  <div className="text-sm text-muted-foreground">Protected approval and review routing.</div>
                </div>
                <button
                  type="button"
                  onClick={() => void refreshTriggers()}
                  className="rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                >
                  {loading ? 'Refreshing…' : 'Refresh'}
                </button>
              </div>
              <div className="mt-3 space-y-2">
                {systemTriggers.map((trigger) => (
                  <button
                    key={trigger.id}
                    type="button"
                    data-testid={`trigger-row-${trigger.id}`}
                    onClick={() => setSelectedTriggerId(trigger.id)}
                    className={`w-full rounded-md border px-3 py-2 text-left ${selectedTriggerId === trigger.id ? 'border-foreground bg-muted/60' : 'border-border bg-background/70'}`}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm font-medium">{trigger.name}</span>
                      <span className="text-[11px] text-muted-foreground">{trigger.enabled ? 'Enabled' : 'Disabled'}</span>
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">{triggerSourceSummary(trigger)}</div>
                  </button>
                ))}
                {systemTriggers.length === 0 ? <p className="text-xs text-muted-foreground">No protected triggers configured.</p> : null}
              </div>
            </div>

            <div className="rounded-md border border-border bg-card p-4 shadow-sm">
              <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Custom triggers</div>
              <div className="mt-3 space-y-2">
                {customTriggers.map((trigger) => (
                  <button
                    key={trigger.id}
                    type="button"
                    onClick={() => setSelectedTriggerId(trigger.id)}
                    className={`w-full rounded-md border px-3 py-2 text-left ${selectedTriggerId === trigger.id ? 'border-foreground bg-muted/60' : 'border-border bg-background/70'}`}
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="text-sm font-medium">{trigger.name}</span>
                      <span className="text-[11px] text-muted-foreground">{trigger.enabled ? 'Enabled' : 'Disabled'}</span>
                    </div>
                    <div className="mt-1 text-xs text-muted-foreground">{triggerSourceSummary(trigger)}</div>
                  </button>
                ))}
                {customTriggers.length === 0 ? <p className="text-xs text-muted-foreground">No custom triggers yet.</p> : null}
              </div>
            </div>
          </div>

          <div className="space-y-6">
            <div className="rounded-md border border-border bg-card p-4 shadow-sm">
              <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Create trigger</div>
              <TriggerEditor
                form={newTriggerForm}
                onChange={setNewTriggerForm}
                mode="create"
                protectedTrigger={false}
              />
              <div className="mt-4 flex justify-end">
                <button
                  type="button"
                  data-testid="trigger-create-button"
                  onClick={() => void onCreateTrigger()}
                  className="rounded border border-border px-3 py-2 text-sm hover:bg-muted"
                >
                  Create trigger
                </button>
              </div>
            </div>

            <div className="rounded-md border border-border bg-card p-4 shadow-sm">
              <div className="flex items-center justify-between gap-2">
                <div>
                  <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">Selected trigger</div>
                  <div className="text-sm text-muted-foreground">
                    {selectedTrigger ? selectedTrigger.id : 'Select a trigger to inspect and edit it.'}
                  </div>
                </div>
                {selectedTrigger && !selectedTrigger.protected ? (
                  <button
                    type="button"
                    data-testid="trigger-delete-button"
                    onClick={() => void onDeleteSelectedTrigger()}
                    className="rounded border border-destructive/40 px-2 py-1 text-xs text-destructive hover:bg-destructive/10"
                  >
                    Delete
                  </button>
                ) : null}
              </div>

              {selectedTrigger && editTriggerForm ? (
                <>
                  <TriggerEditor
                    form={editTriggerForm}
                    onChange={setEditTriggerForm}
                    mode="edit"
                    protectedTrigger={selectedTrigger.protected}
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
                      <button
                        type="button"
                        data-testid="trigger-regenerate-secret-button"
                        onClick={() => void onRegenerateWebhookSecret()}
                        className="rounded border border-border px-2 py-1 text-xs hover:bg-muted"
                      >
                        {isRegenerating ? 'Regenerating…' : 'Regenerate secret'}
                      </button>
                    </div>
                  ) : null}

                  <div className="mt-4 grid gap-3 lg:grid-cols-2">
                    <div className="rounded-md border border-border bg-background/70 p-3 text-sm">
                      <div className="font-medium text-foreground">Runtime</div>
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
                    <button
                      type="button"
                      data-testid="trigger-save-button"
                      onClick={() => void onSaveSelectedTrigger()}
                      className="rounded border border-border px-3 py-2 text-sm hover:bg-muted"
                    >
                      Save trigger
                    </button>
                  </div>
                </>
              ) : null}
            </div>
          </div>
        </div>
      </div>
    </section>
  )
}
