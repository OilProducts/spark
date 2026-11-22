import { type Page, type Locator } from '@playwright/test'

export async function openPicker(page: Page, scope: Page | Locator = page) {
    if (!await page.getByRole('listbox', { name: 'Models' }).isVisible()) await scope.getByRole('button', { name: /^Model:/ }).click()
}
export async function openEffortPicker(page: Page, scope: Page | Locator = page) {
    if (!await page.getByRole('menu', { name: 'Reasoning effort' }).isVisible()) await scope.getByRole('button', { name: /^Reasoning effort:/ }).click()
}
export async function chooseModel(page: Page, group: string, model: string, scope: Page | Locator = page) {
    await openPicker(page, scope)
    await page.getByRole('combobox', { name: 'Search models' }).fill('')
    await page.getByRole('group', { name: group, exact: true }).getByRole('option', { name: model, exact: true }).click()
}
export async function customModel(page: Page, model: string, scope: Page | Locator = page) {
    await openPicker(page, scope)
    await page.getByRole('combobox', { name: 'Search models' }).fill(model)
    await page.getByRole('option', { name: `Use "${model}" as a custom model` }).click()
}
export async function chooseEffort(page: Page, effort: string, scope: Page | Locator = page) {
    await openEffortPicker(page, scope)
    const menu = page.getByRole('menu', { name: 'Reasoning effort' })
    await (effort === 'Default'
        ? menu.getByRole('menuitemradio', { name: /^Default(?: · .+)?$/ })
        : menu.getByRole('menuitemradio', { name: effort, exact: true })
    ).click()
}
