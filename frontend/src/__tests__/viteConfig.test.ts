// @vitest-environment node
import { expect, it } from 'vitest'
import config from '../../vite.config'

it('proxies the public environment descriptor to Spark in development, beside the API mounts', () => {
    const proxy = config.server?.proxy ?? {}
    expect(proxy['/environment']).toBe(process.env.VITE_BACKEND_URL || 'http://127.0.0.1:8000')
    expect(Object.keys(proxy)).toEqual(expect.arrayContaining(['/attractor', '/workspace', '/environment']))
})
