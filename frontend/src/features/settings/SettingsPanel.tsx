import { useEffect, useState } from "react"
import { useStore } from "@/store"
import { fetchLlmProfiles } from "@/lib/api/llmProfilesApi"
import { fetchWorkspaceSettingsValidated, type WorkspaceSettingsResponse } from "@/lib/workspaceClient"
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection, type LlmProfileMetadata } from "@/lib/llmSuggestions"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Field, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { NativeSelect } from "@/components/ui/native-select"

export function SettingsPanel() {
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setUiDefault = useStore((state) => state.setUiDefault)
    const [llmProfiles, setLlmProfiles] = useState<LlmProfileMetadata[]>([])
    const [workspaceSettings, setWorkspaceSettings] = useState<WorkspaceSettingsResponse | null>(null)
    const [settingsError, setSettingsError] = useState<string | null>(null)

    useEffect(() => {
        void fetchLlmProfiles().then(setLlmProfiles)
        void fetchWorkspaceSettingsValidated()
            .then((payload) => {
                setWorkspaceSettings(payload)
                setSettingsError(null)
            })
            .catch((error: unknown) => {
                setWorkspaceSettings(null)
                setSettingsError(error instanceof Error ? error.message : 'Unable to load workspace settings.')
            })
    }, [])

    const executionPlacement = workspaceSettings?.execution_placement

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

                <Card className="gap-4 py-4 shadow-sm">
                    <CardHeader className="gap-1 px-4">
                        <CardTitle className="text-sm">Execution Workers</CardTitle>
                    </CardHeader>
                    <CardContent className="space-y-4 px-4 pt-0">
                        {settingsError ? (
                            <div className="rounded border border-destructive/30 bg-destructive/10 px-3 py-2 text-xs text-destructive">
                                {settingsError}
                            </div>
                        ) : null}
                        {executionPlacement ? (
                            <>
                                <div className="grid gap-3 text-xs sm:grid-cols-3">
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
                                    <div>
                                        <div className="text-muted-foreground">Protocol</div>
                                        <div className="mt-1 font-medium text-foreground">
                                            {executionPlacement.protocol.expected_worker_protocol_version}
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
                                                {profile.worker_id || profile.image ? (
                                                    <div className="mt-1 text-muted-foreground">
                                                        {[profile.worker_id ? `worker ${profile.worker_id}` : null, profile.image].filter(Boolean).join(' / ')}
                                                    </div>
                                                ) : null}
                                            </div>
                                        ))}
                                    </div>
                                </div>

                                <div className="space-y-2">
                                    <div className="text-xs font-medium text-muted-foreground">Workers</div>
                                    <div className="grid gap-2">
                                        {executionPlacement.workers.length === 0 ? (
                                            <div className="rounded border border-border px-3 py-2 text-xs text-muted-foreground">
                                                No remote workers configured.
                                            </div>
                                        ) : executionPlacement.workers.map((worker) => (
                                            <div key={worker.id} className="rounded border border-border px-3 py-2 text-xs">
                                                <div className="flex flex-wrap items-center gap-2">
                                                    <span className="font-medium text-foreground">{worker.label}</span>
                                                    <span className="text-muted-foreground">{worker.status || 'unknown'}</span>
                                                    <span className={worker.protocol_compatible ? 'text-emerald-700' : 'text-muted-foreground'}>
                                                        {worker.protocol_compatible ? 'compatible' : 'not verified'}
                                                    </span>
                                                </div>
                                                <div className="mt-1 break-all text-muted-foreground">{worker.base_url}</div>
                                                <div className="mt-1 text-muted-foreground">
                                                    worker {worker.versions.worker_version || 'unknown'} / protocol {worker.versions.protocol_version || worker.versions.expected_protocol_version}
                                                </div>
                                                {worker.health_error || worker.worker_info_error ? (
                                                    <div className="mt-1 text-destructive">
                                                        {(worker.health_error?.message as string | undefined) || (worker.worker_info_error?.message as string | undefined)}
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
