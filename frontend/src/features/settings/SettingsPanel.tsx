import { isModelSelectionValid } from '@/lib/llmSuggestions'
import { ModelChooser } from '@/components/model-chooser/ModelChooser'
import { Tabs, TabsList, TabsTrigger, TabsContent } from '@/components/ui/tabs'
import { ProviderSettingsEditor } from "./ProviderSettingsEditor"
import { CodexConnectionSettings } from "./CodexConnectionSettings"
import { ClaudeCodeConnectionSettings } from "./ClaudeCodeConnectionSettings"
import { AgentSettingsEditor } from "./AgentSettingsEditor"
import { LlmProfilesEditor, ExecutionProfilesEditor } from "./ProfileSettingsEditors"
import { ClientPreferencesEditor } from "./ClientPreferencesEditor"
import { UtilityModelSettingsEditor } from "./UtilityModelSettingsEditor"
import { useEffect, useState } from "react"
import { useStore } from "@/store"
import { useLlmProfiles } from "@/lib/useLlmProfiles"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import { Switch } from "@/components/ui/switch"
import { useDialogController } from "@/components/app/dialog-controller"
import { Button } from "@/components/ui/button"
import { ConnectionSettingsEditor } from "./ConnectionSettingsEditor"
import { RuntimeSettingsEditor } from "./RuntimeSettingsEditor"
import { useSettingsNavigationProtection } from "./hooks/useSettingsNavigationProtection"
import { useModelSettingsEditor } from "./hooks/useModelSettingsEditor"
import { SaveStatus } from './SaveStatus'
import { ViewLayout } from '@/components/app/view-layout'

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

type DesktopServerSettings = {
    remote_access_enabled: boolean
    bind_host: string
    server_url: string
    requires_restart: boolean
    revision: string
    remote_access_warning: string
}

declare global {
    interface Window {
        __TAURI__?: {
            core?: {
                invoke?: TauriInvoke
            }
        }
    }
}

function getTauriInvoke(): TauriInvoke | null {
    return window.__TAURI__?.core?.invoke ?? null
}

// Category rows look like the other side panels' rows.
const categoryRow = 'h-auto flex-none rounded-none px-2 py-1.5 after:hidden hover:text-primary data-[state=active]:text-primary data-[state=active]:shadow-[inset_2px_0_0_hsl(var(--primary))]'

