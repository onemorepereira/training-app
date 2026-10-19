# Development Conventions

## Test Philosophy

Every test must answer: **"What breaks if this test is deleted?"** Tests verify real behavior, not existence.

## Rules

1. **Hand-computed reference values** — Algorithm tests assert exact expected outputs (e.g., `assert_approx(np, 200.0, 0.1)`), never just `is_some()`
2. **Edge cases over happy paths** — Prioritize boundaries, division by zero, empty state, off-by-one
3. **No trivial getter/setter tests** — Test getters as part of a real workflow, not in isolation
4. **Test the contract** — Public API inputs/outputs only; don't reach into struct internals
5. **One concept per test** — Name after behavior: `np_constant_power_equals_that_power`, not `test_np_1`

## Rust Conventions

- Inline test modules: `#[cfg(test)] mod tests { use super::*; ... }`
- Use `assert_approx(actual, expected, epsilon, msg)` helper for float comparisons with explicit epsilon
- Epsilon guidance: watts ±0.1, ratios ±0.01, TSS ±0.5

## Project Commands

```bash
cd src-tauri && cargo test          # Run all Rust tests
cd src-tauri && cargo test --lib session::metrics  # Run metrics tests only
npm run check                       # Frontend type checking
```

## Schema Changes

- Schema lives only in `src-tauri/migrations/000N_*.sql`, applied by `session/storage/schema.rs` (tracked in `PRAGMA user_version`)
- New change = new numbered file + entry in `MIGRATIONS` + its SHA-256 in `PINNED_SHA256`; never edit a shipped file
- Additive only (no DROP/RENAME/NOT NULL without DEFAULT); never depend on column order
- Every step must pass against all fixtures in `session/storage/testdata/` (`cargo test --lib storage::schema`)
