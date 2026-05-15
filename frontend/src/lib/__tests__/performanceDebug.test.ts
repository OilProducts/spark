import { describe, expect, it } from 'vitest'

import {
  PERFORMANCE_DEBUG_QUERY_PARAM,
  PERFORMANCE_DEBUG_STORAGE_KEY,
  isPerformanceDebugEnabled,
} from '@/lib/performanceDebug'

describe('performance debug flag', () => {
  it('is disabled by default and enabled by the query param or localStorage key', () => {
    window.history.pushState({}, '', '/')
    expect(isPerformanceDebugEnabled()).toBe(false)

    window.history.pushState({}, '', `/?${PERFORMANCE_DEBUG_QUERY_PARAM}=1`)
    expect(isPerformanceDebugEnabled()).toBe(true)

    window.history.pushState({}, '', '/')
    localStorage.setItem(PERFORMANCE_DEBUG_STORAGE_KEY, '1')
    expect(isPerformanceDebugEnabled()).toBe(true)
  })
})
