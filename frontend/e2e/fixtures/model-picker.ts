import { type Page, type Locator } from '@playwright/test'

export async function openPicker(page: Page, scope: Page | Locator = page) {
    if (!await page.getByRole('listbox', { name: 'Models' }).isVisible()) await scope.getByRole('button', { name: /^Model:/ }).click()
}
export async function chooseModel(page: Page, group: string, model: string, scope: Page | Locator = page) {
    await openPicker(page, scope)
    await page.getByRole('combobox', { name: 'Search models' }).fill('')
    await page.getByRole('group', { name: group, exact: true }).getByRole('option', { name: model, exact: true }).click()
    await page.keyboard.press('Escape')
}
export async function customModel(page: Page, model: string, scope: Page | Locator = page) {
    await openPicker(page, scope)
    await page.getByRole('combobox', { name: 'Search models' }).fill(model)
    await page.getByRole('option', { name: `Use "${model}" as a custom model` }).click()
    await page.keyboard.press('Escape')
}
export async function chooseEffort(page: Page, effort: string) {
    await openPicker(page)
    await page.getByRole('group', { name: 'Reasoning effort' }).getByRole('button', { name: effort, exact: true }).click()
}
