# Unlace report — cyclic-oversized

3 files · 3 symbols · 3 edges
Snapshot `fae78f3477658b94f34bcc2f4caf3fa1c4a3e3755a87101edb21e808b25a688c`
All findings and moves are suggestions. Applying them may break the build; use your own judgement.

## Structural findings

```text
Shared findings (1)
  - cycle [violation] `alpha.ts`/`beta.ts`/`gamma.ts` — 3-symbol cycle; unlace keeps a dependency
    cycle in one file; to split it, break `stepGamma` -> `stepAlpha` (w=1.0, exact)

anchored profile-specific findings (0)
  None.

greenfield profile-specific findings (0)
  None.
```

## Candidate layouts

```text
anchored — 0 candidates (baseline -0.0750; search converged; current layout already optimal)
greenfield — 0 candidates (baseline 0.1250; search converged; current layout already optimal)
```

## Advice

```text
  None.
```
