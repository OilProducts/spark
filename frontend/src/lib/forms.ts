// Typed form questions an agent harness asks (ACP form elicitation): reading the restricted
// schema, the person's draft, and the reply. The checks mirror the backend's, which stays the
// authority: a reply the backend refuses leaves the question pending.

/** A form as asked: its Spark request ID, who asks, why, and the original restricted schema. */
export interface FormQuestion {
    requestId: string
    harness: string
    harnessLabel: string
    message: string
    schema: Record<string, unknown>
}

/** A number sent as its exact JSON digits: a whole number beyond f64's exact integers. */
export interface ExactNumber {
    readonly rawJSON: string
}

export type FormValue = string | number | boolean | string[] | ExactNumber

export type FormReply =
    | { action: 'accept'; content: Record<string, FormValue> }
    | { action: 'decline' }
    | { action: 'cancel' }

export interface FormField {
    name: string
    label: string
    description: string | null
    required: boolean
    kind: 'text' | 'number' | 'integer' | 'boolean' | 'choice' | 'multi'
    format: string | null
    choices: Array<{ value: string; label: string; description?: string }>
    pattern: string | null
    minimum: number | null
    maximum: number | null
    minLength: number | null
    maxLength: number | null
    minItems: number | null
    maxItems: number | null
    defaultValue: FormValue | undefined
}

/**
 * What the person entered: text and numbers as typed, a choice's value (booleans 'true'/'false'),
 * picks as lists. `null` is a field left unset, which the reply omits; '' and [] are values. A
 * number field's empty text is unset too, since it can't be a number.
 */
export type FormDraft = Record<string, string | string[] | null>

const asRecord = (value: unknown): Record<string, unknown> | null => (
    value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null
)
const text = (value: unknown) => (typeof value === 'string' ? value : '')
const numberOrNull = (value: unknown) => (typeof value === 'number' ? value : null)

export function parseFormQuestion(value: unknown): FormQuestion | null {
    const record = asRecord(value)
    const schema = asRecord(record?.schema)
    if (!record || !schema) return null
    return {
        requestId: text(record.request_id),
        harness: text(record.harness),
        harnessLabel: text(record.harness_label) || text(record.harness) || 'The agent',
        message: text(record.message),
        schema,
    }
}

const titledChoices = (options: unknown) => (Array.isArray(options) ? options : [])
    .map(asRecord)
    .filter((option): option is Record<string, unknown> => typeof option?.const === 'string')
    .map((option) => ({
        value: option.const as string,
        label: text(option.title) || (option.const as string),
        description: text(option.description) || undefined,
    }))

const plainChoices = (values: unknown) => (Array.isArray(values) ? values : [])
    .filter((value): value is string => typeof value === 'string')
    .map((value) => ({ value, label: value || '(empty)' }))

export function formFields(schema: Record<string, unknown>): FormField[] {
    const required = Array.isArray(schema.required) ? schema.required : []
    return Object.entries(asRecord(schema.properties) ?? {}).map(([name, raw]) => {
        const property = asRecord(raw) ?? {}
        const items = asRecord(property.items) ?? {}
        const type = text(property.type)
        const choices = type === 'array'
            ? (items.anyOf ? titledChoices(items.anyOf) : plainChoices(items.enum))
            : (property.oneOf ? titledChoices(property.oneOf) : plainChoices(property.enum))
        const kind: FormField['kind'] = type === 'array' ? 'multi'
            : type === 'number' || type === 'integer' || type === 'boolean' ? type
                : choices.length > 0 ? 'choice' : 'text'
        return {
            name,
            label: text(property.title).trim() || name,
            description: text(property.description) || null,
            required: required.includes(name),
            kind,
            format: text(property.format) || null,
            choices,
            pattern: text(property.pattern) || null,
            minimum: numberOrNull(property.minimum),
            maximum: numberOrNull(property.maximum),
            minLength: numberOrNull(property.minLength),
            maxLength: numberOrNull(property.maxLength),
            minItems: numberOrNull(property.minItems),
            maxItems: numberOrNull(property.maxItems),
            defaultValue: property.default as FormValue | undefined,
        }
    })
}

