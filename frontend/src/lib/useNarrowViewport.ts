import { useEffect, useState } from 'react'

// Below Tailwind's `lg` (1024px): a 1024px laptop or landscape tablet gets the desktop layout.
export const NARROW_VIEWPORT_MAX_WIDTH = 1023

const computeIsNarrowViewport = () => {
  if (typeof window === 'undefined') {
    return false
  }
  return window.innerWidth <= NARROW_VIEWPORT_MAX_WIDTH
}

export const useNarrowViewport = () => {
  const [isNarrowViewport, setIsNarrowViewport] = useState<boolean>(() => computeIsNarrowViewport())

  useEffect(() => {
    if (typeof window === 'undefined') {
      return () => undefined
    }
    const handleResize = () => {
      setIsNarrowViewport(computeIsNarrowViewport())
    }
    window.addEventListener('resize', handleResize)
    return () => {
      window.removeEventListener('resize', handleResize)
    }
  }, [])

  return isNarrowViewport
}
