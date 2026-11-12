import { Plus, RefreshCw, Zap } from "lucide-react"
import { useEffect, useMemo, useState } from "react"

import { TriggerEditor } from "./components/TriggerEditor"
import {
  createEmptyTriggerForm,
  formatTriggerResult,
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
import { fetchTriggerIssuesValidated, type TriggerIssueResponse } from "@/lib/api/triggersApi"
import { defaultProjectChoice, isUnregisteredTarget, recentProjectPath } from "@/features/projects/model/projectChoices"
import { SheetEmpty, ViewLayout } from "@/components/app/view-layout"
import { InlineError } from "@/components/app/inline-error"
import { SectionHeading } from "@/components/app/section-heading"
import { ListGroupLabel, SidePanelRow } from "@/components/app/side-panel-row"
import { Well } from "@/components/app/well"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
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
    editTriggerErrors,
    newTriggerForm,
    newTriggerErrors,
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
  // Definitions that cannot run are missing from the list; show why.
  const [issues, setIssues] = useState<TriggerIssueResponse[]>([])
  useEffect(() => {
    if (status !== 'ready') {
      return
    }
    let disposed = false
    void fetchTriggerIssuesValidated()
      .then((next) => { if (!disposed) setIssues(next) })
      .catch(() => { if (!disposed) setIssues([]) })
    return () => { disposed = true }
  }, [status, triggersSession.triggers])

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
    // An untouched draft starts in the project worked in most recently, as Run a flow does.
    if (!newTriggerForm.name.trim() && !newTriggerForm.projectPath) {
      setTriggersSessionNewDraft({ form: createEmptyTriggerForm(defaultProjectChoice(projectRegistry, recentProjectPath(projectRegistry, useStore.getState().runsListSession.runs))) })
    }
    updateTriggersSession({ createFormOpen: true })
  }
  const triggerGroups = [
    { title: 'Custom', note: null, triggers: customTriggers, loadingTestId: 'triggers-custom-list-loading', emptyText: 'No custom triggers yet.' },
    { title: 'System', note: 'Built in: they route approvals and reviews, and cannot be deleted.', triggers: systemTriggers, loadingTestId: 'triggers-system-list-loading', emptyText: 'No protected triggers.' },
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
            <p data-testid={group.loadingTestId} className="px-3 py-1 text-sm text-muted-foreground" aria-live="polite">Restoring triggers…</p>
          ) : null}
          {group.triggers.map((trigger) => {
            const unregistered = isUnregisteredTarget(projectRegistry, trigger.project_path)
            const activationError = trigger.state.activation_error
            return (
              <SidePanelRow
                key={triggerKey(trigger)}
                data-testid={`trigger-row-${trigger.id}`}
                data-unregistered-target={unregistered ? 'true' : undefined}
                selected={selectedTriggerId === triggerKey(trigger) && !createFormOpen}
                // A disabled trigger is the exception: hollow dot plus the word.
                // One the server could not turn on is a problem: red dot plus why.
                tone={activationError ? 'danger' : 'neutral'}
                hollow={!trigger.enabled && !activationError}
                title={trigger.name}
                meta={(
                  <>
                    {/* First, so a narrow row never truncates it away. */}
                    {activationError ? <span className="text-destructive" title={activationError}>Not running · </span> : null}
                    <span
                      data-testid="trigger-row-project"
                      title={trigger.project_path}
                      className={unregistered ? 'text-warning' : undefined}
                    >
                      {triggerProjectSummary(trigger, projectRegistry)}
                    </span>
                    {' · '}{triggerSourceSummary(trigger)}
                    {trigger.enabled || activationError ? null : ' · Disabled'}
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
            <p className="px-3 py-1 text-sm text-muted-foreground">{group.emptyText}</p>
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
          <Button type="button" variant="ghost" size="icon-sm" aria-label="Refresh triggers" title="Refresh triggers" onClick={() => void refreshTriggers()}>
            <RefreshCw aria-hidden="true" className={loading ? "animate-spin" : undefined} />
          </Button>
          <Button type="button" data-testid="trigger-new-button" variant="outline" size="sm" aria-label="New trigger" onClick={openCreateForm}>
            <Plus aria-hidden="true" />
            New
          </Button>
        </>
      )}
      panel={panel}
      selected={createFormOpen ? 'new' : selectedTriggerId}
    >
    <section data-testid="triggers-panel" className="flex h-full flex-col overflow-auto p-6">
      <div className="flex w-full max-w-3xl flex-col gap-6">
        {error ? (
          <InlineError>{error}</InlineError>
        ) : null}

        {issues.length > 0 ? (
          <InlineError title="Some triggers can't be loaded" data-testid="trigger-issues">
            <ul className="space-y-1">
              {issues.map((issue) => (
                <li key={`${issue.project_path}\u0000${issue.trigger_id ?? ''}`}>
                  {issue.trigger_id ?? 'Trigger files'} in {issue.project_path}: {issue.error}
                </li>
              ))}
            </ul>
          </InlineError>
        ) : null}

        {createFormOpen ? (
            <Card className="gap-6 border-t-0 py-0">
              <CardHeader className="px-0">
                <CardTitle role="heading" aria-level={1} className="text-xl font-semibold tracking-tight">New trigger</CardTitle>
              </CardHeader>
              <CardContent className="px-0 pt-0">
                <TriggerEditor
                  form={newTriggerForm}
                  onChange={setNewTriggerForm}
                  mode="create"
                  protectedTrigger={false}
                  errors={newTriggerErrors}
                />
                <div className="mt-6 flex flex-wrap gap-2">
                  <Button
                    type="button"
                    data-testid="trigger-create-button"
                    size="sm"
                    disabled={pending || isRegenerating || !newTriggerForm.projectPath}
                    onClick={() => void onCreateTrigger()}
                  >
                    Create trigger
                  </Button>
                  {/* Close keeps the draft for the next New, like the mission editor's Close. */}
                  <Button
                    type="button"
                    aria-label="Close new trigger"
                    data-testid="trigger-create-cancel-button"
                    variant="ghost"
                    size="sm"
                    disabled={pending}
                    onClick={() => updateTriggersSession({ createFormOpen: false })}
                  >
                    Close
                  </Button>
                </div>
              </CardContent>
            </Card>
          ) : selectedTrigger && editTriggerForm ? (
            <Card className="gap-6 border-t-0 py-0">
              <CardHeader className="flex flex-row items-start justify-between gap-2 px-0">
                <div className="min-w-0">
                  <CardTitle role="heading" aria-level={1} className="truncate text-xl leading-snug font-semibold tracking-tight">{selectedTrigger.name}</CardTitle>
                  <div className="mt-1 text-sm text-muted-foreground">
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
                    size="sm"
                  >
                    Delete
                  </Button>
                ) : null}
              </CardHeader>

              <CardContent className="px-0 pt-0">
                {selectedTrigger.state.activation_error ? (
                  <InlineError title="Not running: Spark couldn't turn this trigger on" data-testid="trigger-activation-error" className="mb-6">
                    <p>{selectedTrigger.state.activation_error}</p>
                    <p>To try again, fix the problem, turn on Enabled and save.</p>
                  </InlineError>
                ) : null}
                <TriggerEditor
                  form={editTriggerForm}
                  onChange={setEditTriggerForm}
                  mode="edit"
                  protectedTrigger={selectedTrigger.protected}
                  errors={editTriggerErrors}
                />

                {selectedTrigger.source_type === 'webhook' ? (
                  <Well className="mt-6 space-y-2 text-sm">
                    <SectionHeading>Calling this webhook</SectionHeading>
                    <div className="text-muted-foreground">Send a POST request with a JSON body to <code>{SHARED_WEBHOOK_ENDPOINT}</code> and these headers:</div>
                    <div className="font-mono text-xs text-foreground">
                      X-Spark-Webhook-Key: {String(selectedTrigger.source.webhook_key ?? '')}
                    </div>
                    <div className="font-mono text-xs text-foreground">
                      X-Spark-Webhook-Secret: {revealedWebhookSecrets[triggerKey(selectedTrigger)] ?? <span className="font-sans text-muted-foreground">shown only once, when created. Regenerate it to get a new one.</span>}
                    </div>
                    <Button
                      type="button"
                      data-testid="trigger-regenerate-secret-button"
                      onClick={() => void onRegenerateWebhookSecret()}
                      variant="outline"
                      size="sm"
                    >
                      {isRegenerating ? 'Regenerating…' : 'Regenerate secret'}
                    </Button>
                  </Well>
                ) : null}

                <div className="mt-6 grid gap-4 sm:grid-cols-2">
                  <Well className="text-sm">
                    <SectionHeading>Status</SectionHeading>
                    {/* The Project field names a registered project; only a lost one needs saying here. */}
                    {isUnregisteredTarget(projectRegistry, selectedTrigger.project_path) ? (
                      <div className="mt-2 text-warning">{triggerProjectSummary(selectedTrigger, projectRegistry)}</div>
                    ) : null}
                    <div className="mt-2 text-muted-foreground">Last triggered: {formatTriggerTimestamp(selectedTrigger.state.last_fired_at)}</div>
                    <div className="text-muted-foreground">Next run: {formatTriggerTimestamp(selectedTrigger.state.next_run_at)}</div>
                    <div className="text-muted-foreground">Last result: {formatTriggerResult(selectedTrigger.state.last_result)}</div>
                    {selectedTrigger.state.last_error ? (
                      <div className="mt-2 text-destructive">{selectedTrigger.state.last_error}</div>
                    ) : null}
                  </Well>
                  <Well className="text-sm">
                    <SectionHeading>Recent history</SectionHeading>
                    <div className="mt-2 space-y-2">
                      {selectedTrigger.state.recent_history.slice(0, 5).map((entry) => (
                        <div key={`${entry.timestamp}-${entry.status}`}>
                          <div className="text-xs text-foreground">{formatTriggerResult(entry.status)}</div>
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

                {externalChange ? <p role="status" className="mt-4 text-sm">This trigger changed elsewhere. Your draft is retained; discard to reload.</p> : null}
                <div className="mt-6 flex flex-wrap gap-2">
                  <Button aria-label="Save selected trigger"
                    type="button"
                    data-testid="trigger-save-button"
                    size="sm"
                    disabled={pending || isRegenerating}
                    onClick={() => void onSaveSelectedTrigger()}
                  >
                    Save
                  </Button>
                  <Button aria-label="Discard selected trigger changes" type="button" variant="ghost" size="sm" disabled={!dirty || pending || isRegenerating} onClick={discard}>Discard</Button>
                </div>
              </CardContent>
            </Card>
          ) : null}
      </div>
      {!createFormOpen && !(selectedTrigger && editTriggerForm) ? (
        <SheetEmpty icon={Zap} title="No trigger selected" className="h-auto flex-1">
          Triggers start flows on a schedule, on events, or from webhooks. Select one or create a new one.
        </SheetEmpty>
      ) : null}
    </section>
    </ViewLayout>
  )
}
