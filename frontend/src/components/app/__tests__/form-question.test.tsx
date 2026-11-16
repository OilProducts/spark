import { render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { FormQuestionFields, FormReplySummary } from '@/components/app/form-question'
import type { FormQuestion } from '@/lib/forms'

const form: FormQuestion = {
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
    expect(within(named).getByText("Don't enter passwords, keys or other secrets.")).toBeInTheDocument()
    expect(screen.getByRole('textbox', { name: 'Title' })).toHaveAccessibleDescription('Shown on the page.')
    expect(screen.getByRole('spinbutton', { name: 'Copies' })).toBeInTheDocument()
    expect(screen.getByRole('combobox', { name: 'Keep as draft (optional)' })).toHaveValue('true')
    expect(screen.getByRole('combobox', { name: 'Channel (optional)' })).toHaveValue('')
    expect(screen.getByRole('group', { name: 'Tags (optional)' })).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'News' })).not.toBeChecked()
    expect(document.querySelector('input[type="password"]')).toBeNull()
})

it('keeps an invalid draft for editing, then sends the reviewed typed reply', async () => {
    const user = userEvent.setup()
    const onReply = renderForm()
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).not.toHaveBeenCalled()
    const title = screen.getByRole('textbox', { name: 'Title' })
    expect(title).toHaveAttribute('aria-invalid', 'true')
    expect(title).toHaveAccessibleDescription('Shown on the page. Title is required.')
    expect(title).toHaveFocus()

    await user.type(title, 'Launch')
    expect(title).not.toHaveAttribute('aria-invalid')
    await user.type(screen.getByRole('spinbutton', { name: 'Copies' }), '0')
    await user.selectOptions(screen.getByRole('combobox', { name: 'Keep as draft (optional)' }), 'No')
    await user.click(screen.getByRole('checkbox', { name: 'Tech' }))
    await user.click(screen.getByRole('button', { name: 'Submit' }))
    expect(onReply).toHaveBeenCalledWith({ action: 'accept', content: { title: 'Launch', copies: 0, draft: false, tags: ['tech'] } })
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
    expect(screen.getByText('Copies').nextSibling).toHaveTextContent('0')
    expect(screen.getByText('Keep as draft').nextSibling).toHaveTextContent('No')
    expect(screen.getByText('Tags').nextSibling).toHaveTextContent('News, Tech')
    rerender(<FormReplySummary form={form} reply={{ action: 'decline' }} />)
    expect(screen.getByText('Asked by Writer Agent · Declined')).toBeInTheDocument()
    expect(screen.queryByText('Title')).toBeNull()
})
