import { useId, useMemo, useRef, useState } from 'react'
import { InlineError } from '@/components/app/inline-error'
import { Button } from '@/components/ui/button'
import { Checkbox } from '@/components/ui/checkbox'
import { Input } from '@/components/ui/input'
import { Label } from '@/components/ui/label'
import { NativeSelect, NativeSelectOption } from '@/components/ui/native-select'
import {
    REPLY_ACTION_LABELS,
    draftReply,
    formFields,
    initialDraft,
    replyLines,
    type FormDraft,
    type FormField,
    type FormQuestion,
    type FormReply,
} from '@/lib/forms'

// Unsent forms outlive the view, so leaving and coming back keeps what was entered.
// ponytail: module-level like the other unsent answers; lost on reload.
const draftsByKey = new Map<string, FormDraft>()

const INPUT_TYPES: Record<string, string> = { email: 'email', uri: 'url', date: 'date' }

interface FormQuestionFieldsProps {
    actionError: string | null
    draftKey: string
    form: FormQuestion
    isSubmitting: boolean
    onReply: (reply: FormReply) => void
    testId: string
}

/** A harness's form for the person to fill in, review, and then submit, decline or cancel. */
export function FormQuestionFields({ actionError, draftKey, form, isSubmitting, onReply, testId }: FormQuestionFieldsProps) {
    const idPrefix = useId()
    const fields = useMemo(() => formFields(form.schema), [form.schema])
    const [draft, setDraftState] = useState<FormDraft>(() => draftsByKey.get(draftKey) ?? initialDraft(fields))
    const [invalid, setInvalid] = useState<{ field: string; error: string } | null>(null)
    const formRef = useRef<HTMLFormElement>(null)
    const fieldId = (name: string) => `${idPrefix}-${name}`
    const setDraft = (name: string, value: string | string[]) => {
        setInvalid(null)
        setDraftState((current) => {
            const next = { ...current, [name]: value }
            draftsByKey.set(draftKey, next)
            return next
        })
    }
    const reply = (value: FormReply) => {
        setInvalid(null)
        draftsByKey.delete(draftKey)
        onReply(value)
    }
    const submit = () => {
        const result = draftReply(fields, draft)
        if ('error' in result) {
            setInvalid(result)
            formRef.current?.querySelector<HTMLElement>(`[data-form-field="${CSS.escape(result.field)}"]`)?.focus()
            return
        }
        reply(result.reply)
    }

    const renderControl = (field: FormField, describedBy: string | undefined) => {
        const value = draft[field.name] ?? (field.kind === 'multi' ? [] : '')
        const common = {
            id: fieldId(field.name),
            'data-form-field': field.name,
            'data-testid': `${testId}-field-${field.name}`,
            'aria-describedby': describedBy,
            'aria-invalid': invalid?.field === field.name || undefined,
            'aria-required': field.required || undefined,
            disabled: isSubmitting,
        }
        if (field.kind === 'multi') {
            const picks = value as string[]
            return (
                <div className="flex flex-wrap gap-x-4 gap-y-2">
                    {field.choices.map((choice, index) => {
                        const choiceId = `${fieldId(field.name)}-${index}`
                        return (
                            <div key={choice.value} className="flex items-center gap-2">
                                <Checkbox
                                    id={choiceId}
                                    data-form-field={index === 0 ? field.name : undefined}
                                    checked={picks.includes(choice.value)}
                                    disabled={isSubmitting}
                                    onCheckedChange={(checked) => setDraft(
                                        field.name,
                                        field.choices
                                            .map((option) => option.value)
                                            .filter((option) => (option === choice.value ? checked === true : picks.includes(option))),
                                    )}
                                />
                                <Label htmlFor={choiceId} className="font-normal">{choice.label}</Label>
                            </div>
                        )
                    })}
                </div>
            )
        }
        if (field.kind === 'boolean' || field.kind === 'choice') {
            const choices = field.kind === 'boolean'
                ? [{ value: 'true', label: 'Yes' }, { value: 'false', label: 'No' }]
                : field.choices
            return (
                <NativeSelect
                    {...common}
                    size="sm"
                    value={value as string}
                    onChange={(event) => setDraft(field.name, event.target.value)}
                    className="bg-background"
                >
                    <NativeSelectOption value="">{field.required ? 'Choose…' : 'Not set'}</NativeSelectOption>
                    {choices.map((choice) => (
                        <NativeSelectOption key={choice.value} value={choice.value}>{choice.label}</NativeSelectOption>
                    ))}
                </NativeSelect>
            )
        }
        const numeric = field.kind === 'number' || field.kind === 'integer'
        return (
            <Input
                {...common}
                type={numeric ? 'number' : INPUT_TYPES[field.format ?? ''] ?? 'text'}
                inputMode={field.kind === 'integer' ? 'numeric' : numeric ? 'decimal' : undefined}
                step={field.kind === 'integer' ? 1 : numeric ? 'any' : undefined}
                min={field.minimum ?? undefined}
                max={field.maximum ?? undefined}
                placeholder={field.format === 'date-time' ? '2026-01-31T09:00:00Z' : undefined}
                autoComplete="off"
                value={value as string}
                onChange={(event) => setDraft(field.name, event.target.value)}
                className="h-8 bg-background text-sm"
            />
        )
    }

    return (
        <form
            ref={formRef}
            noValidate
            data-testid={testId}
            aria-labelledby={`${idPrefix}-message`}
            className="space-y-3"
            onSubmit={(event) => {
                event.preventDefault()
                submit()
            }}
        >
            <div className="space-y-0.5">
                <p className="text-xs text-muted-foreground">Asked by {form.harnessLabel}</p>
                <p id={`${idPrefix}-message`} className="text-sm text-foreground">{form.message || 'Fill in the form.'}</p>
                <p className="text-xs text-muted-foreground">Don't enter passwords, keys or other secrets.</p>
            </div>
            {actionError && !invalid ? <InlineError className="mt-2" dense>{actionError}</InlineError> : null}
            {fields.map((field) => {
                const descriptionId = field.description ? `${fieldId(field.name)}-description` : null
                const errorId = invalid?.field === field.name ? `${fieldId(field.name)}-error` : null
                const describedBy = [descriptionId, errorId].filter(Boolean).join(' ') || undefined
                const label = (
                    <>
                        {field.label}
                        {field.required ? null : <>{' '}<span className="font-normal text-muted-foreground">(optional)</span></>}
                    </>
                )
                const details = (
                    <>
                        {descriptionId ? <p id={descriptionId} className="text-xs text-muted-foreground">{field.description}</p> : null}
                        {errorId ? <p id={errorId} role="alert" className="text-xs text-destructive">{invalid?.error}</p> : null}
                    </>
                )
                return field.kind === 'multi' ? (
                    <fieldset
                        key={field.name}
                        className="space-y-1.5"
                        aria-describedby={describedBy}
                        aria-invalid={errorId ? true : undefined}
                        data-testid={`${testId}-field-${field.name}`}
                    >
                        <legend className="text-sm font-medium">{label}</legend>
                        {renderControl(field, undefined)}
                        {details}
                    </fieldset>
                ) : (
                    <div key={field.name} className="space-y-1.5">
                        <Label htmlFor={fieldId(field.name)} className="block">{label}</Label>
                        {renderControl(field, describedBy)}
                        {details}
                    </div>
                )
            })}
            <div className="flex flex-wrap justify-end gap-2">
                <Button type="button" variant="ghost" size="xs" className="h-7" disabled={isSubmitting} onClick={() => reply({ action: 'cancel' })}>
                    Cancel
                </Button>
                <Button type="button" variant="outline" size="xs" className="h-7" disabled={isSubmitting} onClick={() => reply({ action: 'decline' })}>
                    Decline
                </Button>
                <Button type="submit" variant="outline" size="xs" className="h-7" disabled={isSubmitting} data-testid={`${testId}-submit`}>
                    {isSubmitting ? 'Submitting...' : 'Submit'}
                </Button>
            </div>
        </form>
    )
}

/** A sent form's reply: the action taken and, when accepted, each value given. */
export function FormReplySummary({ form, reply }: { form: FormQuestion; reply: FormReply }) {
    const lines = replyLines(formFields(form.schema), reply)
    return (
        <div className="space-y-2">
            <p className="text-sm text-foreground">{form.message || 'Form'}</p>
            <p className="text-xs text-muted-foreground">Asked by {form.harnessLabel} · {REPLY_ACTION_LABELS[reply.action]}</p>
            {reply.action === 'accept' && lines.length === 0 ? <p className="text-sm text-muted-foreground">No values given.</p> : null}
            {lines.length > 0 ? (
                <dl className="space-y-1">
                    {lines.map((line) => (
                        <div key={line.label} className="space-y-0.5">
                            <dt className="text-sm font-medium text-foreground">{line.label}</dt>
                            <dd className="text-sm text-muted-foreground">{line.value}</dd>
                        </div>
                    ))}
                </dl>
            ) : null}
        </div>
    )
}
