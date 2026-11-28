# Unlace report — cyclic-oversized

3 files · 3 symbols · 3 edges
Snapshot `fae78f3477658b94f34bcc2f4caf3fa1c4a3e3755a87101edb21e808b25a688c`
All findings and moves are suggestions. Applying them may break the build; use your own judgement.

## Structural findings

```text
Shared findings (1)
  - cycle [borderline] `alpha.ts`/`beta.ts`/`gamma.ts` — 3-file import cycle; to break it, remove
    `./beta` (line 1) from `alpha.ts`, which carries 1 symbol reference (`stepAlpha` ->
    `stepBeta`) (exact); no import in it runs at load time

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
