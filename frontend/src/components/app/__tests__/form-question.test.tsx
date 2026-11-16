import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FormQuestionFields, FormReplySummary } from '@/components/app/form-question'
import type { FormQuestion } from '@/lib/forms'

const form: FormQuestion = {
    requestId: 'acp-form-7',
    harness: 'acp:writer',
    harnessLabel: 'Writer Agent',
    message: 'How should I publish?',
    schema: {
        type: 'object',
        properties: {
            title: { type: 'string', title: 'Title', description: 'Shown on the page.' },
            copies: { type: 'integer', title: 'Copies', minimum: 0 },
            draft: { type: 'boolean', title: 'Keep as draft', default: true },
            channel: { type: 'string', title: 'Channel', enum: ['web', 'print'] },
            tags: { type: 'array', title: 'Tags', items: { anyOf: [{ const: 'news', title: 'News' }, { const: 'tech', title: 'Tech' }] } },
        },
        required: ['title', 'copies'],
    },
}

const renderForm = (onReply = vi.fn(), draftKey = 'form') => {
    render(<FormQuestionFields actionError={null} draftKey={draftKey} form={form} isSubmitting={false} onReply={onReply} testId="form" />)
    return onReply
}

it('labels each field, names the asking harness and never asks for a secret', () => {
    renderForm()
    const named = screen.getByRole('form', { name: 'How should I publish?' })
    expect(within(named).getByText('Asked by Writer Agent')).toBeInTheDocument()
    expect(within(named).getByText('acp-form-7').parentElement).toHaveTextContent('Request ID: acp-form-7')
    expect(within(named).getByText("Don't enter passwords, keys or other secrets.")).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'Title' })).toHaveAccessibleDescription('Shown on the page.')
    expect(screen.getByRole('textbox', { name: 'Copies' })).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: 'Keep as draft (optional)' })).toHaveDisplayValue('Yes')
    expect(screen.getByRole('combobox', { name: 'Channel (optional)' })).toHaveDisplayValue('Not set')
    expect(screen.getByRole('group', { name: 'Tags (optional)' })).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'News' })).not.toBeChecked()
    expect(screen.getByRole('checkbox', { name: 'Leave Tags unset' })).toBeChecked()
    expect(document.querySelector('input[type="password"]')).toBeNull()
})

it('keeps an invalid draft for editing, then sends the reviewed typed reply', async () => {
    const user = userEvent.setup()
    const onReply = renderForm()
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).not.toHaveBeenCalled()
    const copies = screen.getByRole('textbox', { name: 'Copies' })
    expect(copies).toHaveAttribute('aria-invalid', 'true')
    expect(copies).toHaveAccessibleDescription('Copies is required.')
    expect(copies).toHaveFocus()

    await user.type(copies, '0')
    expect(copies).not.toHaveAttribute('aria-invalid')
    await user.type(screen.getByRole('textbox', { name: 'Title' }), 'Launch')
    await user.selectOptions(screen.getByRole('combobox', { name: 'Keep as draft (optional)' }), 'No')
    await user.click(screen.getByRole('checkbox', { name: 'Tech' }))
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenCalledWith({ action: 'accept', content: { title: 'Launch', copies: 0, draft: false, tags: ['tech'] } })
})

it('sends an empty required text and an emptied multi-select, and omits what is left unset', async () => {
    const user = userEvent.setup()
    const onReply = renderForm(vi.fn(), 'empties')
    await user.type(screen.getByRole('textbox', { name: 'Copies' }), '2')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenLastCalledWith({ action: 'accept', content: { title: '', copies: 2, draft: true } })

    await user.click(screen.getByRole('checkbox', { name: 'News' }))
    expect(screen.getByRole('checkbox', { name: 'Leave Tags unset' })).not.toBeChecked()
    await user.click(screen.getByRole('checkbox', { name: 'News' }))
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenLastCalledWith({ action: 'accept', content: { title: '', copies: 2, draft: true, tags: [] } })

    await user.click(screen.getByRole('checkbox', { name: 'Leave Tags unset' }))
    await user.selectOptions(screen.getByRole('combobox', { name: 'Keep as draft (optional)' }), 'Not set')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenLastCalledWith({ action: 'accept', content: { title: '', copies: 2 } })
})

