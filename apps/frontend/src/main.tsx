import React from 'react';
import { createRoot } from 'react-dom/client';
import './style.css';

function App() {
  return (
    <main className="app-shell">
      <p className="eyebrow">Sanchoris</p>
      <h1>Frontend is ready.</h1>
      <p>pnpm, Vite, React, and TypeScript are wired for this monorepo.</p>
    </main>
  );
}

const root = document.getElementById('root');

if (!root) {
  throw new Error('Root element was not found.');
}

createRoot(root).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
