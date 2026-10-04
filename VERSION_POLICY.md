# Version numbers — do not change unless asked

**NEVER bump, rename, or “release” a new version automatically.**

Do **not** change version numbers in any file unless the project owner
**explicitly** says to (e.g. “bump to 1.0.9” or “update the version”).

- Do **not** bump versions as part of routine fixes, Docker CLI updates, docs, or AppImage rebuilds.
- Do **not** create a new GitHub release tag unless asked.
- Rebuilding and re-uploading the **same** version’s AppImage (e.g. clobber `v1.0.8`) is fine when fixing bugs at the current version.

Current version (leave as-is): **v1.0.8**

Files that carry the version (edit only when instructed):

- `handover_app/package.json` → `"version"`
- `handover_rust/Cargo.toml` → `[package] version`
- `handover_app/src-tauri/Cargo.toml` → `[package] version`
- `handover_app/src-tauri/tauri.conf.json` → `"version"`
- `README.md` → **Current release** line
- `VERSION_POLICY.md` → this “Current version” line

Agents and contributors: fix bugs and features without touching these version fields.
