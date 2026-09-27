import { useState, useId, useRef } from 'react'
import { Popover } from 'radix-ui'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { useModelOptions } from './useModelOptions'
import { useLlmProfiles } from '@/lib/useLlmProfiles'
import type { ModelSettings } from '@/lib/api/settingsApi'

interface ModelChooserProps {
    value: ModelSettings
    inherited?: ModelSettings
    onChange: (value: ModelSettings) => void
    projectPath: string | null
    inheritLabel: string
    layout?: 'fields' | 'compact'
    disabled?: boolean
    invalidModel?: boolean
}

const effortLabel = (effort: string) => effort === 'xhigh' ? 'XHigh' : effort.charAt(0).toUpperCase() + effort.slice(1)
const providerLabel = (provider: string) => provider === 'codex' ? 'Codex' : provider === 'claude-code' ? 'Claude Code' : provider
const empty: ModelSettings = { provider: null, llm_profile: null, model: null, reasoning_effort: null }

export function ModelChooser({ value, inherited, onChange, projectPath, inheritLabel, layout = 'fields', disabled = false, invalidModel = false }: ModelChooserProps) {
    const id = useId()
    const profiles = useLlmProfiles()
    const discovery = useModelOptions(projectPath)
    const [open, setOpen] = useState(false)
    const [search, setSearch] = useState('')
    const [active, setActive] = useState(-1)
    const input = useRef<HTMLInputElement>(null)
    const unavailable = (provider: string) => !!discovery && (discovery.failed || discovery.payload?.providers[provider]?.status === 'unavailable')
    const metadata = (provider: string | null, model: string | null) => !unavailable(provider || '')
        ? discovery?.payload?.models.find(entry => entry.provider === provider && !entry.llm_profile && (model ? entry.id === model : entry.is_default)) : undefined
    const resolve = (settings: ModelSettings) => {
        const profile = profiles.find(entry => entry.id === settings.llm_profile)
        const provider = profile?.provider || settings.provider || 'codex'
        const model = settings.model || profile?.default_model || null
        const meta = profile ? discovery?.payload?.models.find(entry => entry.llm_profile === profile.id && entry.id === model) : metadata(provider, model)
        const fallback = !profile && !!model && !meta ? discovery?.payload?.provider_reasoning_efforts?.[provider] : undefined
        const efforts = profile ? profile.reasoning_efforts ?? [] : meta?.supported_reasoning_efforts ?? fallback ?? []
        return { model: meta?.display || model || meta?.id, effort: settings.reasoning_effort || meta?.default_reasoning_effort, meta, efforts, unverified: meta?.reasoning_unverified || !!fallback?.length }
    }
    const defaults = resolve(inherited ?? { ...empty, provider: value.provider, llm_profile: value.llm_profile })
    const effective = {
        ...value,
        provider: value.provider || (!value.llm_profile ? inherited?.provider : null) || null,
        llm_profile: value.llm_profile || (!value.provider ? inherited?.llm_profile : null) || null,
        model: value.model || (!value.provider && !value.llm_profile ? inherited?.model : null) || null,
        reasoning_effort: value.reasoning_effort || (!value.provider && !value.llm_profile ? inherited?.reasoning_effort : null) || null,
    }
    const resolved = resolve(effective)
    const describe = (entry: ReturnType<typeof resolve>) => entry.model
        ? `${entry.model} · ${entry.effort ? effortLabel(entry.effort) : 'Default effort'}` : inheritLabel
    const providers = [...new Set([...Object.keys(discovery?.payload?.providers ?? {}), ...(discovery?.payload?.models.map(model => model.provider) ?? []), ...profiles.map(profile => profile.provider)])]
    const groups = providers.flatMap(provider => {
        const discovered = unavailable(provider) ? [] : discovery?.payload?.models.filter(model => model.provider === provider && !model.llm_profile) ?? []
        return [...(discovery?.payload?.providers[provider] || discovered.length ? [{ provider, profile: null as string | null, label: providerLabel(provider), models: discovered.map(model => model.id) }] : []),
            ...profiles.filter(profile => profile.provider === provider).map(profile => ({ provider, profile: profile.id, label: profile.label || profile.id, models: profile.models }))]
    })
    const query = search.trim().toLowerCase()
    const rows = groups.flatMap((group, groupIndex) => [...new Set(group.models)].map(model => ({
        groupIndex, provider: group.profile ? null : group.provider, llm_profile: group.profile, model,
        label: metadata(group.provider, model)?.display || model,
        isDefault: group.profile ? profiles.find(profile => profile.id === group.profile)?.default_model === model : metadata(group.provider, model)?.is_default,
    }))).filter(row => [row.model, row.label, groups[row.groupIndex].provider, providerLabel(groups[row.groupIndex].provider), groups[row.groupIndex].label, row.llm_profile].some(text => text?.toLowerCase().includes(query)))
    const context = groups.find(group => value.llm_profile ? group.profile === value.llm_profile : group.provider === value.provider && !group.profile) || groups[0]
    const custom = !!context && (!!context.profile || !unavailable(context.provider)) && !!query && !rows.some(row => row.model.toLowerCase() === query || row.label.toLowerCase() === query)
    const choices = [...rows, ...(custom ? [{ groupIndex: -1, provider: context.profile ? null : context.provider, llm_profile: context.profile, model: search.trim(), label: `Use "${search.trim()}" as a custom model`, isDefault: false }] : [])]
    const highlighted = choices[active]
    // The API inherits a whole model group, so an inherited choice has no effort of its own:
    // setting one would silently pin today's inherited model. Choose a model first.
    const effortLocked = !value.provider && !value.llm_profile && !highlighted
    const effortSelection = highlighted ? resolve({ ...highlighted, reasoning_effort: null }) : resolved
    const supported = effortSelection.efforts
    const allEfforts = [...new Set([...supported, ...(supported.length && value.reasoning_effort ? [value.reasoning_effort] : [])])]
    const commit = (choice: typeof choices[number], close: boolean) => {
        onChange({ provider: choice.provider, llm_profile: choice.llm_profile, model: choice.model, reasoning_effort: value.reasoning_effort })
        if (close) setOpen(false)
    }
    const option = (choice: typeof choices[number], index: number) => <button key={`${choice.groupIndex}:${choice.model}`} type="button"
        id={`${id}-option-${index}`} role="option" tabIndex={-1} aria-label={choice.label} aria-description={choice.isDefault ? 'Provider default' : undefined}
        aria-selected={value.model === choice.model && value.provider === choice.provider && value.llm_profile === choice.llm_profile}
        className={`flex w-full items-center justify-between rounded px-2 py-2 text-left text-sm break-words ${active === index ? 'bg-accent text-accent-foreground' : 'hover:bg-accent'} aria-selected:font-semibold`}
        onClick={() => { setActive(-1); commit(choice, false); input.current?.focus({ preventScroll: true }) }}>
        <span>{choice.label}{choice.isDefault && <span className="ml-2 text-xs text-muted-foreground"> Default</span>}</span>
        {value.model === choice.model && value.provider === choice.provider && value.llm_profile === choice.llm_profile && <span aria-hidden="true">✓</span>}
    </button>

    return <Field className={layout === 'compact' ? 'min-w-0 w-auto max-w-full' : 'w-full'}>
        {layout === 'fields' && <FieldLabel htmlFor={id}>Model</FieldLabel>}
        <Popover.Root open={open && !disabled} onOpenChange={next => { setOpen(next); setSearch(''); setActive(-1) }}>
            <Popover.Trigger asChild>
                <Button id={id} type="button" variant="outline" disabled={disabled} aria-label={`Model: ${value.model ? describe(resolved) : `Default: ${describe(resolved)}`}`}
                    aria-invalid={invalidModel} aria-describedby={invalidModel ? `${id}-error` : undefined}
                    className={`min-w-0 max-w-full justify-between ${layout === 'fields' ? 'w-full' : ''} ${!value.model ? 'text-muted-foreground' : ''}`}>
                    <span className="truncate">{value.model ? describe(resolved) : `Default: ${describe(resolved)}`}</span><span aria-hidden="true">⌄</span>
                </Button>
            </Popover.Trigger>
            <Popover.Portal><Popover.Content align="start" sideOffset={6} collisionPadding={8} aria-label="Choose model"
                className="z-50 flex max-h-[var(--radix-popover-content-available-height)] w-80 max-w-[calc(100vw-1rem)] flex-col rounded-md border border-border bg-popover p-2 text-popover-foreground shadow-md"
                onOpenAutoFocus={event => { event.preventDefault(); input.current?.focus({ preventScroll: true }) }}>
                <Input className="shrink-0" ref={input} aria-label="Search models" role="combobox" aria-expanded="true" aria-controls={`${id}-list`} aria-autocomplete="list"
                    aria-activedescendant={highlighted ? `${id}-option-${active}` : undefined} placeholder="Search models…" value={search}
                    onChange={event => { setSearch(event.target.value); setActive(-1) }}
                    onKeyDown={event => {
                        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                            event.preventDefault()
                            const next = choices.length ? (active + (event.key === 'ArrowDown' ? 1 : active < 0 ? 0 : -1) + choices.length) % choices.length : -1
                            setActive(next)
                            document.getElementById(`${id}-option-${next}`)?.scrollIntoView?.({ block: 'nearest' })
                        } else if (event.key === 'Enter') {
                            event.preventDefault()
                            if (highlighted) commit(highlighted, true)
                        }
                    }} />
                <Button type="button" variant="ghost" className="my-1 h-auto w-full justify-start whitespace-normal text-left" aria-pressed={!value.provider && !value.llm_profile && !value.model} onClick={() => { onChange(empty); setOpen(false) }}>
                    Use default · {describe(defaults)}
                </Button>
                {projectPath && !discovery && <p role="status" className="px-2 text-xs text-muted-foreground">Loading models…</p>}
                <div id={`${id}-list`} role="listbox" aria-label="Models" className="min-h-0 max-h-[min(45vh,20rem)] overflow-y-auto">
                    {groups.map((group, index) => {
                        const entries = rows.filter(row => row.groupIndex === index)
                        if (!entries.length && (group.profile || !unavailable(group.provider))) return null
                        return <div key={`${group.provider}:${group.profile}`} role="group" aria-label={`${providerLabel(group.provider)}${group.profile ? ` / ${group.label}` : ''}`}>
                            <div className="px-2 pt-3 pb-1 text-xs font-medium text-muted-foreground">{providerLabel(group.provider)}{group.profile ? ` / ${group.label}` : ''}
                                {!group.profile && unavailable(group.provider) && <span role="status" className="block">Model discovery unavailable. {discovery?.payload?.providers[group.provider]?.error}</span>}
                            </div>
                            {entries.map(choice => option(choice, choices.indexOf(choice)))}
                        </div>
                    })}
                    {custom && option(choices[choices.length - 1], choices.length - 1)}
                </div>
                <div role="group" aria-label="Reasoning effort" className="mt-2 flex shrink-0 flex-wrap gap-1 border-t border-border pt-2">
                    <span className="w-full text-xs text-muted-foreground">{effortLocked ? 'Effort follows the default. Choose a model to set it.' : 'Reasoning effort'}</span>
                    {effortSelection.unverified && <span className="w-full text-xs text-muted-foreground">Provider levels; unverified for this model.</span>}
                    {[null, ...allEfforts].map(effort => <Button key={effort ?? 'default'} type="button" size="sm" variant="ghost" disabled={effortLocked} className="aria-pressed:bg-accent aria-pressed:text-accent-foreground" aria-pressed={value.reasoning_effort === effort}
                        onClick={() => {
                            const selection = highlighted ?? value
                            onChange({ provider: selection.provider, llm_profile: selection.llm_profile, model: selection.model, reasoning_effort: effort })
                            setOpen(false)
                        }}>{effort ? `${effortLabel(effort)}${supported.includes(effort) ? '' : ' (custom)'}` : 'Default'}</Button>)}
                </div>
            </Popover.Content></Popover.Portal>
        </Popover.Root>
        {invalidModel && <p id={`${id}-error`} role="alert">Choose a compatible model for this provider or profile.</p>}
    </Field>
}
