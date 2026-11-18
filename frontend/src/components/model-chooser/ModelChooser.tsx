import { useState, useId, useRef } from 'react'
import { Popover } from 'radix-ui'
import { ChevronDown } from 'lucide-react'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { NativeSelect, NativeSelectOptGroup, NativeSelectOption } from '@/components/ui/native-select'
import { refreshModelOptions, useModelOptions } from './useModelOptions'
import { useLlmProfiles } from '@/lib/useLlmProfiles'
import { effortNames, providerLabel as labelOf, sessionControls, type SessionControlValue } from '@/lib/llmSuggestions'
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
    /** What an inherited choice is called on the button, such as "Project default". */
    defaultPrefix?: string
    /** Says where the model comes from and where to change it. */
    title?: string
    /** Offers an ACP harness's session controls; off where they would not be saved. */
    sessionControls?: boolean
}

const effortLabel = (effort: string) => effort === 'xhigh' ? 'XHigh' : effort.charAt(0).toUpperCase() + effort.slice(1)
const empty: ModelSettings = { provider: null, llm_profile: null, model: null, reasoning_effort: null }

export function ModelChooser({ value, inherited, onChange, projectPath, inheritLabel, layout = 'fields', disabled = false, invalidModel = false, defaultPrefix = 'Default', title, sessionControls: withControls = true }: ModelChooserProps) {
    const id = useId()
    const profiles = useLlmProfiles()
    const discovery = useModelOptions(projectPath)
    const [open, setOpen] = useState(false)
    const [search, setSearch] = useState('')
    const [budgetInput, setBudgetInput] = useState<string | null>(null)
    const [active, setActive] = useState(-1)
    const input = useRef<HTMLInputElement>(null)
    const statuses = discovery?.payload?.providers
    const providerLabel = (provider: string) => labelOf(provider, statuses)
    const unavailable = (provider: string) => !!discovery && (discovery.failed || discovery.payload?.providers[provider]?.status === 'unavailable')
    const checking = (provider: string) => discovery?.payload?.providers[provider]?.status === 'checking'
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
    // A saved model the provider no longer lists, or a custom one: allowed, but worth a note.
    const unlisted = layout === 'fields' && !invalidModel && !!value.provider && !value.llm_profile && !!value.model && !unavailable(value.provider)
        && !!discovery?.payload?.models.some(entry => entry.provider === value.provider && !entry.llm_profile) && !metadata(value.provider, value.model)
    const note = invalidModel ? `${id}-error` : unlisted ? `${id}-unlisted` : undefined
    const effortSelection = highlighted ? resolve({ ...highlighted, reasoning_effort: null }) : resolved
    const supported = effortSelection.efforts
    const allEfforts = [...new Set([...supported, ...(supported.length && value.reasoning_effort ? [value.reasoning_effort] : [])])]
    // Session controls are a harness's own, so another provider starts without them. Another model
    // keeps only the native controls it offers: the rest would stay saved, hidden, and fail its turns.
    const select = (selection: { provider: string | null; llm_profile: string | null; model: string | null }) => {
        const same = selection.provider === value.provider && selection.llm_profile === value.llm_profile && (selection.model || null) === value.model
        const meta = resolve({ ...empty, ...selection }).meta
        const keep = (current: string | null | undefined, offered?: string[]) => same || current == null || offered?.includes(current) ? current : null
        const thinking = keep(value.thinking, meta?.supported_thinking)
        return { ...value,
            provider: selection.provider, llm_profile: selection.llm_profile, model: selection.model || null,
            thinking, thinking_budget_tokens: thinking === value.thinking ? value.thinking_budget_tokens : null,
            reasoning_mode: keep(value.reasoning_mode, meta?.supported_reasoning_modes),
            reasoning_summary: keep(value.reasoning_summary, meta?.supported_reasoning_summaries),
            session_config: selection.provider === value.provider ? value.session_config : undefined }
    }
    const commit = (choice: typeof choices[number], close: boolean) => {
        setBudgetInput(null)
        onChange({ ...select(choice), reasoning_effort: value.reasoning_effort })
        if (close) setOpen(false)
    }
    const chosen = (choice: typeof choices[number]) => (value.model ?? '') === choice.model && value.provider === choice.provider && value.llm_profile === choice.llm_profile
    const effortStatus = statuses?.[(highlighted ? highlighted.provider : effective.provider) ?? '']
    const names = effortNames(effortStatus)
    const effortName = (effort: string) => names[effort] ?? effortLabel(effort)
    const harness = withControls && value.provider ? statuses?.[value.provider] : undefined
    const offered = sessionControls(harness)
    // A saved control the harness no longer offers stays visible: its turn fails until it is cleared.
    const controls = [...offered, ...Object.keys(harness ? value.session_config ?? {} : {})
        .filter(control => !offered.some(entry => entry.id === control)).map(control => ({ id: control, name: control, description: undefined, values: [] }))]
    const modes = harness && !harness.config_options?.length ? harness.modes?.availableModes ?? [] : []
    const setControl = (id: string, selected: string) => {
        const { [id]: _, ...rest } = value.session_config ?? {}
        onChange({ ...value, session_config: selected ? { ...rest, [id]: selected } : rest })
    }
    const valuesOf = (values: SessionControlValue[]) => [...new Set(values.map(entry => entry.group))].map(group => {
        const entries = values.filter(entry => entry.group === group).map(entry => <NativeSelectOption key={entry.value} value={entry.value}>{entry.name}</NativeSelectOption>)
        return group ? <NativeSelectOptGroup key={group} label={group}>{entries}</NativeSelectOptGroup> : entries
    })
    const option = (choice: typeof choices[number], index: number) => <button key={`${choice.groupIndex}:${choice.model}`} type="button"
        id={`${id}-option-${index}`} role="option" tabIndex={-1} aria-label={choice.label} aria-description={choice.isDefault ? 'Provider default' : undefined}
        aria-selected={chosen(choice)}
        className={`flex w-full items-center justify-between rounded px-2 py-2 text-left text-sm break-words ${active === index ? 'bg-accent text-accent-foreground' : 'hover:bg-accent'} aria-selected:font-semibold`}
        onClick={() => { setActive(-1); commit(choice, false); input.current?.focus({ preventScroll: true }) }}>
        <span>{choice.label}{choice.isDefault && <span className="ml-2 text-xs text-muted-foreground"> Default</span>}</span>
        {chosen(choice) && <span aria-hidden="true">✓</span>}
    </button>

    return <Field className={layout === 'compact' ? 'min-w-0 w-auto max-w-full' : 'w-full'}>
        {layout === 'fields' && <FieldLabel htmlFor={id}>Model</FieldLabel>}
        <Popover.Root open={open && !disabled} onOpenChange={next => { setOpen(next); setSearch(''); setActive(-1); if (next) refreshModelOptions(projectPath) }}>
            <Popover.Trigger asChild>
                <Button id={id} type="button" variant="outline" disabled={disabled} title={title} aria-label={`Model: ${value.model ? describe(resolved) : `${defaultPrefix}: ${describe(resolved)}`}`}
                    aria-invalid={invalidModel} aria-describedby={note}
                    className={`min-w-0 max-w-full justify-between border border-input text-foreground hover:bg-transparent focus-visible:border-ring ${layout === 'fields' ? 'w-full' : ''} ${!value.model ? 'text-muted-foreground' : ''}`}>
                    <span className="truncate">{value.model ? describe(resolved) : `${defaultPrefix}: ${describe(resolved)}`}</span><ChevronDown aria-hidden="true" className="size-4 shrink-0 text-muted-foreground opacity-50" />
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
                {!discovery && <p role="status" className="px-2 text-xs text-muted-foreground">Loading models…</p>}
                <div id={`${id}-list`} role="listbox" aria-label="Models" className="min-h-0 max-h-[min(45vh,20rem)] overflow-y-auto">
                    {groups.map((group, index) => {
                        const entries = rows.filter(row => row.groupIndex === index)
                        if (!entries.length && (group.profile || !unavailable(group.provider) && !checking(group.provider))) return null
                        return <div key={`${group.provider}:${group.profile}`} role="group" aria-label={`${providerLabel(group.provider)}${group.profile ? ` / ${group.label}` : ''}`}>
                            <div className="px-2 pt-3 pb-1 text-xs font-medium text-muted-foreground">{providerLabel(group.provider)}{group.profile ? ` / ${group.label}` : ''}
                                {!group.profile && checking(group.provider) && <span role="status" className="block">Checking…</span>}
                                {!group.profile && unavailable(group.provider) && <span role="status" className="block">{statuses?.[group.provider]?.setup_status === 'unauthenticated' ? 'Authentication required. ' : ''}Model discovery unavailable. {statuses?.[group.provider]?.error}{statuses?.[group.provider]?.setup_status ? ' Check and fix it under Settings › Models & accounts › Agent harnesses.' : ''}</span>}
                                {!group.profile && !unavailable(group.provider) && (['chat', 'flow'] as const).map(use => {
                                    const eligibility = statuses?.[group.provider]?.[use]
                                    return eligibility && eligibility.state !== 'supported' && <span key={use} className="block">{use === 'chat' ? 'Chat' : 'Flows'} unavailable: {eligibility.reason}</span>
                                })}
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
                            onChange({ ...select(highlighted ?? value), reasoning_effort: effort })
                            setOpen(false)
                        }}>{effort ? `${effortName(effort)}${supported.includes(effort) ? '' : ' (custom)'}` : 'Default'}</Button>)}
                </div>
                {([
                    ['thinking', 'Thinking', effortSelection.meta?.supported_thinking],
                    ['reasoning_mode', 'Mode', effortSelection.meta?.supported_reasoning_modes],
                    ['reasoning_summary', 'Summary', effortSelection.meta?.supported_reasoning_summaries],
                ] as const).map(([field, label, options]) => options?.length ? <div key={field} role="group" aria-label={label} className="mt-2 flex shrink-0 flex-wrap gap-1 border-t pt-2">
                    <span className="w-full text-xs text-muted-foreground">{label}</span>
                    {(field === 'reasoning_mode' ? options : [null, ...options]).map(option => <Button key={option ?? 'default'} type="button" size="sm" variant="ghost"
                        disabled={effortLocked} aria-pressed={(value[field] ?? (field === 'reasoning_mode' ? 'standard' : null)) === option}
                        onClick={() => {
                            setBudgetInput(null)
                            onChange({ ...select(highlighted ?? value),
                                [field]: option,
                                ...(field === 'thinking' ? { thinking_budget_tokens: option === 'budget' ? value.thinking_budget_tokens ?? 1024 : null } : {}) })
                        }}>{option ? effortLabel(option) : 'Default'}</Button>)}
                    {field === 'thinking' && value.thinking === 'budget' && <Input type="number" aria-label="Thinking budget tokens" min={1024} step={1}
                        disabled={effortLocked} value={budgetInput ?? value.thinking_budget_tokens ?? 1024} onBlur={() => setBudgetInput(null)}
                        onChange={event => { setBudgetInput(event.target.value); const tokens = event.target.valueAsNumber; if (Number.isInteger(tokens) && tokens >= 1024) onChange({ ...value, thinking_budget_tokens: tokens }) }} />}
                </div> : null)}
                {(controls.length > 0 || modes.length > 0) && <div role="group" aria-label="Session controls" className="mt-2 grid shrink-0 gap-2 border-t pt-2">
                    <span className="text-xs text-muted-foreground">Session controls · changes apply from the next turn</span>
                    {controls.map(control => <label key={control.id} className="grid gap-1 text-xs" title={control.description}>
                        {control.name}
                        <NativeSelect size="sm" aria-label={control.name} value={value.session_config?.[control.id] ?? ''} onChange={event => setControl(control.id, event.target.value)}>
                            <NativeSelectOption value="">Harness default</NativeSelectOption>
                            {valuesOf(control.values)}
                            {(saved => saved && !control.values.some(entry => entry.value === saved) && <NativeSelectOption value={saved}>{saved} (not offered now)</NativeSelectOption>)(value.session_config?.[control.id])}
                        </NativeSelect>
                    </label>)}
                    {modes.length > 0 && <p className="text-xs text-muted-foreground">Harness modes, set by the harness: {modes.map(mode => mode.name).join(', ')}</p>}
                </div>}
            </Popover.Content></Popover.Portal>
        </Popover.Root>
        {invalidModel && <p id={`${id}-error`} role="alert">Choose a compatible model for this provider or profile.</p>}
        {unlisted && <p id={`${id}-unlisted`} className="text-xs text-warning">
            {providerLabel(value.provider!)} doesn’t list {value.model} right now. Choose a listed model unless it’s a custom one you know works.
        </p>}
    </Field>
}
