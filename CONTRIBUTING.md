# Contributing

## Setup

1. Fork and clone the repo
2. Install prerequisites (see [README.md](README.md#prerequisites))
3. Install dependencies:

```bash
npm install
```

4. Verify everything builds and passes:

```bash
cd src-tauri && cargo test
npm run check
```

## Workflow

### 1. Pick an issue

Browse [open issues](https://github.com/onemorepereira/training-app/issues). If you're working on something not yet tracked, open an issue first so the approach can be discussed.

### 2. Create a branch

Branch from `main` with a descriptive name:

```bash
git checkout main && git pull origin main
git checkout -b <type>/<issue-number>-<short-description>
```

Branch name conventions:
- `fix/` for bug fixes (e.g., `fix/3-shell-injection-prerequisites`)
- `feat/` for new features (e.g., `feat/42-session-export-csv`)
- `docs/` for documentation (e.g., `docs/contributing-guide`)
- `test/` for test-only changes (e.g., `test/28-sensor-data-roundtrip`)
- `refactor/` for refactors (e.g., `refactor/15-device-state-store`)

### 3. Make your changes

- Keep commits focused — one logical change per commit
- Follow existing code conventions (see [Development Conventions](#development-conventions) below)
- Write or update tests for any behavioral changes

### 4. Verify locally

```bash
cd src-tauri && cargo test          # All Rust tests must pass
npm run check                       # Frontend type checking must pass
```

### 5. Commit

Write a clear commit message:

```
<imperative summary of what changed>

<optional body explaining why, not what>

Closes #<issue-number>
```

Example:

```
Fix shell injection risk in prerequisites fix script

Replace build_fix_script (bash string interpolation) with
build_fix_commands (discrete argument vectors passed to pkexec
via execvp), eliminating shell injection from paths with special
characters.

Closes #3
```

### 6. Push and open a PR

```bash
git push -u origin <branch-name>
```

Then open a pull request against `main`. In the PR body:

- Summarize what changed and why (1-3 bullet points)
- Include a test plan (what to verify manually, if anything)
- Reference the issue with `Closes #N`

### 7. CI and review

- CI runs automatically on every PR (Rust tests + frontend type check)
- PRs require passing CI before merge
- All PRs are squash-merged to keep a clean linear history

## Development Conventions

### Rust

- Inline test modules: `#[cfg(test)] mod tests { use super::*; ... }`
- Every test must answer: **"What breaks if this test is deleted?"**
- Use hand-computed reference values in assertions, not just `is_some()`
- Prioritize edge cases over happy paths
- Use `assert_approx(actual, expected, epsilon, msg)` for float comparisons
- Epsilon guidance: watts +/-0.1, ratios +/-0.01, TSS +/-0.5
- Test one concept per test; name after behavior (e.g., `np_constant_power_equals_that_power`)

### Frontend (Svelte 5 / TypeScript)

- Use Svelte 5 runes (`$state`, `$derived`, `$effect`)
- Type all Tauri API calls — see `src/lib/tauri.ts`
- Handle Tauri command errors gracefully in the UI

### Project commands

```bash
cd src-tauri && cargo test                         # All Rust tests
cd src-tauri && cargo test --lib session::metrics   # Specific module
npm run check                                       # Frontend types
npm run dev                                         # Dev server
npm run build:release                               # Production build
```

## Releases

Releases are automated. When a version tag is pushed:

```bash
git tag v0.2.0
git push origin v0.2.0
```

The release workflow builds Linux packages (.deb, .AppImage, .rpm) and publishes them as a GitHub Release with auto-generated release notes.
