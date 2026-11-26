# Unlace report — cyclic

2 files · 2 symbols · 2 edges
Snapshot `cd39e5cfbb68018083053c2d961061c067529adcfefb1268843964f54d460977`
All findings and moves are suggestions. Applying them may break the build; use your own judgement.

## Structural findings

```text
Shared findings (1)
  - cycle [violation] `alpha.py`/`beta.py` — 2-symbol cycle; unlace keeps a dependency cycle in
    one file; to split it, break `beta_step` -> `alpha_step` (w=1.0, exact)

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
