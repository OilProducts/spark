import { useStore } from "@/store"
import { getModelSuggestions, LLM_PROVIDER_OPTIONS } from "@/lib/llmSuggestions"
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card"
import { Field, FieldLabel } from "@/components/ui/field"
import { Input } from "@/components/ui/input"
import { NativeSelect } from "@/components/ui/native-select"

export function SettingsPanel() {
    const uiDefaults = useStore((state) => state.uiDefaults)
    const setUiDefault = useStore((state) => state.setUiDefault)

    return (
        <div data-testid="settings-panel" className="flex-1 overflow-auto p-6">
            <div className="mx-auto w-full max-w-2xl space-y-6">
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
                                value={uiDefaults.llm_provider}
                                onChange={(event) => setUiDefault('llm_provider', event.target.value)}
                                list="settings-llm-provider-options"
                                className="text-xs"
                                placeholder="openai"
                            />
                            <datalist id="settings-llm-provider-options">
                                {LLM_PROVIDER_OPTIONS.map((provider) => (
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
                                placeholder="gpt-5.2"
                            />
                            <datalist id="settings-llm-model-options">
                                {getModelSuggestions(uiDefaults.llm_provider).map((modelOption) => (
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
                            </NativeSelect>
                        </Field>
                    </CardContent>
                </Card>
            </div>
        </div>
    )
}
