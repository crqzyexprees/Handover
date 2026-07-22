# Version numbers — do not change unless asked

**Do not bump** the app or backend version in any file unless the project owner explicitly requests it.

Current version (leave as-is): **v1.0.7**

Files that carry the version (edit only when instructed):

- `handover_app/package.json` → `"version"`
- `handover_rust/Cargo.toml` → `[package] version`
- `handover_app/src-tauri/tauri.conf.json` → `"version"`
- `README.md` → **Current release** line

Agents and contributors: fix bugs and features without touching these version fields.
