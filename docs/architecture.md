# Architecture

High-level repo structure:

```text
golish/
├── frontend/               # React 19 + TypeScript + Vite
│   ├── components/         # UI components
│   ├── hooks/              # Tauri event subscriptions
│   ├── lib/                # Typed invoke() wrappers
│   └── store/              # Zustand + Immer state
└── backend/crates/         # Rust workspace
    ├── golish/               # Main app: Tauri commands, CLI
    ├── golish-ai/            # Agent orchestration, LLM clients
    ├── golish-core/          # Foundation types
    └── ...
```

Related docs:
- [Planning system](planning-system.md)
- [System hooks](system-hooks.md)
- [Tool use](tool-use.md)
