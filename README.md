<div align="center">

# Nouve

**A tiny friend that lives at the top of your screen and keeps an eye on your AI coding agent sessions.**

Approve permissions, watch your agents work, drop a file, chat with Claude — all without leaving what you're doing.

![Windows 10/11](https://img.shields.io/badge/Windows-10%2F11-0078D4?logo=windows&logoColor=white)
![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131?logo=tauri&logoColor=black)
![License: MIT](https://img.shields.io/badge/license-MIT-green)

</div>

---

## About

**Nouve** is a fork of [Coucou](https://github.com/Louis-CFM/coucou) by Louis Raillé.

The original **UI and animations** — the island, the character, the sounds, the
feel — were designed and built by **Louis Raillé** (Louis-CFM), and are used here
under the MIT License. The **backend** — the brain, the memory, the agent
plumbing and the integrations — was reworked for Nouve by **Dhan4u**.

Nouve ships under its own name and its own identity, as the upstream asset
license requires. It does not use the Coucou or Mochi names, character or
artwork.

## Features

- 🤖 **Claude Code, Gemini CLI, Antigravity and other agents, live** — see every session at the top of your screen: what it reads, edits and runs, step by step. Tag a hook payload with `nouve_agent` to give any agent its own pill.
- ✅ **Approve from the island** — Claude Code permission requests show up with **Allow / Deny**. One click, back to work. Colour tells you what it is: calm green for routine, amber when an answer is awaited, red for something destructive.
- 💬 **Chat with Claude, or with Gemini and OpenAI models using your own keys** — click the model name above the chat box to switch provider and pick a model.
- 📎 **Drop a file on the island** — the character turns into a box and swallows it, then ask a question about it.
- 🔌 **Integrations** — Stripe payments, n8n workflows, GitHub, Vercel deployments, Resend emails, Notion, Cal.com. Each one gets its own little card.
- 🎭 **A real character** — idle breathing, blinks, eyes that follow your mouse, emotes, handcrafted sounds, a greeting on launch.
- 🫥 **Invisible when idle** — hides away when nothing is running, peeks out when you hover the top edge of the screen.
- 🔒 **Private by design** — no telemetry, no account. Keys live in your Windows Credential Manager or Linux Secret Service (GNOME Keyring, KWallet). The app only talks to the services you plug in.

## Install (Windows)

The Windows installer is built from source:

```powershell
git clone <your-repo-url>
cd <repo>/windows
npm install
npm run pack                # installer lands in windows/release/
```

There is no notch on a PC, so the island slides out of the top edge of the screen
instead of hiding inside one. See [`windows/README.md`](windows/README.md) for the
rest of the differences.

## Setup

Click the Nouve icon in the system tray → **Settings…**

| What | Why | Where the key goes |
|---|---|---|
| **Claude Code hooks** | live sessions and approvals | **Install hooks** — Nouve backs up `~/.claude/settings.json`, merges its hooks and shows you the diff before writing anything |
| **Anthropic API key** | chat and questions about files | Settings → Anthropic API · Windows Credential Manager / Secret Service |
| **OpenAI API key** | chat with OpenAI | Settings → Chat — other providers · Credential Manager |
| Stripe, n8n, GitHub, Vercel, Resend, Notion, Cal.com | the service cards | Credential Manager / Secret Service, all optional |

If Nouve isn't running, the hook exits immediately: **Claude Code is never blocked.**

## How it works

**Windows**

- A [Tauri 2](https://tauri.app) app (Rust + TypeScript): the island is a transparent, always-on-top window that never steals focus, the character is drawn in Canvas 2D.
- Claude Code hooks go through a tiny `nouve-hook.exe` and a named pipe; keys live in Windows Credential Manager.
- Details and differences in [`windows/README.md`](windows/README.md).

**Linux**

- The same Tauri app as Windows. On Wayland the island is a gtk-layer-shell overlay anchored to the top edge, and click-through is its input region.
- Claude Code hooks go through the same `nouve-hook`, over a Unix socket in `$XDG_RUNTIME_DIR`; keys live in the Secret Service.

## Contributing

Issues and PRs are welcome — new integrations, new emotes, new sounds, bug fixes.

## Credits

- **Louis Raillé** ([Louis-CFM](https://github.com/Louis-CFM)) — original **UI and animations**: the island, the character, the sounds and the feel. Upstream project: [coucou](https://github.com/Louis-CFM/coucou). Used under the MIT License.
- **Dhan4u** — **Nouve fork**: backend, brain, memory, agent plumbing and integrations.

## License

- **Code:** [MIT](LICENSE) — use it, fork it, learn from it, just keep the copyright notice (which is why Louis Raillé's copyright is kept alongside Dhan4u's).
- **Name, character, icon, sounds and media:** © Louis Raillé, all rights reserved — see [LICENSE-ASSETS.md](LICENSE-ASSETS.md). Nouve ships under its own name and identity.
