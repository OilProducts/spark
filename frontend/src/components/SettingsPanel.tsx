import { useStore } from "@/store"
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from "@/lib/llmSuggestions"

export function SettingsPanel() {
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setUiDefault = useStore((state) => state.setUiDefault)

    return (
        <div data-testid="settings-panel" className="flex-1 overflow-auto p-6">
            <div className="mx-auto w-full max-w-2xl space-y-6">
                <div className="space-y-1">
                    <h2 className="text-lg font-semibold">Settings</h2>
                    <p className="text-sm text-muted-foreground">
                        Global defaults apply to new flows and are snapshotted per flow.
                    </p>
                </div>

                <div className="rounded-md border border-border bg-card p-4 shadow-sm">
                    <div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
                        LLM Defaults (Global)
                    </div>
                    <div className="mt-4 space-y-3">
                        <div className="space-y-1">
                            <label htmlFor="settings-default-llm-provider" className="text-xs font-medium text-foreground">
                                Default LLM Provider
                            </label>
                            <input
                                id="settings-default-llm-provider"
                                value={uiDefaults.llm_provider}
                                onChange={(event) => setUiDefault('llm_provider', event.target.value)}
                                list="settings-llm-provider-options"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                placeholder="openai"
                            />
                            <datalist id="settings-llm-provider-options">
                                {LLM_PROVIDER_OPTIONS.map((provider) => (
                                    <option key={provider} value={provider} />
                                ))}
                            </datalist>
                        </div>
                        <div className="space-y-1">
                            <label htmlFor="settings-default-llm-model" className="text-xs font-medium text-foreground">
                                Default LLM Model
                            </label>
                            <input
                                id="settings-default-llm-model"
                                value={uiDefaults.llm_model}
                                onChange={(event) => setUiDefault('llm_model', event.target.value)}
                                list="settings-llm-model-options"
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                                placeholder="gpt-5.2"
                            />
                            <datalist id="settings-llm-model-options">
                                {getModelSuggestions(uiDefaults.llm_provider).map((modelOption) => (
                                    <option key={modelOption} value={modelOption} />
                                ))}
                            </datalist>
                        </div>
                        <div className="space-y-1">
                            <label htmlFor="settings-default-reasoning-effort" className="text-xs font-medium text-foreground">
                                Default Reasoning Effort
                            </label>
                            <select
                                id="settings-default-reasoning-effort"
                                value={uiDefaults.reasoning_effort}
                                onChange={(event) => setUiDefault('reasoning_effort', event.target.value)}
                                className="h-8 w-full rounded-md border border-input bg-background px-2 text-xs shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                            >
                                <option value="">Use handler default</option>
                                <option value="low">Low</option>
                                <option value="medium">Medium</option>
                                <option value="high">High</option>
                            </select>
                        </div>
                    </div>
                </div>
            </div>
        </div>
    )
}
