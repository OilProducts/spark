import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'
import { applyTheme, cachedAppearance } from './lib/theme'

// Shows the last appearance at once; a new browser's System isn't a choice it made, so it isn't recorded.
applyTheme(cachedAppearance(), false)

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
