# Docker sandbox base image

Builds `handover-base:latest`, used by Rust sandboxes when starting Docker terminals.

Pre-installed CLIs (on `PATH` inside every sandbox):

- `claude` — Claude Code (`@anthropic-ai/claude-code`)
- `codex` — OpenAI Codex CLI (`@openai/codex`)
- `cursor-agent` / `agent` — Cursor CLI

Host auth/config is copied in at container start (`~/.claude`, `~/.codex`,
`~/.cursor`, `~/.config/cursor`, etc.). Absolute symlinks inside those dirs
(e.g. Codex `packages/app-server-daemon/current`) are rewritten to the sandbox
`$HOME` so tools keep working. **CLI binaries are not shared from the host** —
rebuild this image whenever you want newer Claude / Codex / Cursor versions in
Docker.

```bash
./handover/docker/build-base.sh
```

To force a fresh pull of Claude / Codex / Cursor (skip Docker layer cache):

```bash
./handover/docker/build-base.sh --no-cache
```

Or from `handover_app`:

```bash
npm run build:docker-base
```

Then open a **new** Docker terminal in Handover (existing containers keep the
old image until stopped/recreated).
