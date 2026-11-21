import { useEffect, useId, useRef, useState } from 'react'
import { Popover } from 'radix-ui'
import { ChevronDown, ChevronRight, Star } from 'lucide-react'
import { Field, FieldLabel } from '@/components/ui/field'
import { Input } from '@/components/ui/input'
import { Button } from '@/components/ui/button'
import { NativeSelect, NativeSelectOptGroup, NativeSelectOption } from '@/components/ui/native-select'
import { refreshModelOptions, useModelOptions } from './useModelOptions'
import {
    EMPTY_MODEL_SETTINGS,
    buildModelCatalog,
    effectiveModelSettings,
    effortLabel,
    findModelMetadata,
    isChoiceAvailable,
    isProviderChecking,
    isProviderUnavailable,
    isSameModelRef,
    pinInheritedModelSettings,
    recordRecentModel,
    resolveModelSettings,
    selectModelSettings,
    togglePinnedModel,
    useModelPreferences,
    type ModelChoiceOption,
    type ResolvedModelInfo,
} from './modelSelection'
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
    /** What an inherited choice is called in tooltips and accessible labels, such as "Project default". */
    defaultPrefix?: string
    /** Says where the model comes from and where to change it. */
    title?: string
    /** Offers an ACP harness's session controls; off where they would not be saved. */
    sessionControls?: boolean
    /** Hides ACP harnesses (`acp:*`), e.g. for utility calls that only support native harnesses and API providers. */
    excludeAcp?: boolean
    /** Optional controlled open state for the Model popover (used by composer shortcuts and `/model`). */
    open?: boolean
    onOpenChange?: (open: boolean) => void
}

type RenderChoice = ModelChoiceOption & {
    section: 'pinned' | 'recent' | 'group' | 'custom'
    choiceIndex: number
}

