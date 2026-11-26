# Unlace report — test-heavy-ts

6 files · 9 symbols · 9 edges
Snapshot `0ae89d660327a975bb2eab84e51e4ab33f3028cd6175b9fda2cc72d69ddce4f4`
All findings and moves are suggestions. Applying them may break the build; use your own judgement.

## Structural findings

```text
Shared findings (4)
  - testOnly [violation] at `src/adapters/anthropic/client.ts`, `closeClient`: Only tests in this
    repo use `closeClient`. It is public, so code outside this repo may use it too; check before
    moving or deleting it.
  - testOnly [violation] at `src/adapters/anthropic/client.ts`, `createClient`: Only tests in this
    repo use `createClient`. It is public, so code outside this repo may use it too; check before
    moving or deleting it.
  - testOnly [violation] at `src/adapters/anthropic/client.ts`, `sendMessage`: Only tests in this
    repo use `sendMessage`. It is public, so code outside this repo may use it too; check before
    moving or deleting it.
  - testOnly [violation] at `src/adapters/anthropic/models.ts`, `modelContext`: Only tests in this
    repo use `modelContext`. It is public, so code outside this repo may use it too; check before
    moving or deleting it.

anchored profile-specific findings (0)
  None.

greenfield profile-specific findings (0)
  None.
```

## Candidate layouts

```text
anchored — 0 candidates (baseline 0.0770; search converged; current layout already optimal)
greenfield — 0 candidates (baseline 0.2770; search converged; current layout already optimal)
```

## Advice

```text
  None.
```
