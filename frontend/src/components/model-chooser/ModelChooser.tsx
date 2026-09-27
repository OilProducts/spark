import { useState, useId } from 'react'
import { getLlmSelectionOptions, getModelSuggestions, splitLlmSelection } from '@/lib/llmSuggestions'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { NativeSelect } from '@/components/ui/native-select'
import { useModelOptions } from './useModelOptions'
import { useLlmProfiles } from '@/lib/useLlmProfiles'
import type { ModelSettings } from '@/lib/api/settingsApi'

interface ModelChooserProps {
    value: ModelSettings
    onChange: (value: ModelSettings) => void
    projectPath: string | null
    inheritLabel: string
    layout?: 'fields' | 'compact'
    disabled?: boolean
    invalidModel?: boolean
}

const standardEfforts = ['low', 'medium', 'high', 'xhigh', 'max', 'ultra']
const effortLabel = (effort: string) => effort === 'xhigh' ? 'XHigh' : effort[0].toUpperCase() + effort.slice(1)

export function ModelChooser({ value, onChange, projectPath, inheritLabel, layout = 'fields', disabled = false, invalidModel = false }: ModelChooserProps) {
    const id = useId()
    const llmProfiles = useLlmProfiles()
    const uiDefaults = {
        llm_provider: value.provider ?? '', llm_profile: value.llm_profile ?? '',
        llm_model: value.model ?? '', reasoning_effort: value.reasoning_effort ?? '',
    }
    const setUiDefault = (key: 'llm_model' | 'reasoning_effort', next: string) => onChange({
        ...value, [key === 'llm_model' ? 'model' : key]: next || null,
    })
    const [customModel, setCustomModel] = useState(false)
    const provider = uiDefaults.llm_profile || uiDefaults.llm_provider
    const providerOptions = [...new Set([...getLlmSelectionOptions(llmProfiles), provider])].filter(Boolean)
    const profile = llmProfiles.find((entry) => entry.id === provider)
    const currentDiscovery = useModelOptions(projectPath)
    const discoveryProvider = profile?.provider || provider
    const discoveredModels = currentDiscovery?.payload?.models.filter((model) => model.provider === discoveryProvider)
    const discoverable = discoveryProvider === 'codex' || discoveryProvider === 'claude-code'
    const discoveryUnavailable = currentDiscovery?.failed || (discoveryProvider === 'codex'
        ? currentDiscovery?.payload?.providers.codex.status === 'unavailable' || !!currentDiscovery && !discoveredModels?.length
        : discoverable && !!currentDiscovery && !discoveredModels?.length)
    const modelOptions = [...new Set(profile ? profile.models : (
        discoveredModels?.length && !discoveryUnavailable
            ? discoveredModels.map((model) => model.id)
            : getModelSuggestions(provider === 'codex' ? 'openai' : provider === 'claude-code' ? 'anthropic' : provider, llmProfiles)
    ))].filter(Boolean)
    const unlistedModel = !!uiDefaults.llm_model && !modelOptions.includes(uiDefaults.llm_model)
    const discoveryMessage = !profile && projectPath
        ? (!currentDiscovery ? 'Loading models…'
            : currentDiscovery.failed || discoveryUnavailable ? 'Model discovery unavailable. Using suggestions.' : null)
        : null

    const metadata = !discoveryUnavailable
        ? discoveredModels?.find((model) => model.id === (value.model || profile?.default_model)
            || !value.model && !profile && model.is_default) : undefined
    const efforts = metadata?.supported_reasoning_efforts?.length ? metadata.supported_reasoning_efforts : standardEfforts
    const compact = layout === 'compact'
    const labelClass = compact ? 'sr-only' : undefined

    return <fieldset disabled={disabled} className={compact ? 'flex min-w-0 flex-wrap items-center gap-2' : 'space-y-3'}>
        <Field className={compact ? 'w-auto' : '[&>[data-slot=native-select-wrapper]]:w-full'}>
            <FieldLabel className={labelClass} htmlFor={`${id}-llm-provider`}>
                Provider or profile
            </FieldLabel>
            <NativeSelect
                id={`${id}-llm-provider`}
                value={provider}
                onChange={(event) => {
                    const selection = splitLlmSelection(event.target.value, llmProfiles)
                    const profile = llmProfiles.find((entry) => entry.id === selection.llm_profile)
                    onChange({ provider: selection.llm_provider || null,
                        llm_profile: selection.llm_profile || null, model: profile && !profile.default_model ? profile.models[0] ?? null : null, reasoning_effort: null })
                    setCustomModel(false)
                }}
                className={compact ? 'max-w-[13rem] text-sm' : 'text-sm'}
            >
                <option value="">{inheritLabel}</option>
                {providerOptions.map((option) => {
                    const label = llmProfiles.find((entry) => entry.id === option)?.label
                    return <option key={option} value={option}>{label ? `${label} (${option})` : option}</option>
                })}
            </NativeSelect>
        </Field>
        <Field className={compact ? 'w-auto' : '[&>[data-slot=native-select-wrapper]]:w-full'}>
            <FieldLabel className={labelClass} htmlFor={`${id}-llm-model`}>
                Model
            </FieldLabel>
            <NativeSelect
                id={`${id}-llm-model`}
                aria-invalid={!!invalidModel} aria-describedby={invalidModel ? `${id}-error` : undefined}
                value={customModel ? 'custom' : uiDefaults.llm_model ? `model:${uiDefaults.llm_model}` : ''}
                onChange={(event) => {
                    const value = event.target.value
                    setCustomModel(value === 'custom')
                    if (value !== 'custom') setUiDefault('llm_model', value.replace(/^model:/, ''))
                }}
                className={compact ? 'max-w-[13rem] text-sm' : 'text-sm'}
            >
                <option value="">{inheritLabel}</option>
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
                    <FieldLabel className={labelClass} htmlFor={`${id}-custom-llm-model`}>Custom model</FieldLabel>
                    <Input
                        id={`${id}-custom-llm-model`}
                        aria-invalid={!!invalidModel} aria-describedby={invalidModel ? `${id}-error` : undefined}
                        value={uiDefaults.llm_model}
                        onChange={(event) => {
                            setCustomModel(true)
                            setUiDefault('llm_model', event.target.value)
                        }}
                        className={compact ? 'max-w-[13rem] text-sm' : 'text-sm'}
                    />
                </>
            )}
            {discoveryMessage && <p role="status" className="text-xs text-muted-foreground">{discoveryMessage}</p>}
        </Field>
        <Field className={compact ? 'w-auto' : undefined}>
            <FieldLabel className={labelClass} htmlFor={`${id}-reasoning-effort`}>
                Reasoning effort
            </FieldLabel>
            <NativeSelect
                id={`${id}-reasoning-effort`}
                value={uiDefaults.reasoning_effort}
                onChange={(event) => setUiDefault('reasoning_effort', event.target.value)}
                className={compact ? 'max-w-[13rem] text-sm' : 'text-sm'}
            >
                <option value="">{inheritLabel}</option>
                {efforts.map((effort) => <option key={effort} value={effort}>{effortLabel(effort)}</option>)}
                {value.reasoning_effort && !efforts.includes(value.reasoning_effort) &&
                    <option value={value.reasoning_effort}>{effortLabel(value.reasoning_effort)} (custom)</option>}
            </NativeSelect>
        </Field>
        {invalidModel && <p id={`${id}-error`} role="alert">Choose a compatible model for this provider or profile.</p>}
    </fieldset>
}
