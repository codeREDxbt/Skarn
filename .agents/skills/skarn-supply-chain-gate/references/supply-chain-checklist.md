# Supply-Chain & De-Brand Checklist

## Hardcoded URLs to Consolidate to `[workspace.package] repository`
- `apps/selora-desktop/src-tauri/src/state.rs` (hardcoded `https://api.github.com/repos/ToxicAngelX/Selora-IDE/releases/latest`)
- `apps/selora-desktop/frontend/src/App.tsx` (docs URL)
- `apps/selora-desktop/frontend/src/AgentsWindowApp.tsx` (docs URL)
- `apps/selora-desktop/frontend/package.json` (`repository.url`)
- `installer/selora-ide.iss` (`RepoUrl`, `AppPublisher`, `OutputBaseFilename`)

## Gateway Removal Sites
- `features/auth/ConnectModal.tsx`: remove `id: 'selora'` preset, `https://api.selora.lol` default, `sk-selora-`/`sk-gw-` validation & hints, `'Recommended'` tag, `"Gateway"` copy.
- `features/auth/provider-visibility.ts`: remove `haystack.includes('selora')` heuristic.
- `ConnectModal.providers.test.tsx` & `provider-visibility.test.ts`: rewrite around neutral provider fixtures.
- Rust side: remove any `api.selora.lol` default, `x-selora-*` headers, `selora/` UA, or `'selora'` in `disabled_providers`.

## Allowlisted `selora` Occurrences at Final Release (`--strict`)
- `LICENSE` (original MIT attribution)
- `README.md` (single attribution line: "This project began as Selora IDE...")
- `CHANGELOG.md` (historical entries + "1.0.0 — renamed from Selora IDE")
- `docs/research/*` and root research dossiers (`REBRAND_PLAN.md`, `SELORA_*.md`, `ZCODE_*.md`, `SKARN_MASTER_*.md`)
- v1.0 migration bridges explicitly annotated with `// skarn-v1-migration-compat` (e.g. `#[serde(alias = "selora")]`, `%APPDATA%\selora-ide` fallback reader, `selora-ide` keyring reader, `SELORA_*` env fallback, 11 `localStorage` key copier, `selora-dark` theme mapper)
