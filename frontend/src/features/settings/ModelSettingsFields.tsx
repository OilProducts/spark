import { useState, useId } from 'react'
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection, type LlmProfileMetadata } from '@/lib/llmSuggestions'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { useModelDiscovery } from './hooks/useModelDiscovery'
import type { useModelSettingsEditor } from './hooks/useModelSettingsEditor'

export function ModelSettingsFields({ models, activeProjectPath, invalidModel, profiles: llmProfiles }: { profiles: LlmProfileMetadata[]; models: ReturnType<typeof useModelSettingsEditor>; activeProjectPath: string | null; invalidModel: boolean }) {
    const id = useId()
    const uiDefaults = {
        llm_provider: models.draft?.provider ?? '', llm_profile: models.draft?.llm_profile ?? '',
        llm_model: models.draft?.model ?? '', reasoning_effort: models.draft?.reasoning_effort ?? '',
    }
    const setUiDefault = (key: 'llm_model' | 'reasoning_effort', value: string) => models.setDraft((draft) => draft && ({
        ...draft, [key === 'llm_model' ? 'model' : key]: value || null,
    }))
    const [customModel, setCustomModel] = useState(false)
    const provider = uiDefaults.llm_profile || uiDefaults.llm_provider
    const providerOptions = [...new Set([...getLlmSelectionOptions(llmProfiles), provider])].filter(Boolean)
    const profile = llmProfiles.find((entry) => entry.id === provider)
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

    return <>
        <Field className="[&>[data-slot=native-select-wrapper]]:w-full">
            <FieldLabel htmlFor={`${id}-settings-default-llm-provider`}>
                Provider or profile
            </FieldLabel>
            <NativeSelect
                id={`${id}-settings-default-llm-provider`}
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
                {providerOptions.map((option) => {
                    const label = llmProfiles.find((entry) => entry.id === option)?.label
                    return <option key={option} value={option}>{label ? `${label} (${option})` : option}</option>
                })}
            </NativeSelect>
        </Field>
        <Field className="[&>[data-slot=native-select-wrapper]]:w-full">
            <FieldLabel htmlFor={`${id}-settings-default-llm-model`}>
                Model
            </FieldLabel>
            <NativeSelect
                id={`${id}-settings-default-llm-model`}
                aria-invalid={!!invalidModel} aria-describedby={invalidModel ? `${id}-error` : undefined}
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
                    <FieldLabel htmlFor={`${id}-settings-custom-llm-model`}>Custom model</FieldLabel>
                    <Input
                        id={`${id}-settings-custom-llm-model`}
                        aria-invalid={!!invalidModel} aria-describedby={invalidModel ? `${id}-error` : undefined}
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
            <FieldLabel htmlFor={`${id}-settings-default-reasoning-effort`}>
                Reasoning effort
            </FieldLabel>
            <NativeSelect
                id={`${id}-settings-default-reasoning-effort`}
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
        {invalidModel && <p id={`${id}-error`} role="alert">Choose a compatible model for this provider or profile.</p>}
    </>
}
