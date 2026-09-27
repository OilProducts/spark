import { screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

type User = ReturnType<typeof userEvent.setup>
export async function openPicker(user: User, scope = screen) {
    if (!screen.queryByRole('listbox', { name: 'Models' })) await user.click(scope.getByRole('button', { name: /^Model:/ }))
}
export async function chooseModel(user: User, group: string, model: string, scope = screen) {
    await openPicker(user, scope)
    await user.clear(screen.getByRole('combobox', { name: 'Search models' }))
    await user.click(within(await screen.findByRole('group', { name: group, exact: true })).getByRole('option', { name: model, exact: true }))
    await user.keyboard('{Escape}')
}
export async function customModel(user: User, model: string, scope = screen) {
    await openPicker(user, scope)
    await user.clear(screen.getByRole('combobox', { name: 'Search models' }))
    await user.type(screen.getByRole('combobox', { name: 'Search models' }), model)
    await user.click(screen.getByRole('option', { name: `Use "${model}" as a custom model` }))
    await user.keyboard('{Escape}')
}
export async function chooseEffort(user: User, effort: string, scope = screen) {
    await openPicker(user, scope)
    await user.click(within(screen.getByRole('group', { name: 'Reasoning effort' })).getByRole('button', { name: effort, exact: true }))
}
