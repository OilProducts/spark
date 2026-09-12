import { afterEach, expect, it, vi } from 'vitest'
import { clientIdentity } from '../services/clientPreferences'

afterEach(() => { localStorage.clear(); delete window.__TAURI__; vi.restoreAllMocks() })
it('persists browser identity and uses the platform identity for Desktop', async () => {
    const id = await clientIdentity()
    expect(id).toMatch(/^browser-/)
    expect(await clientIdentity()).toBe(id)
    localStorage.clear()
    expect(await clientIdentity()).not.toBe(id)
    const invoke = vi.fn().mockResolvedValue('desktop-stable')
    window.__TAURI__ = { core: { invoke } }
    expect(await clientIdentity()).toBe('desktop-stable')
    localStorage.clear() // Simulate a different port/origin's empty browser storage.
    expect(await clientIdentity()).toBe('desktop-stable')
    expect(invoke).toHaveBeenCalledWith('desktop_client_identity')
    expect(localStorage.getItem('spark.client_id')).toBeNull()
})
it('reports corrupted identities without silently assigning a different preference owner', async () => {
    localStorage.setItem('spark.client_id', '../bad')
    await expect(clientIdentity()).rejects.toThrow('Invalid browser client identity')
    expect(localStorage.getItem('spark.client_id')).toBe('../bad')
})
