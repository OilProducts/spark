import '@testing-library/jest-dom/vitest'
import { cleanup } from '@testing-library/react'
import { afterEach, beforeEach, vi } from 'vitest'

type ResizeObserverCallback = ConstructorParameters<typeof ResizeObserver>[0]

class ResizeObserverMock {
  static readonly instances = new Set<ResizeObserverMock>()
  private readonly callback: ResizeObserverCallback
  private readonly observedElements = new Set<Element>()

  constructor(callback: ResizeObserverCallback) {
    this.callback = callback
    ResizeObserverMock.instances.add(this)
  }

  observe(target: Element) {
    this.observedElements.add(target)
  }

  unobserve(target: Element) {
    this.observedElements.delete(target)
  }

  disconnect() {
    this.observedElements.clear()
    ResizeObserverMock.instances.delete(this)
  }

  static notify(target: Element) {
    const entry = {
      target,
      contentRect: target.getBoundingClientRect(),
    } as ResizeObserverEntry

    ResizeObserverMock.instances.forEach((observer) => {
      if (!observer.observedElements.has(target)) {
        return
      }
      observer.callback([entry], observer as unknown as ResizeObserver)
    })
  }

  static reset() {
    ResizeObserverMock.instances.clear()
  }
}

const createStorageMock = () => {
  const entries = new Map<string, string>()
  return {
    get length() {
      return entries.size
    },
    clear() {
      entries.clear()
    },
    getItem(key: string) {
      return entries.get(String(key)) ?? null
    },
    key(index: number) {
      return Array.from(entries.keys())[index] ?? null
    },
    removeItem(key: string) {
      entries.delete(String(key))
    },
    setItem(key: string, value: string) {
      entries.set(String(key), String(value))
    },
  } satisfies Storage
}

const localStorageMock = createStorageMock()
const sessionStorageMock = createStorageMock()

globalThis.ResizeObserver = ResizeObserverMock as unknown as typeof ResizeObserver

Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: localStorageMock,
})

Object.defineProperty(globalThis, 'sessionStorage', {
  configurable: true,
  value: sessionStorageMock,
})

if (typeof window !== 'undefined') {
  Object.defineProperty(window, 'localStorage', {
    configurable: true,
    value: localStorageMock,
  })

  Object.defineProperty(window, 'sessionStorage', {
    configurable: true,
    value: sessionStorageMock,
  })
}

if (typeof Element !== 'undefined') {
  if (typeof Element.prototype.scrollIntoView !== 'function') {
    Object.defineProperty(Element.prototype, 'scrollIntoView', {
      configurable: true,
      value: () => {},
    })
  }
  if (typeof Element.prototype.hasPointerCapture !== 'function') {
    Object.defineProperty(Element.prototype, 'hasPointerCapture', {
      configurable: true,
      value: () => false,
    })
  }
  if (typeof Element.prototype.setPointerCapture !== 'function') {
    Object.defineProperty(Element.prototype, 'setPointerCapture', {
      configurable: true,
      value: () => {},
    })
  }
  if (typeof Element.prototype.releasePointerCapture !== 'function') {
    Object.defineProperty(Element.prototype, 'releasePointerCapture', {
      configurable: true,
      value: () => {},
    })
  }
}

beforeEach(() => {
  vi.stubGlobal('confirm', vi.fn(() => true))
})

afterEach(() => {
  cleanup()
  ResizeObserverMock.reset()
  if (typeof localStorage?.clear === 'function') {
    localStorage.clear()
  }
  if (typeof sessionStorage?.clear === 'function') {
    sessionStorage.clear()
  }
  if (typeof globalThis.confirm === 'function' && 'mockClear' in globalThis.confirm) {
    ;(globalThis.confirm as unknown as { mockClear: () => void }).mockClear()
  }
})