it('sends a whole number with every digit typed, and leaves a past-precision bound tie to the backend', async () => {
    const user = userEvent.setup()
    const onReply = vi.fn()
    render(<FormQuestionFields
        actionError={null} draftKey="exact" isSubmitting={false} onReply={onReply} testId="form"
        form={{ ...form, schema: { type: 'object', properties: {
            id: { type: 'integer', title: 'ID', maximum: 9007199254740991 },
            big: { type: 'integer', title: 'Big', default: 9007199254740992, maximum: 9007199254740992 },
            ratio: { type: 'number', title: 'Ratio', default: 0.5 },
        }, required: ['id', 'big'] } }}
    />)
    const id = screen.getByRole('textbox', { name: 'ID' })
    const big = screen.getByRole('textbox', { name: 'Big' })
    expect(big).toHaveValue('')
    expect(screen.getByRole('textbox', { name: 'Ratio (optional)' })).toHaveValue('0.5')
    await user.type(id, '9007199254740992')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(id).toHaveAccessibleDescription('ID is above its maximum.')
    await user.clear(id)
    await user.type(id, '-9007199254740993')
    await user.type(big, '9007199254740993')
    expect(big).toHaveValue('9007199254740993')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(JSON.stringify(onReply.mock.lastCall?.[0])).toBe('{"action":"accept","content":{"id":-9007199254740993,"big":9007199254740993,"ratio":0.5}}')
})

it('reads ACP nulls as omitted and shows what each choice means', async () => {
    const user = userEvent.setup()
    const onReply = vi.fn()
    render(<FormQuestionFields
        actionError={null} draftKey="nulls" isSubmitting={false} onReply={onReply} testId="form"
        form={{ ...form, schema: { type: 'object', required: null, properties: {
            plan: { type: 'string', title: 'Plan', enum: null, default: null, minLength: null, oneOf: [
                { const: 'free', title: 'Free', description: 'No cost', _meta: { k: 1 } },
                { const: 'pro', title: 'Pro', description: null },
            ] },
            count: { type: 'integer', title: 'Count', minimum: null, default: null },
            tags: { type: 'array', title: 'Tags', default: null, items: { anyOf: [{ const: 'a', title: 'A', description: 'First' }], _meta: null } },
        } } }}
    />)
    const plan = screen.getByRole('combobox', { name: 'Plan (optional)' })
    expect(plan).toHaveDisplayValue('Not set')
    expect(plan).toHaveAccessibleDescription('Free: No cost')
    expect(screen.getByRole('group', { name: 'Tags (optional)' })).toHaveAccessibleDescription('A: First')
    expect(screen.getByRole('textbox', { name: 'Count (optional)' })).toHaveValue('')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenLastCalledWith({ action: 'accept', content: {} })
    await user.type(screen.getByRole('textbox', { name: 'Count (optional)' }), '-3')
    await user.selectOptions(plan, 'Pro')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenLastCalledWith({ action: 'accept', content: { plan: 'pro', count: -3 } })
})

it('offers a choice whose value is empty, apart from leaving it unset', async () => {
    const user = userEvent.setup()
    const onReply = vi.fn()
    render(<FormQuestionFields
        actionError={null} draftKey="empty-choice" isSubmitting={false} onReply={onReply} testId="form"
        form={{ ...form, schema: { type: 'object', properties: { size: { type: 'string', title: 'Size', enum: ['', 's'] } }, required: ['size'] } }}
    />)
    const size = screen.getByRole('combobox', { name: 'Size' })
    expect(size).toHaveDisplayValue('Choose…')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(size).toHaveAccessibleDescription('Size is required.')
    await user.selectOptions(size, '(empty)')
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenCalledWith({ action: 'accept', content: { size: '' } })
})

it('declines or cancels without asking for answers', async () => {
    const user = userEvent.setup()
    const onReply = renderForm(vi.fn(), 'other')
    await user.click(screen.getByRole('button', { name: 'Decline' }))
    expect(onReply).toHaveBeenLastCalledWith({ action: 'decline' })
    await user.click(screen.getByRole('button', { name: 'Cancel' }))
    expect(onReply).toHaveBeenLastCalledWith({ action: 'cancel' })
})

it('summarizes a sent reply by its labels and choice titles', () => {
    const { rerender } = render(<FormReplySummary form={form} reply={{ action: 'accept', content: { title: 'Launch', copies: 0, draft: false, tags: ['news', 'tech'] } }} />)
    expect(screen.getByText('Asked by Writer Agent · Submitted')).toBeInTheDocument()
    expect(screen.getByText('acp-form-7')).toBeInTheDocument()
    expect(screen.getByText('Copies').nextSibling).toHaveTextContent('0')
    expect(screen.getByText('Keep as draft').nextSibling).toHaveTextContent('No')
    expect(screen.getByText('Tags').nextSibling).toHaveTextContent('News, Tech')
    rerender(<FormReplySummary form={form} reply={{ action: 'decline' }} />)
    expect(screen.getByText('Asked by Writer Agent · Declined')).toBeInTheDocument()
    expect(screen.queryByText('Title')).toBeNull()
})
