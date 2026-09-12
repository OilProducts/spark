import { ProviderSettingsEditor } from "./ProviderSettingsEditor"
import { AgentSettingsEditor } from "./AgentSettingsEditor"
import { LlmProfilesEditor, ExecutionProfilesEditor } from "./ProfileSettingsEditors"
import { ClientPreferencesEditor } from "./ClientPreferencesEditor"
import { ProjectModelSettingsEditor } from "./ProjectModelSettingsEditor"
import { useEffect, useState } from "react"
import { useStore } from "@/store"
import { useLlmProfiles } from "@/lib/useLlmProfiles"
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection } from "@/lib/llmSuggestions"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Field, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { NativeSelect } from "@/components/ui/native-select"
import { Switch } from "@/components/ui/switch"
import { useDialogController } from "@/components/app/dialog-controller"
import { useModelDiscovery } from "./hooks/useModelDiscovery"
import { Button } from "@/components/ui/button"
import { ConnectionSettingsEditor } from "./ConnectionSettingsEditor"
import { RuntimeSettingsEditor } from "./RuntimeSettingsEditor"
import { useSettingsNavigationProtection } from "./hooks/useSettingsNavigationProtection"
import { useModelSettingsEditor } from "./hooks/useModelSettingsEditor"

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

export function SettingsPanel() {
    const models = useModelSettingsEditor()
    const uiDefaults = {
        llm_provider: models.draft?.provider ?? '', llm_profile: models.draft?.llm_profile ?? '',
        llm_model: models.draft?.model ?? '', reasoning_effort: models.draft?.reasoning_effort ?? '',
    }
    const setUiDefault = (key: 'llm_model' | 'reasoning_effort', value: string) => models.setDraft((draft) => draft && ({
        ...draft, [key === 'llm_model' ? 'model' : key]: value || null,
    }))
    const activeProjectPath = useStore((state) => state.activeProjectPath)
    const { confirm } = useDialogController()
    const llmProfiles = useLlmProfiles()
    const [desktopSettings, setDesktopSettings] = useState<DesktopServerSettings | null>(null)
    const [desktopSettingsError, setDesktopSettingsError] = useState<string | null>(null)
    const [isSavingDesktopSettings, setIsSavingDesktopSettings] = useState(false)

    const [desktopMessage, setDesktopMessage] = useState('')
    const [remoteDraft, setRemoteDraft] = useState<boolean | null>(null)
    const desktopDirty = remoteDraft !== null && remoteDraft !== desktopSettings?.remote_access_enabled
    useSettingsNavigationProtection(desktopDirty || isSavingDesktopSettings)

    const [customModel, setCustomModel] = useState(false)
    const provider = uiDefaults.llm_profile || uiDefaults.llm_provider
    const providerOptions = [...new Set([...getLlmSelectionOptions(llmProfiles), provider])].filter(Boolean)
    const profile = llmProfiles.find((entry) => entry.id === provider)
    const invalidModel = profile ?
        (!!uiDefaults.llm_model && !profile.models.includes(uiDefaults.llm_model)) || (!uiDefaults.llm_model && !profile.default_model)
        : ['openrouter', 'litellm', 'openai_compatible'].includes(provider) && !uiDefaults.llm_model
    const currentDiscovery = useModelDiscovery(activeProjectPath)
    const discoveredModels = currentDiscovery?.payload?.models.filter((model) => model.provider === provider)
    const discoveryUnavailable = provider === 'codex'
        && currentDiscovery?.payload?.providers.codex.status === 'unavailable'
    const modelOptions = [...new Set(profile ? profile.models : (
        discoveredModels?.length && !discoveryUnavailable
            ? discoveredModels.map((model) => model.id)
            : getModelSuggestions(provider, llmProfiles)
    ))].filter(Boolean)
    const unlistedModel = !!uiDefaults.llm_model && !modelOptions.includes(uiDefaults.llm_model)
    const discoveryMessage = !profile && activeProjectPath
        ? (!currentDiscovery ? 'Loading models…'
            : currentDiscovery.failed || discoveryUnavailable ? 'Model discovery unavailable. Using suggestions.' : null)
        : null

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
    }, [desktopSettings?.revision, desktopDirty, isSavingDesktopSettings])

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
            setDesktopMessage('Desktop settings saved.')
            setDesktopSettingsError(null)
        } catch (error) {
            setDesktopSettingsError(error instanceof Error ? error.message : 'Unable to save desktop settings.')
        } finally {
            setIsSavingDesktopSettings(false)
        }
    }

    return (
        <div data-testid="settings-panel" className="flex-1 overflow-auto p-6">
            <div className="mx-auto w-full max-w-3xl space-y-6">
                <div className="space-y-1">
                    <h2 className="text-sm font-semibold text-foreground">Settings</h2>
                    <p className="text-xs leading-5 text-muted-foreground">
                        Model defaults apply to inheriting conversations on their next message.
                    </p>
                </div>

                <Card className="gap-4 py-4 shadow-sm">
                    <CardHeader className="gap-1 px-4">
                        <CardTitle className="text-sm">Model defaults (Workspace)</CardTitle>
                    </CardHeader>
                    <CardContent className="space-y-3 px-4 pt-0">
                        <p className="text-xs text-muted-foreground">Workspace default</p>
                        <fieldset disabled={!models.saved || models.pending} className="space-y-3">
                        <Field className="[&>[data-slot=native-select-wrapper]]:w-full">
                            <FieldLabel htmlFor="settings-default-llm-provider">
                                Default LLM Provider
                            </FieldLabel>
                            <NativeSelect
                                id="settings-default-llm-provider"
                                value={provider}
                                onChange={(event) => {
                                    const selection = splitLlmSelection(event.target.value, llmProfiles)
                                    models.setDraft({ provider: selection.llm_profile ? null : selection.llm_provider || 'codex',
                                        llm_profile: selection.llm_profile || null, model: null, reasoning_effort: null })
                                    setCustomModel(false)
                                }}
                                className="text-xs"
                            >
                                <option value="">Use provider default</option>
                                {providerOptions.map((option) => (
                                    <option key={option} value={option}>{option}</option>
                                ))}
                            </NativeSelect>
                        </Field>
                        <Field className="[&>[data-slot=native-select-wrapper]]:w-full">
                            <FieldLabel htmlFor="settings-default-llm-model">
                                Default LLM Model
                            </FieldLabel>
                            <NativeSelect
                                id="settings-default-llm-model"
                                aria-invalid={!!invalidModel}
                                value={customModel ? 'custom' : uiDefaults.llm_model ? `model:${uiDefaults.llm_model}` : ''}
                                onChange={(event) => {
                                    const value = event.target.value
                                    setCustomModel(value === 'custom')
                                    if (value !== 'custom') setUiDefault('llm_model', value.replace(/^model:/, ''))
                                }}
                                className="text-xs"
                            >
                                <option value="">Use provider default</option>
                                {modelOptions.map((option) => (
                                    <option key={option} value={`model:${option}`}>{option}</option>
                                ))}
                                {unlistedModel && (
                                    <option value={`model:${uiDefaults.llm_model}`}>{uiDefaults.llm_model} (custom)</option>
                                )}
                                <option value="custom">Custom model…</option>
                            </NativeSelect>
                            {(customModel || unlistedModel) && (
                                <>
                                    <FieldLabel htmlFor="settings-custom-llm-model">Custom model</FieldLabel>
                                    <Input
                                        id="settings-custom-llm-model"
                                        value={uiDefaults.llm_model}
                                        onChange={(event) => {
                                            setCustomModel(true)
                                            setUiDefault('llm_model', event.target.value)
                                        }}
                                        className="text-xs"
                                    />
                                </>
                            )}
                            {discoveryMessage && <p role="status" className="text-xs text-muted-foreground">{discoveryMessage}</p>}
                        </Field>
                        <Field>
                            <FieldLabel htmlFor="settings-default-reasoning-effort">
                                Default Reasoning Effort
                            </FieldLabel>
                            <NativeSelect
                                id="settings-default-reasoning-effort"
                                value={uiDefaults.reasoning_effort}
                                onChange={(event) => setUiDefault('reasoning_effort', event.target.value)}
                                className="text-xs"
                            >
                                <option value="">Use provider default</option>
                                <option value="low">Low</option>
                                <option value="medium">Medium</option>
                                <option value="high">High</option>
                                <option value="xhigh">XHigh</option>
                            </NativeSelect>
                        </Field>
                        </fieldset>
                        <div className="flex gap-2">
                            <Button size="sm" disabled={!models.dirty || models.pending || !!invalidModel} onClick={() => void models.save()}>Save model defaults</Button>
                            <Button size="sm" variant="outline" disabled={!models.saved || models.pending} onClick={() => void models.discard()}>Discard model changes</Button>
                        </div>
                        {invalidModel && <p role="alert" className="text-xs text-destructive">Choose a compatible model for this provider or profile.</p>}
                        {models.error && <p role="alert" className="text-xs text-destructive">{models.error}</p>}
                        {models.message && <p role="status" className="text-xs">{models.message}</p>}
                    </CardContent>
                </Card>

                <Card className="gap-4 py-4 shadow-sm">
                    <CardHeader className="px-4"><CardTitle className="text-sm">Scoped configuration</CardTitle></CardHeader>
                    <CardContent className="space-y-3 px-4">
                        <p className="text-xs text-muted-foreground">Edit flow launch permissions and execution locks in the flow editor’s graph settings. Manage automation in the trigger editor.</p>
                        <div className="flex gap-2">
                            <Button variant="outline" onClick={() => useStore.getState().setViewMode('editor')}>Edit flow policies</Button>
                            <Button variant="outline" onClick={() => useStore.getState().setViewMode('triggers')}>Edit triggers</Button>
                        </div>
                    </CardContent>
                </Card>
                {activeProjectPath && <ProjectModelSettingsEditor key={activeProjectPath} projectPath={activeProjectPath} />}
                <RuntimeSettingsEditor />
                <ConnectionSettingsEditor />
                <ClientPreferencesEditor />

                {desktopSettings ? (
                    <Card className="gap-4 py-4 shadow-sm">
                        <CardHeader className="gap-1 px-4"><CardTitle className="text-sm">Desktop Server</CardTitle></CardHeader>
                        <CardContent className="space-y-4 px-4 pt-0">
                            <div className="flex items-center justify-between gap-4 rounded border border-border px-3 py-2">
                                <div className="min-w-0 space-y-1">
                                    <div className="text-xs font-medium text-foreground">Remote access</div>
                                    <div className="break-all text-xs text-muted-foreground">{desktopSettings.bind_host} - {desktopSettings.server_url}</div>
                                </div>
                                <Switch data-testid="desktop-remote-access-toggle" checked={remoteDraft ?? desktopSettings.remote_access_enabled}
                                    disabled={isSavingDesktopSettings} onCheckedChange={setRemoteDraft} aria-label="Remote desktop server access" />
                            </div>
                            <div className="flex gap-2">
                                <Button disabled={!desktopDirty || isSavingDesktopSettings} onClick={() => void updateRemoteAccess(remoteDraft ?? false)}>Save Desktop settings</Button>
                                <Button variant="outline" disabled={isSavingDesktopSettings || (!desktopDirty && !desktopSettingsError)} onClick={() => {
                                    const invoke = getTauriInvoke()
                                    if (!invoke) return
                                    setIsSavingDesktopSettings(true)
                                    void invoke<DesktopServerSettings>('desktop_server_settings').then((value) => {
                                        setDesktopSettings(value); setRemoteDraft(null); setDesktopSettingsError(null); setDesktopMessage('')
                                    }).catch(() => setDesktopSettingsError('Unable to reload Desktop settings.'))
                                        .finally(() => setIsSavingDesktopSettings(false))
                                }}>Discard Desktop changes</Button>
                            </div>
                            {desktopMessage && <p role="status" className="text-xs">{desktopMessage}</p>}
                            {desktopSettings.requires_restart ? <div className="rounded border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900">Restart Spark Desktop to apply the server binding change.</div> : null}
                            {desktopSettingsError ? <div className="rounded border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">{desktopSettingsError}</div> : null}
                        </CardContent>
                    </Card>
                ) : null}
                <ProviderSettingsEditor />
                <AgentSettingsEditor />
                <LlmProfilesEditor />
                <ExecutionProfilesEditor />
            </div>
        </div>
    )
}
