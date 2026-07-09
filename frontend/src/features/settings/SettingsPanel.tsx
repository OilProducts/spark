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
import { useWorkspaceSettings } from "./hooks/useWorkspaceSettings"

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

type DesktopServerSettings = {
    remote_access_enabled: boolean
    bind_host: string
    server_url: string
    requires_restart: boolean
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
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setUiDefault = useStore((state) => state.setUiDefault)
    const { confirm } = useDialogController()
    const llmProfiles = useLlmProfiles()
    const { workspaceSettings, settingsError } = useWorkspaceSettings()
    const [desktopSettings, setDesktopSettings] = useState<DesktopServerSettings | null>(null)
    const [desktopSettingsError, setDesktopSettingsError] = useState<string | null>(null)
    const [isSavingDesktopSettings, setIsSavingDesktopSettings] = useState(false)

    useEffect(() => {
        const invoke = getTauriInvoke()
        if (invoke) {
            void invoke<DesktopServerSettings>('desktop_server_settings')
                .then((payload) => {
                    setDesktopSettings(payload)
                    setDesktopSettingsError(null)
                })
                .catch((error: unknown) => {
                    setDesktopSettingsError(error instanceof Error ? error.message : 'Unable to load desktop settings.')
                })
        }
    }, [])

    const executionPlacement = workspaceSettings?.execution_placement
    const updateRemoteAccess = async (enabled: boolean) => {
        const invoke = getTauriInvoke()
        if (!invoke || !desktopSettings) {
            return
        }
        const confirmedWarning = enabled
            ? await confirm({
                title: 'Enable remote access?',
                description: desktopSettings.remote_access_warning,
                confirmLabel: 'Enable',
                cancelLabel: 'Cancel',
            })
            : false
        if (enabled && !confirmedWarning) {
            return
        }
        setIsSavingDesktopSettings(true)
        try {
            const payload = await invoke<DesktopServerSettings>('set_desktop_remote_access_enabled', {
                enabled,
                confirmedWarning,
            })
            setDesktopSettings(payload)
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
                        Global defaults apply to new flows and are snapshotted per flow.
                    </p>
                </div>

                <Card className="gap-4 py-4 shadow-sm">
                    <CardHeader className="gap-1 px-4">
                        <CardTitle className="text-sm">LLM Defaults (Global)</CardTitle>
                    </CardHeader>
                    <CardContent className="space-y-3 px-4 pt-0">
                        <Field>
                            <FieldLabel htmlFor="settings-default-llm-provider">
                                Default LLM Provider
                            </FieldLabel>
                            <Input
                                id="settings-default-llm-provider"
                                value={uiDefaults.llm_profile || uiDefaults.llm_provider}
                                onChange={(event) => {
                                    const selection = splitLlmSelection(event.target.value, llmProfiles)
                                    setUiDefault('llm_provider', selection.llm_provider)
                                    setUiDefault('llm_profile', selection.llm_profile)
                                }}
                                list="settings-llm-provider-options"
                                className="text-xs"
                                placeholder="openai"
                            />
                            <datalist id="settings-llm-provider-options">
                                {getLlmSelectionOptions(llmProfiles).map((provider) => (
                                    <option key={provider} value={provider} />
                                ))}
                            </datalist>
                        </Field>
                        <Field>
                            <FieldLabel htmlFor="settings-default-llm-model">
                                Default LLM Model
                            </FieldLabel>
                            <Input
                                id="settings-default-llm-model"
                                value={uiDefaults.llm_model}
                                onChange={(event) => setUiDefault('llm_model', event.target.value)}
                                list="settings-llm-model-options"
                                className="text-xs"
                                placeholder="gpt-5.5"
                            />
                            <datalist id="settings-llm-model-options">
                                {getModelSuggestions(uiDefaults.llm_profile || uiDefaults.llm_provider, llmProfiles).map((modelOption) => (
                                    <option key={modelOption} value={modelOption} />
                                ))}
                            </datalist>
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
                                <option value="">Use handler default</option>
                                <option value="low">Low</option>
                                <option value="medium">Medium</option>
                                <option value="high">High</option>
                                <option value="xhigh">XHigh</option>
                            </NativeSelect>
                        </Field>
                    </CardContent>
                </Card>

                {desktopSettings ? (
                    <Card className="gap-4 py-4 shadow-sm">
                        <CardHeader className="gap-1 px-4">
                            <CardTitle className="text-sm">Desktop Server</CardTitle>
                        </CardHeader>
                        <CardContent className="space-y-4 px-4 pt-0">
                            <div className="flex items-center justify-between gap-4 rounded border border-border px-3 py-2">
                                <div className="min-w-0 space-y-1">
                                    <div className="text-xs font-medium text-foreground">Remote access</div>
                                    <div className="break-all text-xs text-muted-foreground">
                                        {desktopSettings.bind_host} - {desktopSettings.server_url}
                                    </div>
                                </div>
                                <Switch
                                    data-testid="desktop-remote-access-toggle"
                                    checked={desktopSettings.remote_access_enabled}
                                    disabled={isSavingDesktopSettings}
                                    onCheckedChange={updateRemoteAccess}
                                    aria-label="Remote desktop server access"
                                />
                            </div>
                            {desktopSettings.requires_restart ? (
                                <div className="rounded border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900">
                                    Restart Spark Desktop to apply the server binding change.
                                </div>
                            ) : null}
                            {desktopSettingsError ? (
                                <div className="rounded border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                                    {desktopSettingsError}
                                </div>
                            ) : null}
                        </CardContent>
                    </Card>
                ) : null}

                <Card className="gap-4 py-4 shadow-sm">
                    <CardHeader className="gap-1 px-4">
                        <CardTitle className="text-sm">Execution Profiles</CardTitle>
                    </CardHeader>
                    <CardContent className="space-y-4 px-4 pt-0">
                        {settingsError ? (
                            <div className="rounded border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                                {settingsError}
                            </div>
                        ) : null}
                        {executionPlacement ? (
                            <>
                                <div className="grid gap-3 text-xs sm:grid-cols-2">
                                    <div>
                                        <div className="text-muted-foreground">Modes</div>
                                        <div className="mt-1 font-medium text-foreground">
                                            {executionPlacement.execution_modes.join(', ')}
                                        </div>
                                    </div>
                                    <div>
                                        <div className="text-muted-foreground">Default Profile</div>
                                        <div className="mt-1 font-medium text-foreground">
                                            {executionPlacement.default_execution_profile_id || 'runtime default'}
                                        </div>
                                    </div>
                                </div>

                                {executionPlacement.validation_errors.length > 0 ? (
                                    <div className="space-y-2">
                                        {executionPlacement.validation_errors.map((error, index) => (
                                            <div key={`${error.field || 'config'}-${index}`} className="rounded border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                                                <span className="font-medium">{error.field || 'execution-profiles.toml'}</span>: {error.message}
                                            </div>
                                        ))}
                                    </div>
                                ) : null}

                                <div className="space-y-2">
                                    <div className="text-xs font-medium text-muted-foreground">Profiles</div>
                                    <div className="grid gap-2">
                                        {executionPlacement.profiles.map((profile) => (
                                            <div key={profile.id || profile.label || 'profile'} className="rounded border border-border px-3 py-2 text-xs">
                                                <div className="flex flex-wrap items-center gap-2">
                                                    <span className="font-medium text-foreground">{profile.label || profile.id}</span>
                                                    <span className="rounded bg-muted px-2 py-0.5 text-muted-foreground">{profile.mode}</span>
                                                    <span className={profile.enabled ? 'text-emerald-700' : 'text-muted-foreground'}>
                                                        {profile.enabled ? 'enabled' : 'disabled'}
                                                    </span>
                                                </div>
                                                {profile.image ? (
                                                    <div className="mt-1 text-muted-foreground">
                                                        {profile.image}
                                                    </div>
                                                ) : null}
                                            </div>
                                        ))}
                                    </div>
                                </div>
                            </>
                        ) : null}
                    </CardContent>
                </Card>
            </div>
        </div>
    )
}