/**
 * The draft a form opens with: its defaults filled in. A required text or multi-select field
 * starts empty, since '' and [] may answer it; anything else starts unset, as does a field whose
 * default is `null` (ACP's omitted) or of another type.
 */
export function initialDraft(fields: FormField[]): FormDraft {
    return Object.fromEntries(fields.map((field) => {
        const value = field.defaultValue
        if (field.kind === 'multi') {
            const picks = Array.isArray(value) ? value.filter((item) => field.choices.some((choice) => choice.value === item)) : null
            return [field.name, picks ?? (field.required ? [] : null)]
        }
        // ponytail: a whole number beyond f64's exact integers may have been rounded when the
        // schema was read, so it isn't offered; reading the schema with its source text would keep it.
        const offered = field.kind === 'number' || field.kind === 'integer'
            ? typeof value === 'number' && (Number.isSafeInteger(value) || !Number.isInteger(value))
            : typeof value === (field.kind === 'boolean' ? 'boolean' : 'string')
        if (offered) return [field.name, String(value)]
        return [field.name, field.required && field.kind === 'text' ? '' : null]
    }))
}

/** Whether a field may be left unset: optional, and its empty control is itself a value. */
export const canLeaveUnset = (field: FormField) => !field.required && (field.kind === 'text' || field.kind === 'multi')

const isEmail = (value: string) => {
    const at = value.indexOf('@')
    return !/\s/.test(value) && at > 0 && at < value.length - 1 && value.indexOf('@', at + 1) < 0
}

const isDate = (value: string) => {
    const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value)
    if (!match) return false
    const date = new Date(Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3])))
    return date.getUTCFullYear() === Number(match[1]) && date.getUTCMonth() === Number(match[2]) - 1 && date.getUTCDate() === Number(match[3])
}

const isDateTime = (value: string) => (
    /^\d{4}-\d{2}-\d{2}[Tt]\d{2}:\d{2}:\d{2}(\.\d+)?([Zz]|[+-]\d{2}:\d{2})$/.test(value)
    && isDate(value.slice(0, 10)) && !Number.isNaN(Date.parse(value))
)

const isUri = (value: string) => {
    try {
        new URL(value)
        return true
    } catch {
        return false
    }
}

const FORMAT_CHECKS: Record<string, (value: string) => boolean> = {
    email: isEmail,
    uri: isUri,
    date: isDate,
    'date-time': isDateTime,
}

const NUMBER_TEXT = /^[-+]?(\d+\.?\d*|\.\d+)(e[-+]?\d+)?$/i
const WHOLE_TEXT = /^[-+]?\d+$/
// JSON.rawJSON (ES2025) writes a number's exact digits; TypeScript's ES2022 lib doesn't declare it.
const rawJSON = (JSON as JSON & { rawJSON?: (text: string) => ExactNumber }).rawJSON

/**
 * The number a field's text sends, or why it can't. A whole number beyond f64's exact integers
 * is sent as the digits typed, so the value is never rounded on its way out.
 */
function readNumber(field: FormField, entered: string): { value: FormValue } | { error: string } {
    const { label } = field
    const text = entered.trim()
    const number = Number(text)
    const whole = WHOLE_TEXT.test(text)
    const integer = field.kind === 'integer'
    if (!NUMBER_TEXT.test(text) || !Number.isFinite(number) || (integer && !whole && !Number.isSafeInteger(number))) {
        return { error: integer ? `${label} must be a whole number.` : `${label} must be a number.` }
    }
    // Rounding to f64 keeps order, so these never refuse wrongly; a tie past f64's exact
    // integers (where the bound itself was read rounded) is the backend's, which compares exactly.
    if (field.minimum !== null && number < field.minimum) return { error: `${label} is below its minimum.` }
    if (field.maximum !== null && number > field.maximum) return { error: `${label} is above its maximum.` }
    if (!whole || Number.isSafeInteger(number)) return { value: number }
    if (!rawJSON) return { error: `${label} is too large for this browser to send exactly.` }
    return { value: rawJSON(BigInt(text).toString()) }
}

