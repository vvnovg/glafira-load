# Load profile

The load profile turns the production figures into targets for the test stand: how many requests per second each
operation gets and how the load changes over time. It is kept in `load/profile.yaml`; the runner of the next stage
executes it as is. Open the tab with [[ui:sidebar.profile]] in the left panel.

## Coefficients {#coefficients}

The target of an operation is

```
target RPS = production figure × k_env × k_growth
```

- `k_env` — the capacity of the test stand relative to production: `0.5` for a stand half as large;
- `k_growth` — the growth margin: `1.3` to test 30 % above today's load.

Both are `1` by default and cannot be negative. Set them in [[ui:profile.coefficients]]. When a service or an operation
needs other values, add a row to [[ui:profile.serviceOverrides]] (by service ID) or [[ui:profile.operationOverrides]]
(by operation ID) and fill in only the coefficient that differs. An operation override wins over a service one, and a
service one over the global value.

Changes are kept as a draft until you click [[ui:common.save]]; [[ui:project.revert]] returns to the saved file.

## Profiles and phases {#phases}

A project has several named profiles. A new project has three:

| Profile | Basis | Phases |
|---|---|---|
| `nominal` | average | ramp to 100 % in 10 min, steady 60 min, ramp down to 0 % in 5 min |
| `peak` | peak | ramp to 100 % in 5 min, steady 30 min |
| `stress` | peak | steps of 25 % for 10 min each, up to 300 % |

Add, rename and remove profiles in [[ui:profile.profiles]]. The [[ui:profile.basis]] chooses which production figure is
100 %: [[ui:profile.basisAvg]] or [[ui:profile.basisPeak]]. A profile is a list of phases, run one after another:

- **ramp** — goes linearly from the current level to a percentage in the given minutes;
- **steady** — holds the current level for the given minutes;
- **steps** — rises in steps: one step, two steps and so on up to the maximum, each step held for the given minutes.

Add phases with [[ui:profile.addPhase]] and reorder them with [[ui:profile.moveUp]] and [[ui:profile.moveDown]].

## Targets and shares {#targets}

After saving, the tab shows the targets of the chosen profile: for every operation with production figures, the basis,
the coefficients that apply, the target RPS and its share of the total. The heading shows the total RPS and the
duration of the profile. The targets are computed from the saved file; while there are unsaved changes, the tab says so.

Active operations without production figures are listed in "not covered": they get no load until you import their
figures ([Production load](prod-load.md)). Removed operations do not take part.

## Chart {#chart}

The chart shows the profile over time: minutes on the horizontal axis, the percentage of the target on the left axis
and the total RPS on the right axis. Vertical lines mark where each phase begins.

## File load/profile.yaml {#file}

```yaml
format: 1
coefficients:
  k_env: 0.5
  k_growth: 1.3
  overrides:
    services:
      billing: { k_env: 0.3 }
    operations:
      "http:orders:POST /v1/orders": { k_growth: 2.0 }
profiles:
  nominal:
    basis: avg
    phases:
      - { kind: ramp, minutes: 10, to_pct: 100 }
      - { kind: steady, minutes: 60 }
      - { kind: ramp, minutes: 5, to_pct: 0 }
  stress:
    basis: peak
    phases:
      - { kind: steps, step_pct: 25, step_minutes: 10, max_pct: 300 }
```

The file is meant to be read in `git diff` and can be edited by hand; the tab shows the changes after you reopen it.
