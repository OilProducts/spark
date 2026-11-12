import { useSyncExternalStore } from 'react'

// Below Tailwind's `lg` (1024px): a 1024px laptop or landscape tablet gets the desktop layout.
export const NARROW_VIEWPORT_MAX_WIDTH = 1023

const computeIsNarrowViewport = () => {
  if (typeof window === 'undefined') {
    return false
  }
  return window.innerWidth <= NARROW_VIEWPORT_MAX_WIDTH
}

// One resize listener tells every user at once, so the whole app switches layout in one commit.
// With a listener each, React commits each switch on its own, and an effect that measures the
// page (scrolling a picked run into view) runs while the shell is still in the other layout.
const listeners = new Set<() => void>()
const notify = () => listeners.forEach((listener) => listener())

const subscribe = (listener: () => void) => {
  if (listeners.size === 0) {
    window.addEventListener('resize', notify)
  }
  listeners.add(listener)
  return () => {
    listeners.delete(listener)
    if (listeners.size === 0) {
      window.removeEventListener('resize', notify)
    }
  }
}

export const useNarrowViewport = () => useSyncExternalStore(subscribe, computeIsNarrowViewport, () => false)
