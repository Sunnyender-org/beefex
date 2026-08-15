import React from 'react'
import ReactDOM from 'react-dom/client'
import App from './App.tsx'
import { api } from './api/tauri'
import { installDiagnosticsErrorBridge } from './diagnostics/errorBridge'
import './index.css'

installDiagnosticsErrorBridge(api.recordRendererDiagnostic)

// React 应用入口文件
// 使用 createRoot 将 App 组件挂载到 DOM 的 #root 元素上
ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
)
