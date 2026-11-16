import { draftReply, formFields, initialDraft, parseFormQuestion, parseFormReply, replyLines, type FormDraft } from '@/lib/forms'

const schema = {
    type: 'object',
    properties: {
        name: { type: 'string', title: 'Name', minLength: 2, maxLength: 5 },
        email: { type: 'string', format: 'email' },
        code: { type: 'string', pattern: '^[A-Z]{3}$' },
        size: { type: 'string', enum: ['s', 'm'] },
        tier: { type: 'string', oneOf: [{ const: 'free', title: 'Free' }, { const: 'pro', title: 'Pro' }] },
        count: { type: 'integer', minimum: 0, maximum: 3 },
        ratio: { type: 'number' },
        ok: { type: 'boolean', default: true },
        tags: { type: 'array', items: { type: 'string', enum: ['a', 'b'] }, minItems: 1 },
        roles: { type: 'array', items: { anyOf: [{ const: 'dev', title: 'Developer' }] } },
    },
    required: ['name', 'count', 'tags'],
}
const fields = formFields(schema)
const reply = (draft: FormDraft) => draftReply(fields, { ...initialDraft(fields), ...draft })

it('reads every restricted field type and opens on its defaults', () => {
    expect(fields.map((field) => [field.name, field.kind, field.required])).toEqual([
        ['name', 'text', true], ['email', 'text', false], ['code', 'text', false], ['size', 'choice', false],
        ['tier', 'choice', false], ['count', 'integer', true], ['ratio', 'number', false], ['ok', 'boolean', false],
        ['tags', 'multi', true], ['roles', 'multi', false],
    ])
    expect(fields[4].choices).toEqual([{ value: 'free', label: 'Free' }, { value: 'pro', label: 'Pro' }])
    expect(fields[9].choices).toEqual([{ value: 'dev', label: 'Developer' }])
    expect(initialDraft(fields)).toMatchObject({ name: '', email: null, size: null, count: null, ok: 'true', tags: [], roles: null })
})

it('sends typed values, keeps zero and false, and omits what is unset', () => {
    expect(reply({ name: 'Ada', count: '0', ok: 'false', tags: ['b'] })).toEqual({
        reply: { action: 'accept', content: { name: 'Ada', count: 0, ok: false, tags: ['b'] } },
    })
    expect(reply({ name: 'Ada', count: '2', ratio: '0.5', size: 'm', tier: 'pro', email: 'a@b.c', code: 'ABC', tags: ['a', 'b'], roles: ['dev'], ok: null })).toEqual({
        reply: { action: 'accept', content: { name: 'Ada', email: 'a@b.c', code: 'ABC', size: 'm', tier: 'pro', count: 2, ratio: 0.5, tags: ['a', 'b'], roles: ['dev'] } },
    })
})

it('names the first field an invalid draft fails, as the backend does', () => {
    const valid = { name: 'Ada', count: '1', tags: ['a'] }
    expect(reply({ ...valid, name: null })).toEqual({ field: 'name', error: 'Name is required.' })
    expect(reply({ ...valid, name: '' })).toEqual({ field: 'name', error: 'Name is too short.' })
    expect(reply({ ...valid, count: '' })).toEqual({ field: 'count', error: 'count is required.' })
    expect(reply({ ...valid, name: 'A' })).toEqual({ field: 'name', error: 'Name is too short.' })
    expect(reply({ ...valid, name: 'Adaline' })).toEqual({ field: 'name', error: 'Name is too long.' })
    expect(reply({ ...valid, email: 'ada' })).toEqual({ field: 'email', error: 'email must be a valid email.' })
    expect(reply({ ...valid, code: 'abc' })).toEqual({ field: 'code', error: 'code does not match the required pattern.' })
    expect(reply({ ...valid, count: '1.5' })).toEqual({ field: 'count', error: 'count must be a whole number.' })
    expect(reply({ ...valid, count: '4' })).toEqual({ field: 'count', error: 'count is above its maximum.' })
    expect(reply({ ...valid, count: '-1' })).toEqual({ field: 'count', error: 'count is below its minimum.' })
    expect(reply({ ...valid, ratio: 'x' })).toEqual({ field: 'ratio', error: 'ratio must be a number.' })
    expect(reply({ ...valid, size: 'xl' })).toEqual({ field: 'size', error: 'size must be one of its choices.' })
    expect(reply({ ...valid, tags: [] })).toEqual({ field: 'tags', error: 'tags needs more choices.' })
})

it('checks a choice against its string constraints too', () => {
    const constrained = formFields({ type: 'object', properties: {
        pick: { type: 'string', enum: ['a', 'bb'], minLength: 2 },
        code: { type: 'string', pattern: '^[A-Z]', oneOf: [{ const: 'x1', title: 'X' }, { const: 'Y1', title: 'Y' }] },
    } })
    expect(draftReply(constrained, { pick: 'a', code: null })).toEqual({ field: 'pick', error: 'pick is too short.' })
    expect(draftReply(constrained, { pick: 'bb', code: 'x1' })).toEqual({ field: 'code', error: 'code does not match the required pattern.' })
    expect(draftReply(constrained, { pick: 'bb', code: 'Y1' })).toEqual({ reply: { action: 'accept', content: { pick: 'bb', code: 'Y1' } } })
})