export function ModelChooser({
    value,
    inherited,
    onChange,
    projectPath,
    inheritLabel,
    layout = 'fields',
    disabled = false,
    invalidModel = false,
    defaultPrefix = 'Default',
    title,
    sessionControls: withControls = true,
    excludeAcp = false,
    open: controlledOpen,
    onOpenChange,
}: ModelChooserProps) {
    const id = useId()
    const profiles = useLlmProfiles()
    const discovery = useModelOptions(projectPath)
    const { pinned, recent } = useModelPreferences()

    const [internalOpen, setInternalOpen] = useState(false)
    const [effortOpen, setEffortOpen] = useState(false)
    const [search, setSearch] = useState('')
    const [budgetInput, setBudgetInput] = useState<string | null>(null)
    const [active, setActive] = useState(-1)
    const [unavailableOpen, setUnavailableOpen] = useState(false)
    const [moreOptionsOverride, setMoreOptionsOverride] = useState<boolean | null>(null)
    const input = useRef<HTMLInputElement>(null)

    const isOpen = (controlledOpen ?? internalOpen) && !disabled
    const wasOpen = useRef(false)

    useEffect(() => {
        if (isOpen && !wasOpen.current) {
            refreshModelOptions(projectPath)
        } else if (!isOpen && wasOpen.current) {
            setSearch('')
            setActive(-1)
            setUnavailableOpen(false)
            setMoreOptionsOverride(null)
            setBudgetInput(null)
        }
        wasOpen.current = isOpen
    }, [isOpen, projectPath])

    const setOpen = (next: boolean) => {
        const target = next && !disabled
        if (controlledOpen === undefined) setInternalOpen(target)
        onOpenChange?.(target)
    }

    const statuses = discovery?.payload?.providers
    const providerLabel = (provider: string) => labelOf(provider, statuses)
    const unavailable = (provider: string) => isProviderUnavailable(discovery, provider)
    const checking = (provider: string) => isProviderChecking(discovery, provider)

    const isInherited = !value.provider && !value.llm_profile
    const isUsingDefault = isInherited || !value.model
    const effective = effectiveModelSettings(value, inherited)
    const resolved = resolveModelSettings(effective, profiles, discovery)
    const defaults = resolveModelSettings(
        inherited ?? { ...EMPTY_MODEL_SETTINGS, provider: value.provider, llm_profile: value.llm_profile },
        profiles,
        discovery,
    )

    const { groups, rows: allRows, unavailableProviders } = buildModelCatalog(profiles, discovery, excludeAcp)

    const query = search.trim().toLowerCase()
    const filteredRows = query
        ? allRows.filter((row) => [
            row.model,
            row.label,
            row.providerKey,
            providerLabel(row.providerKey),
            groups[row.groupIndex]?.label,
            row.llm_profile,
            row.groupLabel,
        ].some((text) => text?.toLowerCase().includes(query)))
        : allRows

    const context = groups.find((group) => (
        effective.llm_profile
            ? group.profile === effective.llm_profile
            : group.provider === effective.provider && !group.profile
    )) || groups.find((group) => Boolean(group.profile) || !unavailable(group.provider)) || groups[0]
    const custom = Boolean(context)
        && (Boolean(context?.profile) || !unavailable(context.provider))
        && Boolean(query)
        && !filteredRows.some((row) => row.model.toLowerCase() === query || row.label.toLowerCase() === query)

    // Keep known-unavailable entries (including unconfigured profiles) out of Pinned and Recent sections.
    const availableQuickRows = allRows.filter((row) => isChoiceAvailable(row, profiles, discovery))
    const pinnedRows = !query
        ? pinned.map((ref) => availableQuickRows.find((row) => isSameModelRef(row, ref))).filter((row): row is ModelChoiceOption => Boolean(row))
        : []
    const recentRows = !query
        ? recent
            .filter((ref) => !pinnedRows.some((row) => isSameModelRef(row, ref)))
            .map((ref) => availableQuickRows.find((row) => isSameModelRef(row, ref)))
            .filter((row): row is ModelChoiceOption => Boolean(row))
        : []

    let nextChoiceIndex = 0
    const pinnedChoices: RenderChoice[] = pinnedRows.map((row) => ({ ...row, section: 'pinned', choiceIndex: nextChoiceIndex++ }))
    const recentChoices: RenderChoice[] = recentRows.map((row) => ({ ...row, section: 'recent', choiceIndex: nextChoiceIndex++ }))
    const groupChoices: RenderChoice[] = filteredRows.map((row) => ({ ...row, section: 'group', choiceIndex: nextChoiceIndex++ }))
    const customChoice: RenderChoice | null = custom && context
        ? {
            groupIndex: -1,
            groupLabel: context.profile ? `${providerLabel(context.provider)} / ${context.label}` : providerLabel(context.provider),
            providerKey: context.provider,
            provider: context.profile ? null : context.provider,
            llm_profile: context.profile,
            model: search.trim(),
            label: `Use "${search.trim()}" as a custom model`,
            isDefault: false,
            section: 'custom',
            choiceIndex: nextChoiceIndex++,
        }
        : null

    const choices: RenderChoice[] = [...pinnedChoices, ...recentChoices, ...groupChoices, ...(customChoice ? [customChoice] : [])]

    // Pre-highlight discovered/profile matches only; never auto-highlight a synthetic custom model on mistyped search.
    const hasCatalogMatch = groupChoices.length > 0
    const effectiveActive = choices.length === 0
        ? -1
        : active >= 0 && active < choices.length
            ? active
            : query && hasCatalogMatch
                ? 0
                : -1
    const highlighted = effectiveActive >= 0 ? choices[effectiveActive] : undefined

    // Inherited effort & secondary controls unlock whenever the effective model is resolved;
    // they stay disabled only when no effective model is known yet.
    const hasKnownEffectiveModel = !isInherited || Boolean(resolved.pinTarget)
    const controlsLocked = !hasKnownEffectiveModel

    // A saved model the provider no longer lists, or a custom one: allowed, but worth a note in fields layout.
    const unlisted = layout === 'fields'
        && !invalidModel
        && Boolean(value.provider)
        && !value.llm_profile
        && Boolean(value.model)
        && !unavailable(value.provider!)
        && Boolean(discovery?.payload?.models.some((entry) => entry.provider === value.provider && !entry.llm_profile))
        && !findModelMetadata(discovery, value.provider, value.model)
    const note = invalidModel ? `${id}-error` : unlisted ? `${id}-unlisted` : undefined

    const supported = resolved.efforts
    const savedEffort = value.reasoning_effort ?? (isInherited ? inherited?.reasoning_effort ?? null : null)
    const allEfforts = [...new Set([...supported, ...(supported.length && savedEffort ? [savedEffort] : [])])]
    const effortStatus = statuses?.[resolved.provider ?? '']
    const names = effortNames(effortStatus)
    const effortName = (effort: string) => names[effort] ?? effortLabel(effort)

    // Render Effort trigger whenever the effective model supports efforts, carries a saved/default effort,
    // or is still unknown (so the disabled state + hint is visible).
    const showEffortTrigger = supported.length > 0 || Boolean(savedEffort) || Boolean(resolved.effort) || controlsLocked

    const describeDefault = (entry: ResolvedModelInfo) => {
        if (!entry.model) return inheritLabel
        if (!entry.effort) return entry.model
        const defaultNames = effortNames(statuses?.[entry.provider ?? ''])
        const formattedEffort = defaultNames[entry.effort] ?? effortLabel(entry.effort)
        return `${entry.model} · ${formattedEffort}`
    }

    const select = (selection: { provider: string | null; llm_profile: string | null; model: string | null }) =>
        selectModelSettings(value, selection, profiles, discovery)

    const resolveEditableBase = (preserveEffort = false) => {
        if (!isInherited) return value
        if (!resolved.pinTarget) return null
        const pinnedBase = pinInheritedModelSettings(value, inherited, resolved.pinTarget)
        return preserveEffort ? { ...pinnedBase, reasoning_effort: effective.reasoning_effort } : pinnedBase
    }

    const commitModel = (choice: RenderChoice) => {
        setBudgetInput(null)
        if (choice.section !== 'custom' && isChoiceAvailable(choice, profiles, discovery)) {
            recordRecentModel({ provider: choice.provider, llm_profile: choice.llm_profile, model: choice.model })
        }
        onChange({ ...select(choice), reasoning_effort: value.reasoning_effort })
        setOpen(false)
    }

    const commitEffort = (effort: string | null) => {
        const base = resolveEditableBase(false)
        if (!base) return
        onChange({ ...base, reasoning_effort: effort })
        setEffortOpen(false)
    }

    const chosen = (choice: ModelChoiceOption) =>
        (value.model ?? '') === choice.model && value.provider === choice.provider && value.llm_profile === choice.llm_profile

    const isPinned = (choice: ModelChoiceOption) => pinned.some((entry) => isSameModelRef(entry, choice))

    // ACP session controls & native secondary controls tucked behind "More options".
    // Keep saved secondary settings discoverable even when capability metadata or the harness is missing from discovery.
    const activeProvider = value.provider || (isInherited ? effective.provider : null)
    const harness = withControls && activeProvider ? statuses?.[activeProvider] : undefined
    const offered = sessionControls(harness)
    const effectiveSessionConfig = withControls ? (effective.session_config ?? {}) : {}
    const savedSessionEntries = Object.entries(effectiveSessionConfig)
    // A saved control the harness no longer offers (or from a harness missing from discovery) stays visible until cleared.
    const controls = [
        ...offered,
        ...savedSessionEntries
            .filter(([control]) => !offered.some((entry) => entry.id === control))
            .map(([control]) => ({ id: control, name: control, description: undefined, values: [] })),
    ]
    const modes = harness && !harness.config_options?.length ? harness.modes?.availableModes ?? [] : []
    const setControl = (controlId: string, selected: string) => {
        const base = resolveEditableBase(true)
        if (!base) return
        const { [controlId]: _, ...rest } = base.session_config ?? {}
        onChange({ ...base, session_config: selected ? { ...rest, [controlId]: selected } : rest })
    }
    const valuesOf = (values: SessionControlValue[]) => [...new Set(values.map((entry) => entry.group))].map((group) => {
        const entries = values
            .filter((entry) => entry.group === group)
            .map((entry) => <NativeSelectOption key={entry.value} value={entry.value}>{entry.name}</NativeSelectOption>)
        return group ? <NativeSelectOptGroup key={group} label={group}>{entries}</NativeSelectOptGroup> : entries
    })

    const nativeSecondaryGroups = ([
        ['thinking', 'Thinking', resolved.meta?.supported_thinking ?? []],
        ['reasoning_mode', 'Mode', resolved.meta?.supported_reasoning_modes ?? []],
        ['reasoning_summary', 'Summary', resolved.meta?.supported_reasoning_summaries ?? []],
    ] as const).map(([field, label, offeredOptions]) => {
        const current = effective[field] ?? null
        const hasNonDefaultSaved = field === 'reasoning_mode'
            ? Boolean(current && current !== 'standard')
            : Boolean(current)
        const unofferedSaved = hasNonDefaultSaved && !offeredOptions.includes(current!)
        const optionList: (string | null)[] = field === 'reasoning_mode'
            ? [...new Set([...(offeredOptions.length ? offeredOptions : ['standard']), ...(unofferedSaved && current ? [current] : [])])]
            : [null, ...offeredOptions, ...(unofferedSaved && current ? [current] : [])]
        return { field, label, offeredOptions, current, hasNonDefaultSaved, unofferedSaved, optionList }
    }).filter((entry) => entry.offeredOptions.length > 0 || entry.hasNonDefaultSaved)

    const hasSecondaryControls = nativeSecondaryGroups.length > 0 || controls.length > 0 || modes.length > 0

    const staleSessionControlCount = Boolean(discovery) && !checking(activeProvider ?? '')
        ? savedSessionEntries.filter(([controlId, savedVal]) => {
            const offeredControl = offered.find((entry) => entry.id === controlId)
            return !offeredControl || !offeredControl.values.some((entry) => entry.value === savedVal)
        }).length
        : 0
    const unofferedNativeCount = Boolean(discovery) && !checking(activeProvider ?? '')
        ? nativeSecondaryGroups.filter((entry) => entry.unofferedSaved).length
        : 0
    const unavailableSecondaryCount = staleSessionControlCount + unofferedNativeCount

    const activeSecondaryLabels: string[] = []
    if (effective.thinking) {
        activeSecondaryLabels.push(
            effective.thinking === 'budget'
                ? `Budget (${effective.thinking_budget_tokens ?? 1024})`
                : effortLabel(effective.thinking),
        )
    }
    if (effective.reasoning_mode && effective.reasoning_mode !== 'standard') {
        activeSecondaryLabels.push(effortLabel(effective.reasoning_mode))
    }
    if (effective.reasoning_summary) {
        activeSecondaryLabels.push(`Summary: ${effortLabel(effective.reasoning_summary)}`)
    }
    for (const [controlId, savedVal] of savedSessionEntries) {
        const control = controls.find((entry) => entry.id === controlId)
        const valName = control?.values.find((entry) => entry.value === savedVal)?.name ?? savedVal
        activeSecondaryLabels.push(`${control?.name ?? controlId}: ${valName}`)
    }

    const moreOptionsOpen = moreOptionsOverride ?? (unavailableSecondaryCount > 0)

    // Labels, tooltips, and accessible names.
    const modelText = resolved.model ?? inheritLabel
    const modelSourceSummary = resolved.model
        ? `${defaultPrefix}: ${resolved.model}${resolved.providerDisplay ? ` (${resolved.providerDisplay})` : ''}`
        : `${defaultPrefix}: ${inheritLabel}`
    const modelTooltip = isUsingDefault
        ? (title ? `${modelSourceSummary}. ${title}` : modelSourceSummary)
        : title
    const modelAriaLabel = [
        `Model: ${modelText}`,
        resolved.model && resolved.providerDisplay ? resolved.providerDisplay : null,
        isUsingDefault ? defaultPrefix : null,
    ].filter(Boolean).join(', ')

    const activeEffortValue = savedEffort ?? resolved.effort ?? null
    const effortTriggerText = controlsLocked
        ? 'Effort'
        : activeEffortValue
            ? effortName(activeEffortValue)
            : 'Default'
    const effortIsDefault = controlsLocked || isInherited || !value.reasoning_effort
    const effortHint = 'Choose a model to set reasoning effort.'
    const effortAriaLabel = controlsLocked
        ? 'Reasoning effort: unavailable'
        : `Reasoning effort: ${effortTriggerText}${effortIsDefault && activeEffortValue ? ' (default)' : ''}`
    const effortTooltip = controlsLocked
        ? effortHint
        : isInherited
            ? `Reasoning effort (${defaultPrefix}; selecting pins ${modelText})`
            : 'Reasoning effort'

    const triggerSize = layout === 'compact' ? 'sm' : 'default'

    const renderOption = (choice: RenderChoice, showGroupTag: boolean) => {
        const pinnedState = choice.section !== 'custom' && isPinned(choice)
        const optionAriaLabel = showGroupTag ? `${choice.label}, ${choice.groupLabel}` : choice.label
        const pinAriaLabel = pinnedState
            ? `Unpin ${choice.label} (${choice.groupLabel})`
            : `Pin ${choice.label} (${choice.groupLabel})`
        return (
            <div
                key={`${choice.section}:${choice.groupIndex}:${choice.provider ?? ''}:${choice.llm_profile ?? ''}:${choice.model}`}
                className="group/row flex items-center gap-0.5"
            >
                <button
                    type="button"
                    id={`${id}-option-${choice.choiceIndex}`}
                    role="option"
                    tabIndex={-1}
                    aria-label={optionAriaLabel}
                    aria-description={choice.isDefault ? 'Provider default' : undefined}
                    aria-selected={chosen(choice)}
                    className={`flex min-w-0 flex-1 items-center justify-between gap-2 rounded px-2 py-1.5 text-left text-sm break-words ${effectiveActive === choice.choiceIndex ? 'bg-accent text-accent-foreground' : 'hover:bg-accent'} aria-selected:font-semibold`}
                    onClick={() => commitModel(choice)}
                >
                    <span className="flex min-w-0 items-center gap-1.5">
                        <span className="truncate">{choice.label}</span>
                        {showGroupTag && (
                            <span className="shrink-0 rounded bg-muted px-1 py-0.5 text-xs font-normal leading-none text-muted-foreground">
                                {choice.groupLabel}
                            </span>
                        )}
                        {!showGroupTag && choice.isDefault && (
                            <span className="shrink-0 text-xs font-normal text-muted-foreground">Default</span>
                        )}
                    </span>
                    {chosen(choice) && <span aria-hidden="true" className="shrink-0">✓</span>}
                </button>
                {choice.section !== 'custom' && (
                    <button
                        type="button"
                        aria-label={pinAriaLabel}
                        aria-pressed={pinnedState}
                        title={pinnedState ? 'Unpin model' : 'Pin model'}
                        onClick={() => togglePinnedModel({ provider: choice.provider, llm_profile: choice.llm_profile, model: choice.model })}
                        className={`inline-flex size-7 shrink-0 items-center justify-center rounded text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring ${pinnedState ? 'text-foreground' : 'opacity-60 group-hover/row:opacity-100 focus-visible:opacity-100'}`}
                    >
                        <Star aria-hidden="true" className={`size-3.5 ${pinnedState ? 'fill-current' : ''}`} />
                    </button>
                )}
            </div>
        )
    }

    const visibleGroups = groups.map((group, index) => ({
        group,
        index,
        entries: groupChoices.filter((choice) => choice.groupIndex === index),
        isChecking: !group.profile && checking(group.provider),
    })).filter(({ entries, isChecking }) => entries.length > 0 || isChecking)

    const hasListContent = pinnedChoices.length > 0 || recentChoices.length > 0 || visibleGroups.length > 0 || Boolean(customChoice)

    return (
        <Field className={layout === 'compact' ? 'min-w-0 w-auto max-w-full' : 'w-full'}>
            {layout === 'fields' && <FieldLabel htmlFor={id}>Model</FieldLabel>}
            <div className={layout === 'compact' ? 'flex min-w-0 max-w-full items-center gap-1.5' : 'flex min-w-0 w-full items-center gap-2'}>
                <Popover.Root open={isOpen} onOpenChange={setOpen}>
                    <Popover.Trigger asChild>
                        <Button
                            id={id}
                            type="button"
                            variant="outline"
                            size={triggerSize}
                            disabled={disabled}
                            title={modelTooltip}
                            aria-label={modelAriaLabel}
                            aria-invalid={invalidModel}
                            aria-describedby={note}
                            data-inherited={isInherited ? 'true' : undefined}
                            className={`min-w-0 max-w-full justify-between border border-input text-foreground hover:bg-transparent focus-visible:border-ring ${layout === 'fields' ? 'flex-1' : 'px-2.5'} ${isUsingDefault ? 'text-muted-foreground' : ''} ${isInherited ? 'border-dashed' : ''}`}
                        >
                            <span className="flex min-w-0 items-center gap-1.5">
                                <span className="truncate">{modelText}</span>
                                {resolved.model && resolved.providerDisplay && (
                                    <span className="shrink-0 rounded bg-muted px-1 py-0.5 text-xs font-normal leading-none text-muted-foreground">
                                        {resolved.providerDisplay}
                                    </span>
                                )}
                                {layout === 'fields' && isUsingDefault && resolved.model && (
                                    <span className="shrink-0 text-xs font-normal text-muted-foreground">Default</span>
                                )}
                                {(activeSecondaryLabels.length > 0 || unavailableSecondaryCount > 0) && (
                                    <span
                                        aria-hidden="true"
                                        className={`size-1.5 shrink-0 rounded-full ${unavailableSecondaryCount > 0 ? 'bg-warning' : 'bg-primary'}`}
                                    />
                                )}
                            </span>
                            <ChevronDown aria-hidden="true" className="size-4 shrink-0 text-muted-foreground opacity-50" />
                        </Button>
                    </Popover.Trigger>
                    <Popover.Portal>
                        <Popover.Content
                            align="start"
                            sideOffset={6}
                            collisionPadding={8}
                            aria-label="Choose model"
                            className="z-50 flex max-h-[var(--radix-popover-content-available-height)] w-80 max-w-[calc(100vw-1rem)] flex-col rounded-md border border-border bg-popover p-2 text-popover-foreground shadow-md"
                            onOpenAutoFocus={(event) => {
                                event.preventDefault()
                                input.current?.focus({ preventScroll: true })
                            }}
                        >
                            <Input
                                className="shrink-0"
                                ref={input}
                                aria-label="Search models"
                                role="combobox"
                                aria-expanded="true"
                                aria-controls={`${id}-list`}
                                aria-autocomplete="list"
                                aria-activedescendant={highlighted ? `${id}-option-${highlighted.choiceIndex}` : undefined}
                                placeholder="Search models…"
                                value={search}
                                onChange={(event) => {
                                    setSearch(event.target.value)
                                    setActive(-1)
                                }}
                                onKeyDown={(event) => {
                                    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
                                        event.preventDefault()
                                        const next = choices.length
                                            ? (effectiveActive + (event.key === 'ArrowDown' ? 1 : effectiveActive < 0 ? 0 : -1) + choices.length) % choices.length
                                            : -1
                                        setActive(next)
                                        document.getElementById(`${id}-option-${next}`)?.scrollIntoView?.({ block: 'nearest' })
                                    } else if (event.key === 'Enter') {
                                        event.preventDefault()
                                        if (highlighted) commitModel(highlighted)
                                    }
                                }}
                            />
                            <Button
                                type="button"
                                variant="ghost"
                                className="my-1 h-auto w-full justify-start whitespace-normal text-left text-xs"
                                aria-pressed={!value.provider && !value.llm_profile && !value.model}
                                onClick={() => {
                                    onChange(EMPTY_MODEL_SETTINGS)
                                    setOpen(false)
                                }}
                            >
                                Use default · {describeDefault(defaults)}
                            </Button>
                            {!discovery && <p role="status" className="px-2 py-1 text-xs text-muted-foreground">Loading models…</p>}
                            <div id={`${id}-list`} role="listbox" aria-label="Models" className="min-h-0 max-h-[min(45vh,20rem)] overflow-y-auto">
                                {pinnedChoices.length > 0 && (
                                    <div role="group" aria-label="Pinned">
                                        <div className="px-2 pt-2 pb-1 text-xs font-medium text-muted-foreground">Pinned</div>
                                        {pinnedChoices.map((choice) => renderOption(choice, true))}
                                    </div>
                                )}
                                {recentChoices.length > 0 && (
                                    <div role="group" aria-label="Recent">
                                        <div className="px-2 pt-2 pb-1 text-xs font-medium text-muted-foreground">Recent</div>
                                        {recentChoices.map((choice) => renderOption(choice, true))}
                                    </div>
                                )}
                                {visibleGroups.map(({ group, entries, isChecking }) => (
                                    <div
                                        key={`${group.provider}:${group.profile}`}
                                        role="group"
                                        aria-label={`${providerLabel(group.provider)}${group.profile ? ` / ${group.label}` : ''}`}
                                    >
                                        <div className="px-2 pt-3 pb-1 text-xs font-medium text-muted-foreground">
                                            {providerLabel(group.provider)}{group.profile ? ` / ${group.label}` : ''}
                                            {isChecking && <span role="status" className="block font-normal">Checking…</span>}
                                        </div>
                                        {entries.map((choice) => renderOption(choice, false))}
                                    </div>
                                ))}
                                {customChoice && renderOption(customChoice, false)}
                                {discovery && !hasListContent && (
                                    <p role="status" className="px-2 py-2 text-xs text-muted-foreground">
                                        {query ? 'No matching models.' : 'No available models.'}
                                    </p>
                                )}
                            </div>

                            {hasSecondaryControls && (
                                <div className="mt-2 shrink-0 border-t border-border pt-2">
                                    <button
                                        type="button"
                                        aria-expanded={moreOptionsOpen}
                                        onClick={() => setMoreOptionsOverride(!moreOptionsOpen)}
                                        className="flex w-full items-center justify-between gap-2 rounded px-1.5 py-1 text-left text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
                                    >
                                        <span className="flex min-w-0 items-center gap-1">
                                            {moreOptionsOpen
                                                ? <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
                                                : <ChevronRight aria-hidden="true" className="size-3.5 shrink-0" />}
                                            <span className="font-medium">More options</span>
                                        </span>
                                        {(unavailableSecondaryCount > 0 || activeSecondaryLabels.length > 0) && (
                                            <span className={`truncate text-xs ${unavailableSecondaryCount > 0 ? 'text-warning' : 'text-foreground'}`}>
                                                {unavailableSecondaryCount > 0
                                                    ? `${unavailableSecondaryCount} unavailable`
                                                    : activeSecondaryLabels.join(' · ')}
                                            </span>
                                        )}
                                    </button>
                                    {moreOptionsOpen && (
                                        <div className="mt-1 space-y-2 px-1 pt-1">
                                            {controlsLocked && (
                                                <p className="text-xs text-muted-foreground">{effortHint}</p>
                                            )}
                                            {nativeSecondaryGroups.map(({ field, label, offeredOptions, current, optionList }) => (
                                                <div key={field} role="group" aria-label={label} className="flex flex-wrap gap-1 border-t border-border/60 pt-2 first:border-t-0 first:pt-0">
                                                    <span className="w-full text-xs text-muted-foreground">{label}</span>
                                                    {optionList.map((optionValue) => {
                                                        const isUnoffered = Boolean(optionValue && !offeredOptions.includes(optionValue))
                                                            && !(field === 'reasoning_mode' && optionValue === 'standard')
                                                        const nextFieldValue = field === 'reasoning_mode' && optionValue === 'standard' && offeredOptions.length === 0
                                                            ? null
                                                            : optionValue
                                                        return (
                                                            <Button
                                                                key={optionValue ?? 'default'}
                                                                type="button"
                                                                size="sm"
                                                                variant="ghost"
                                                                disabled={controlsLocked}
                                                                className="aria-pressed:bg-accent aria-pressed:text-accent-foreground"
                                                                aria-pressed={(current ?? (field === 'reasoning_mode' ? 'standard' : null)) === optionValue}
                                                                onClick={() => {
                                                                    const base = resolveEditableBase(true)
                                                                    if (!base) return
                                                                    setBudgetInput(null)
                                                                    onChange({
                                                                        ...base,
                                                                        [field]: nextFieldValue,
                                                                        ...(field === 'thinking'
                                                                            ? { thinking_budget_tokens: nextFieldValue === 'budget' ? base.thinking_budget_tokens ?? 1024 : null }
                                                                            : {}),
                                                                    })
                                                                }}
                                                            >
                                                                {optionValue
                                                                    ? `${effortLabel(optionValue)}${isUnoffered ? ' (saved)' : ''}`
                                                                    : 'Default'}
                                                            </Button>
                                                        )
                                                    })}
                                                    {field === 'thinking' && current === 'budget' && (
                                                        <Input
                                                            type="number"
                                                            aria-label="Thinking budget tokens"
                                                            min={1024}
                                                            step={1}
                                                            disabled={controlsLocked}
                                                            value={budgetInput ?? effective.thinking_budget_tokens ?? 1024}
                                                            onBlur={() => setBudgetInput(null)}
                                                            onChange={(event) => {
                                                                setBudgetInput(event.target.value)
                                                                const tokens = event.target.valueAsNumber
                                                                if (Number.isInteger(tokens) && tokens >= 1024) {
                                                                    const base = resolveEditableBase(true)
                                                                    if (base) onChange({ ...base, thinking_budget_tokens: tokens })
                                                                }
                                                            }}
                                                        />
                                                    )}
                                                </div>
                                            ))}
                                            {(controls.length > 0 || modes.length > 0) && (
                                                <div role="group" aria-label="Session controls" className="grid gap-2 border-t border-border/60 pt-2 first:border-t-0 first:pt-0">
                                                    <span className="text-xs text-muted-foreground">Session controls · changes apply from the next turn</span>
                                                    {controls.map((control) => (
                                                        <label key={control.id} className="grid gap-1 text-xs" title={control.description}>
                                                            {control.name}
                                                            <NativeSelect
                                                                size="sm"
                                                                aria-label={control.name}
                                                                disabled={controlsLocked}
                                                                value={effectiveSessionConfig[control.id] ?? ''}
                                                                onChange={(event) => setControl(control.id, event.target.value)}
                                                            >
                                                                <NativeSelectOption value="">Harness default</NativeSelectOption>
                                                                {valuesOf(control.values)}
                                                                {((saved) => saved && !control.values.some((entry) => entry.value === saved) && (
                                                                    <NativeSelectOption value={saved}>
                                                                        {saved}{checking(activeProvider ?? '') ? '' : ' (not offered now)'}
                                                                    </NativeSelectOption>
                                                                ))(effectiveSessionConfig[control.id])}
                                                            </NativeSelect>
                                                        </label>
                                                    ))}
                                                    {modes.length > 0 && (
                                                        <p className="text-xs text-muted-foreground">
                                                            Harness modes, set by the harness: {modes.map((mode) => mode.name).join(', ')}
                                                        </p>
                                                    )}
                                                </div>
                                            )}
                                        </div>
                                    )}
                                </div>
                            )}

                            {unavailableProviders.length > 0 && (
                                <div className="mt-2 shrink-0 border-t border-border pt-2">
                                    <button
                                        type="button"
                                        aria-expanded={unavailableOpen}
                                        onClick={() => setUnavailableOpen((prev) => !prev)}
                                        className="flex w-full items-center gap-1 rounded px-1.5 py-1 text-left text-xs text-muted-foreground hover:bg-accent hover:text-foreground"
                                    >
                                        {unavailableOpen
                                            ? <ChevronDown aria-hidden="true" className="size-3.5 shrink-0" />
                                            : <ChevronRight aria-hidden="true" className="size-3.5 shrink-0" />}
                                        <span>
                                            {unavailableProviders.length === 1
                                                ? '1 unavailable provider'
                                                : `${unavailableProviders.length} unavailable providers`}
                                        </span>
                                    </button>
                                    {unavailableOpen && (
                                        <div className="mt-1 max-h-40 space-y-2 overflow-y-auto px-2 py-1 text-xs">
                                            {unavailableProviders.map((provider) => (
                                                <div key={provider} role="group" aria-label={providerLabel(provider)}>
                                                    <div className="font-medium text-foreground">{providerLabel(provider)}</div>
                                                    <p role="status" className="text-muted-foreground">
                                                        {statuses?.[provider]?.setup_status === 'unauthenticated' ? 'Authentication required. ' : ''}
                                                        Model discovery unavailable. {statuses?.[provider]?.error}
                                                        {statuses?.[provider]?.setup_status ? ' Check and fix it under Settings › Models & accounts › Agent harnesses.' : ''}
                                                    </p>
                                                </div>
                                            ))}
                                        </div>
                                    )}
                                </div>
                            )}
                        </Popover.Content>
                    </Popover.Portal>
                </Popover.Root>

                {showEffortTrigger && (
                    <Popover.Root
                        open={effortOpen && !disabled && !controlsLocked}
                        onOpenChange={(next) => {
                            const target = next && !disabled && !controlsLocked
                            setEffortOpen(target)
                            if (target) refreshModelOptions(projectPath)
                        }}
                    >
                        <Popover.Trigger asChild>
                            <Button
                                type="button"
                                variant="outline"
                                size={triggerSize}
                                disabled={disabled || controlsLocked}
                                title={effortTooltip}
                                aria-label={effortAriaLabel}
                                aria-describedby={controlsLocked ? `${id}-effort-hint` : undefined}
                                className={`shrink-0 justify-between border border-input px-2.5 text-foreground hover:bg-transparent focus-visible:border-ring ${effortIsDefault ? 'text-muted-foreground' : ''} ${isInherited && !controlsLocked ? 'border-dashed' : ''}`}
                            >
                                <span>{effortTriggerText}</span>
                                <ChevronDown aria-hidden="true" className="size-3.5 shrink-0 text-muted-foreground opacity-50" />
                            </Button>
                        </Popover.Trigger>
                        <Popover.Portal>
                            <Popover.Content
                                align="end"
                                sideOffset={6}
                                collisionPadding={8}
                                aria-label="Choose reasoning effort"
                                className="z-50 w-56 max-w-[calc(100vw-1rem)] rounded-md border border-border bg-popover p-2 text-popover-foreground shadow-md"
                            >
                                <div role="group" aria-label="Reasoning effort" className="flex flex-wrap gap-1">
                                    <span className="w-full text-xs text-muted-foreground">Reasoning effort</span>
                                    {resolved.unverified && (
                                        <span className="w-full text-xs text-muted-foreground">Provider levels; unverified for this model.</span>
                                    )}
                                    {[null, ...allEfforts].map((effort) => (
                                        <Button
                                            key={effort ?? 'default'}
                                            type="button"
                                            size="sm"
                                            variant="ghost"
                                            className="aria-pressed:bg-accent aria-pressed:text-accent-foreground"
                                            aria-pressed={(isInherited ? effective.reasoning_effort : value.reasoning_effort) === effort}
                                            onClick={() => commitEffort(effort)}
                                        >
                                            {effort ? `${effortName(effort)}${supported.includes(effort) ? '' : ' (custom)'}` : 'Default'}
                                        </Button>
                                    ))}
                                </div>
                            </Popover.Content>
                        </Popover.Portal>
                    </Popover.Root>
                )}
            </div>
            {controlsLocked && <span id={`${id}-effort-hint`} className="sr-only">{effortHint}</span>}
            {invalidModel && <p id={`${id}-error`} role="alert">Choose a compatible model for this provider or profile.</p>}
            {unlisted && (
                <p id={`${id}-unlisted`} className="text-xs text-warning">
                    {providerLabel(value.provider!)} doesn’t list {value.model} right now. Choose a listed model unless it’s a custom one you know works.
                </p>
            )}
        </Field>
    )
}
