import React from 'react'
import ReactDOM from 'react-dom/client'
import Root from './Root'
import './styles.css'
import { applyAppearance } from './appearance'

// Масштаб и размер текста — до первого кадра, чтобы окно не «прыгало».
applyAppearance()

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
)