it('sends empty text, an empty choice and an emptied multi-select as values, apart from leaving them unset', () => {
    const empties = formFields({ type: 'object', properties: {
        note: { type: 'string' },
        pick: { type: 'string', enum: ['', 'a'], default: '' },
        tags: { type: 'array', items: { type: 'string', enum: ['a'] } },
        aside: { type: 'string' },
    }, required: ['note', 'pick'] })
    const draft = initialDraft(empties)
    expect(draft).toEqual({ note: '', pick: '', tags: null, aside: null })
    expect(empties[1].choices).toEqual([{ value: '', label: '(empty)' }, { value: 'a', label: 'a' }])
    expect(draftReply(empties, draft)).toEqual({ reply: { action: 'accept', content: { note: '', pick: '' } } })
    expect(draftReply(empties, { ...draft, tags: [], aside: '' })).toEqual({
        reply: { action: 'accept', content: { note: '', pick: '', tags: [], aside: '' } },
    })
    expect(draftReply(empties, { ...draft, pick: null })).toEqual({ field: 'pick', error: 'pick is required.' })
})

it('sends numbers as typed: exact past f64 precision, and never a text JSON would read differently', () => {
    const numbers = formFields({ type: 'object', properties: { n: { type: 'integer' }, x: { type: 'number' } } })
    const sent = (draft: FormDraft) => {
        const result = draftReply(numbers, { n: null, x: null, ...draft })
        return 'reply' in result ? JSON.stringify(result.reply) : result.error
    }
    expect(sent({ n: '9007199254740993', x: '-18446744073709551615' })).toBe('{"action":"accept","content":{"n":9007199254740993,"x":-18446744073709551615}}')
    expect(sent({ n: ' 007 ', x: '1e3' })).toBe('{"action":"accept","content":{"n":7,"x":1000}}')
    expect(sent({ n: '1e3', x: '.5' })).toBe('{"action":"accept","content":{"n":1000,"x":0.5}}')
    expect(sent({ n: '9.007199254740993e15' })).toBe('n must be a whole number.')
    for (const text of [' ', 'Infinity', '0x10', '1e400', '1,5']) expect(sent({ x: text })).toBe('x must be a number.')
})

it('opens on defaults of the field\'s own type, reading null as none', () => {
    const defaults = formFields({ type: 'object', properties: {
        n: { type: 'integer', default: null }, big: { type: 'integer', default: 2 ** 60 }, x: { type: 'number', default: 1e-7 },
        s: { type: 'string', default: null }, b: { type: 'boolean', default: false },
        tags: { type: 'array', items: { type: 'string', enum: ['a', 'b'] }, default: ['b', 'gone'] },
    } })
    expect(initialDraft(defaults)).toEqual({ n: null, big: null, x: '1e-7', s: null, b: 'false', tags: ['b'] })
    expect(draftReply(defaults, initialDraft(defaults))).toEqual({ reply: { action: 'accept', content: { x: 1e-7, b: false, tags: ['b'] } } })
})

it('accepts an empty reply when no field is required', () => {
    const empty = formFields({ type: 'object', properties: { note: { type: 'string' } } })
    expect(draftReply(empty, initialDraft(empty))).toEqual({ reply: { action: 'accept', content: {} } })
    expect(draftReply([], {})).toEqual({ reply: { action: 'accept', content: {} } })
})

it('reads forms and replies off the wire and shows a reply in labelled lines', () => {
    expect(parseFormQuestion({ request_id: 'acp-form-1', harness: 'acp:x', harness_label: 'X Agent', message: 'Pick', schema })).toMatchObject({ requestId: 'acp-form-1', harnessLabel: 'X Agent', message: 'Pick' })
    expect(parseFormQuestion({ message: 'no schema' })).toBeNull()
    expect(parseFormReply({ action: 'decline' })).toEqual({ action: 'decline' })
    expect(parseFormReply({ action: 'accept' })).toEqual({ action: 'accept', content: {} })
    expect(parseFormReply('Declined.')).toBeNull()
    expect(replyLines(fields, { action: 'accept', content: { name: 'Ada', tier: 'pro', ok: false, count: 0, roles: ['dev'] } })).toEqual([
        { label: 'Name', value: 'Ada' }, { label: 'tier', value: 'Pro' }, { label: 'count', value: '0' },
        { label: 'ok', value: 'No' }, { label: 'roles', value: 'Developer' },
    ])
    expect(replyLines(fields, { action: 'cancel' })).toEqual([])
})
