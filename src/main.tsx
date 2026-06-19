import { StrictMode } from 'react'
import { createRoot } from 'react-dom/client'
import './index.css'
import App from './App.tsx'

// Prevent default browser-like keyboard shortcuts (Caret browsing, Refresh)
window.addEventListener('keydown', (e) => {
  if (e.key === 'F7' || e.key === 'F5' || (e.ctrlKey && e.key.toLowerCase() === 'r') || (e.ctrlKey && e.key.toLowerCase() === 'f')) {
    e.preventDefault();
  }
});

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