/** The field a draft value fails, and why; null when it is valid. */
function checkField(field: FormField, value: FormValue): string | null {
    const { label } = field
    if (field.kind === 'multi') {
        const picks = value as string[]
        if (field.minItems !== null && picks.length < field.minItems) return `${label} needs more choices.`
        if (field.maxItems !== null && picks.length > field.maxItems) return `${label} has too many choices.`
        return null
    }
    // A choice is still text, so it meets the field's other string constraints too.
    if (field.kind === 'choice' && !field.choices.some((choice) => choice.value === value)) {
        return `${label} must be one of its choices.`
    }
    if (field.kind !== 'text' && field.kind !== 'choice') return null
    const entered = value as string
    const length = [...entered].length
    if (field.minLength !== null && length < field.minLength) return `${label} is too short.`
    if (field.maxLength !== null && length > field.maxLength) return `${label} is too long.`
    if (field.pattern) {
        let pattern: RegExp | null = null
        try {
            pattern = new RegExp(field.pattern, 'u')
        } catch {
            // ponytail: a pattern JavaScript can't read is left to the backend's check.
        }
        if (pattern && !pattern.test(entered)) return `${label} does not match the required pattern.`
    }
    const check = field.format ? FORMAT_CHECKS[field.format] : undefined
    if (check && !check(entered)) return `${label} must be a valid ${field.format}.`
    return null
}

/**
 * The accept reply a draft makes, or the first field it fails and why. Unset optional fields
 * are omitted; false, zero, '' and [] are values.
 */
export function draftReply(fields: FormField[], draft: FormDraft): { reply: FormReply } | { field: string; error: string } {
    const content: Record<string, FormValue> = {}
    for (const field of fields) {
        const entered = draft[field.name]
        const numeric = field.kind === 'number' || field.kind === 'integer'
        if (entered === null || entered === undefined || (numeric && entered === '')) {
            if (field.required) return { field: field.name, error: `${field.label} is required.` }
            continue
        }
        const read = numeric ? readNumber(field, entered as string) : null
        if (read && 'error' in read) return { field: field.name, error: read.error }
        const value: FormValue = read ? read.value
            : field.kind === 'multi' ? [...entered]
                : field.kind === 'boolean' ? entered === 'true'
                    : entered as string
        const error = checkField(field, value)
        if (error) return { field: field.name, error }
        content[field.name] = value
    }
    return { reply: { action: 'accept', content } }
}

export function parseFormReply(value: unknown): FormReply | null {
    const record = asRecord(value)
    if (record?.action === 'decline' || record?.action === 'cancel') return { action: record.action }
    if (record?.action !== 'accept') return null
    return { action: 'accept', content: (asRecord(record.content) ?? {}) as Record<string, FormValue> }
}

/** A reply as its labelled values, in the form's order. */
export function replyLines(fields: FormField[], reply: FormReply): Array<{ label: string; value: string }> {
    if (reply.action !== 'accept') return []
    return fields
        .filter((field) => field.name in reply.content)
        .map((field) => {
            const value = reply.content[field.name]
            // ponytail: a reply read back from the server shows a whole number beyond f64's exact
            // integers rounded; what was sent and stored keeps every digit.
            const shown = Array.isArray(value)
                ? value.map((item) => field.choices.find((choice) => choice.value === item)?.label ?? item).join(', ') || 'None'
                : typeof value === 'boolean' ? (value ? 'Yes' : 'No')
                    : typeof value === 'object' && value !== null ? value.rawJSON
                        : field.choices.find((choice) => choice.value === value)?.label ?? String(value)
            return { label: field.label, value: shown || '(empty)' }
        })
}

export const REPLY_ACTION_LABELS: Record<FormReply['action'], string> = {
    accept: 'Submitted',
    decline: 'Declined',
    cancel: 'Cancelled',
}