export function SettingsPanel() {
    const models = useModelSettingsEditor()
    const { confirm } = useDialogController()
    const llmProfiles = useLlmProfiles()
    const [desktopSettings, setDesktopSettings] = useState<DesktopServerSettings | null>(null)
    const [desktopSettingsError, setDesktopSettingsError] = useState<string | null>(null)
    const [isSavingDesktopSettings, setIsSavingDesktopSettings] = useState(false)

    const [desktopMessage, setDesktopMessage] = useState('')
    const [remoteDraft, setRemoteDraft] = useState<boolean | null>(null)
    const desktopDirty = remoteDraft !== null && remoteDraft !== desktopSettings?.remote_access_enabled
    useSettingsNavigationProtection(desktopDirty, isSavingDesktopSettings)

    const invalidModel = !!models.draft && !isModelSelectionValid(models.draft.llm_profile || models.draft.provider || '', models.draft.model, llmProfiles)
    const [category, setCategory] = useState('models')
    const [desktopRetry, setDesktopRetry] = useState(0)

    useEffect(() => {
        const invoke = getTauriInvoke()
        if (!invoke) return
        let cancelled = false
        const refresh = () => {
            if (isSavingDesktopSettings) return
            void invoke<DesktopServerSettings>('desktop_server_settings').then((payload) => {
                if (cancelled || payload.revision === desktopSettings?.revision) return
                if (desktopDirty) {
                    setDesktopMessage('Desktop settings changed elsewhere. Your draft is retained; Discard reloads the latest values.')
                } else {
                    setDesktopSettings(payload)
                    setRemoteDraft(null)
                    setDesktopSettingsError(null)
                    setDesktopMessage('')
                }
            }).catch((error: unknown) => {
                if (!cancelled) setDesktopSettingsError(error instanceof Error ? error.message : 'Unable to load desktop settings.')
            })
        }
        refresh()
        window.addEventListener('spark:settings-live-event', refresh)
        window.addEventListener('focus', refresh)
        return () => {
            cancelled = true
            window.removeEventListener('spark:settings-live-event', refresh)
            window.removeEventListener('focus', refresh)
        }
    }, [desktopSettings?.revision, desktopDirty, isSavingDesktopSettings, desktopRetry])

    const updateRemoteAccess = async (enabled: boolean) => {
        const invoke = getTauriInvoke()
        if (!invoke || !desktopSettings || isSavingDesktopSettings) {
            return
        }
        setIsSavingDesktopSettings(true)
        try {
            const confirmedWarning = enabled
                ? await confirm({
                    title: 'Enable remote access?',
                    description: desktopSettings.remote_access_warning,
                    confirmLabel: 'Enable',
                    cancelLabel: 'Cancel',
                })
                : false
            if (enabled && !confirmedWarning) return
            const payload = await invoke<DesktopServerSettings>('set_desktop_remote_access_enabled', {
                enabled,
                confirmedWarning,
                expectedRevision: desktopSettings.revision,
            })
            setDesktopSettings(payload)
            setRemoteDraft(null)
            setDesktopMessage('Saved.')
            setDesktopSettingsError(null)
        } catch (error) {
            setDesktopSettingsError(error instanceof Error ? error.message : 'Unable to save desktop settings.')
        } finally {
            setIsSavingDesktopSettings(false)
        }
    }

    // The categories are the side panel; the chosen category fills the main area.
    return (
        <Tabs className="absolute inset-0 flex-col gap-0" orientation="vertical" value={category} onValueChange={setCategory}>
        <ViewLayout
            view="settings"
            title="Settings"
            panel={(
                <TabsList className="w-full items-stretch px-2" aria-label="Settings categories">
                    <TabsTrigger className={categoryRow} value="models">Models &amp; accounts</TabsTrigger>
                    <TabsTrigger className={categoryRow} value="preferences">Preferences</TabsTrigger>
                    <TabsTrigger className={categoryRow} value="execution">Execution</TabsTrigger>
                    <TabsTrigger className={categoryRow} value="system">System</TabsTrigger>
                </TabsList>
            )}
        >
        <div data-testid="settings-panel" className="absolute inset-0 overflow-auto p-3 sm:p-6 [overflow-wrap:anywhere] [&_fieldset]:min-w-0 [&_summary]:cursor-pointer [&_summary]:rounded [&_summary]:py-2 [&_summary]:focus-visible:outline-2 [&_details>div]:min-w-0 [&_[data-slot=button]]:max-w-full [&_[data-slot=button]]:whitespace-normal [&_[data-slot=button]]:h-auto [&_[data-slot=button]]:min-h-8 [&_[data-slot=card]]:gap-4 [&_[data-slot=card]]:py-4 [&_[data-slot=card-header]]:px-4 [&_[data-slot=card-content]]:px-4">
            <div className="mx-auto w-full max-w-3xl space-y-6">
                <p className="text-sm text-muted-foreground">
                    Model defaults apply to inheriting conversations on their next message.
                </p>

                <TabsContent value="models" forceMount hidden={category !== 'models'} className="space-y-6">
                <CodexConnectionSettings />
                <ClaudeCodeConnectionSettings />

                <Card className="gap-4 py-4">
                    <CardHeader className="gap-1 px-4">
                        <h3 className="text-lg font-light">Model defaults (Workspace)</h3>
                    </CardHeader>
                    <CardContent className="space-y-3 px-4 pt-0">
                        <p className="text-xs text-muted-foreground">Workspace-wide defaults for inheriting projects and conversations.</p>
                        <fieldset disabled={!models.saved || models.pending} className="space-y-3">
                        {models.pending ? <p role="status">Saving or reloading settings…</p> : !models.saved && !models.error ? <p role="status">Loading settings…</p> : null}
                        {/* The workspace has no parent: its default is the stored Codex group, whose effort can still be set. */}
                        <ModelChooser disabled={!models.saved || models.pending} inherited={{ provider: 'codex', llm_profile: null, model: null, reasoning_effort: null }} value={models.draft ?? { provider: null, llm_profile: null, model: null, reasoning_effort: null }} onChange={next => models.setDraft(next.provider || next.llm_profile ? next : { ...next, provider: 'codex' })} projectPath={null} inheritLabel="Provider default" invalidModel={!!invalidModel} />
                        </fieldset>
                        <div className="flex flex-wrap gap-2">
                            <Button aria-label="Save workspace model defaults" size="sm" disabled={!models.dirty || models.pending || !!invalidModel} onClick={() => void models.save()}>Save</Button>
                            <Button aria-label="Discard workspace model defaults changes" size="sm" variant="outline" disabled={!models.saved || models.pending} onClick={() => void models.discard()}>Discard</Button>
                        </div>
                        {!models.draft && models.saved?.repair_defaults && <Button variant="outline" disabled={models.pending} onClick={() => models.setDraft(models.saved!.repair_defaults!)}>Start replacement draft with defaults</Button>}
            {models.saved?.validation_errors?.map((error) => <p role="alert" key={error}>{error}</p>)}
                        <SaveStatus message={models.message} error={models.error} dirty={models.dirty} />
                    </CardContent>
                </Card>

                <UtilityModelSettingsEditor projectPath={null} />
                <ProviderSettingsEditor />
                <LlmProfilesEditor />
                </TabsContent>
                <TabsContent value="preferences" forceMount hidden={category !== 'preferences'} className="space-y-6"><ClientPreferencesEditor /></TabsContent>
                <TabsContent value="execution" forceMount hidden={category !== 'execution'} className="space-y-6">
                <ExecutionProfilesEditor />
                <AgentSettingsEditor />
                <Card className="gap-4 py-4">
                    <CardHeader className="px-4"><h3 className="text-lg font-light">Scoped configuration</h3></CardHeader>
                    <CardContent className="space-y-3 px-4">
                        <p className="text-xs text-muted-foreground">Edit flow launch permissions and execution locks in the flow editor’s graph settings. Manage automation in the trigger editor.</p>
                        <div className="flex flex-wrap gap-2">
                            <Button variant="outline" onClick={() => useStore.getState().setViewMode('editor')}>Open flow editor</Button>
                            <Button variant="outline" onClick={() => useStore.getState().setViewMode('triggers')}>Edit triggers</Button>
                        </div>
                    </CardContent>
                </Card>
                </TabsContent>
                <TabsContent value="system" forceMount hidden={category !== 'system'} className="space-y-6">
                <p className="text-xs text-muted-foreground">Server/Desktop configuration. Environment overrides take precedence over saved values.</p>
                {getTauriInvoke() ? (
                    <Card className="gap-4 py-4">
                        <CardHeader className="gap-1 px-4"><h3 className="text-lg font-light">Desktop Server</h3></CardHeader>
                        <CardContent className="space-y-4 px-4 pt-0">
                            {desktopSettings ? <>
                            <div className="flex items-center justify-between gap-4 rounded border border-border px-3 py-2">
                                <div className="min-w-0 space-y-1">
                                    <div className="text-sm font-medium text-foreground">Remote access</div>
                                    <div className="break-all text-xs text-muted-foreground">{desktopSettings.bind_host} - {desktopSettings.server_url}</div>
                                </div>
                                <Switch data-testid="desktop-remote-access-toggle" checked={remoteDraft ?? desktopSettings.remote_access_enabled}
                                    disabled={isSavingDesktopSettings} onCheckedChange={setRemoteDraft} aria-label="Remote desktop server access" />
                            </div>
                            <div className="flex flex-wrap gap-2">
                                <Button aria-label="Save Desktop Server settings" disabled={!desktopDirty || isSavingDesktopSettings} onClick={() => void updateRemoteAccess(remoteDraft ?? false)}>Save</Button>
                                <Button aria-label="Discard Desktop Server changes" variant="outline" disabled={isSavingDesktopSettings || (!desktopDirty && !desktopSettingsError)} onClick={() => {
                                    const invoke = getTauriInvoke()
                                    if (!invoke) return
                                    setIsSavingDesktopSettings(true)
                                    void invoke<DesktopServerSettings>('desktop_server_settings').then((value) => {
                                        setDesktopSettings(value); setRemoteDraft(null); setDesktopSettingsError(null); setDesktopMessage('')
                                    }).catch(() => setDesktopSettingsError('Unable to reload Desktop settings.'))
                                        .finally(() => setIsSavingDesktopSettings(false))
                                }}>Discard</Button>
                            </div>
                            {isSavingDesktopSettings && <p role="status">Saving or reloading settings…</p>}
                            {desktopSettings.requires_restart ? <div className="border-0 border-l border-warning px-3 py-2 text-xs text-warning">Restart Spark Desktop to apply the server binding change.</div> : null}
                            </> : desktopSettingsError ? <Button variant="outline" onClick={() => { setDesktopSettingsError(null); setDesktopRetry((value) => value + 1) }}>Retry Desktop settings</Button> : <p role="status">Loading Desktop settings…</p>}
                            <SaveStatus message={desktopMessage} error={desktopSettingsError} dirty={desktopDirty} />
                        </CardContent>
                    </Card>
                ) : null}
                <ConnectionSettingsEditor />
                <RuntimeSettingsEditor />
                </TabsContent>
            </div>
        </div>
        </ViewLayout>
        </Tabs>
    )
}
