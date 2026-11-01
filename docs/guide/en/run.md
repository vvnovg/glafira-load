# Run

The run window starts the load by a profile on the agents of a stand, shows the live progress and the summary and
lists the past runs. Open it with [[ui:sidebar.run]] in the left panel. The same run is available from the terminal as
`glafira-load run`.

## Form {#form}

Choose the [[ui:run.suite]], the [[ui:run.profile]] ([Load profile](profile.md)), the [[ui:run.env]] with its agents
([Agents and stands](agents.md#stand)) and the [[ui:run.mode]]: [[ui:run.modes.run]] executes the timeline of the
profile, [[ui:run.modes.smoke]] runs every scenario once ([Smoke](run.md#smoke)) and needs no profile.
[[ui:run.maxInFlight]] limits the number of iterations in flight on one agent; empty means the value of the suite (1000
by default). The rows of the agents show their state ([Checking the agents](agents.md#probe)). For a stand without
agents the window says that the run goes on the agent built into the application.

Before starting, the window runs the check of the suite ([Check](suites.md#check)) and does not start with errors: the
report is shown with [[ui:run.recheck]]. [[ui:run.start]] is also unavailable while another run of the project is going
on. The fields are locked during a run; when the tab is opened again during a run, they show its parameters.

## Live progress {#live}

During the run the window shows the current phase, the elapsed minutes and the [[ui:run.progress]], two charts, the
counters of requests, errors and dropped iterations, the operations table and the recent error samples.
[[ui:run.chart.rps]] draws the target and the actual total requests per second, [[ui:run.chart.p95]] the p95 latency of
up to five operations you tick. The [[ui:run.ops.title]] table has a row per operation with the values of the last
second: requests, p95, [[ui:run.ops.errors]] and [[ui:run.ops.dropped]]; the columns sort by click. The rows of the
agents show [[ui:run.agentState.running]], [[ui:run.agentState.lost]] or [[ui:run.agentState.done]].

Dropped iterations mean the agent reached [[ui:run.maxInFlight]]: the stand does not keep up with the profile, and the
agent did not start new iterations instead of queueing them. Leaving the tab or closing it does not interrupt the run.
Closing the application during a run asks whether to stop the run or leave it to the agents
([When an agent is lost](agents.md#loss)).

## Stop {#stop}

[[ui:run.stop]] asks every agent to stop: new iterations are not started, the running ones finish within `--drain-ms`
of the agent. The files are collected, the summary is computed, and the status of the result is
[[ui:run.runStatus.stopped]]. In the terminal `Ctrl+C` does the same, for a local run and for `--agents`.

## Summary and files {#files}

When the run ends, the [[ui:run.summary.title]] table shows every operation: requests, average and peak RPS, the share
of errors, dropped, p50, p95, p99, max, the target from the profile and the achieved percentage; the columns sort by
click. [[ui:run.summary.openFolder]] opens the folder of the run. The statuses: [[ui:run.runStatus.completed]],
[[ui:run.runStatus.stopped]], [[ui:run.runStatus.incomplete]] (an agent was lost or its files were not collected) and
[[ui:run.runStatus.failed]]. The achieved percentage is the average RPS over the profile time (the drain seconds after
its end do not count) divided by the target, so a ramp phase lowers it; compare the steady phase in `metrics.jsonl` when
you need the exact figure.

The folder is `runs/<date-time>/` in the project (UTC, for example `runs/2026-11-02T14-30-00/`), kept out of git by
the `.gitignore` that Glafira writes when it creates a project:

| File | Contents |
|---|---|
| `run.yaml` | suite, profile, stand, mode, agents with addresses, weights and versions, the commit of the project, the versions of the services, start, end, status |
| `metrics.jsonl` | the merged per-second series: a line per operation with `t`, `op`, `n`, `err` (by code), `dropped`, `p50`, `p90`, `p95`, `p99`, `max`, `bytes_in`, `bytes_out` and `lag` for `kafka_consume`, and a line per scenario with `t`, `scenario`, `n`, `err`, `dropped`, `p50`, `p95`, `p99`, `max` |
| `phases.yaml` | the phases with their actual start (`t` in seconds) and the target percentage |
| `errors.jsonl` | samples of errors, at most 5 per code and operation: code, operation, scenario, step, agent, time, message, request and response (truncated, secrets masked); the file is absent when there were no errors |
| `summary.yaml` | the summary: per operation also p90 and errors by code, per scenario requests, errors and percentiles |
| `agents/<id>/` | the raw files of every agent: `metrics.jsonl`, `errors.jsonl`, `summary.yaml`, `hist.bin`, and from an agent on a load machine also `agent.log` and `run.json` |

Percentiles of the summary are computed from the merged HDR histograms of the agents (`hist.bin`), not averaged.
`summary.yaml` and `run.yaml` are sorted and stable. The `runs/` folder is git-ignored in a new project, so to keep a
result in git add its files with `git add -f runs/<folder>/summary.yaml runs/<folder>/run.yaml`, or copy the folder out.

[[ui:run.past.title]] lists the folders of `runs/` with their start, suite, profile, stand, mode and status; choosing
one shows its summary. A run without an end is marked [[ui:run.past.inProgress]] and is completed from the agents when
the list is opened ([When an agent is lost](agents.md#loss)).

## Smoke {#smoke}

[[ui:run.modes.smoke]] (or `glafira-load smoke`) executes every scenario of the suite exactly once, one after another,
against the stand and lists the steps with their result: `ok` or the code and the reason of the error. It runs inside
the application or the CLI, not on the agents, and needs no profile; stand secrets therefore come from the environment
of the application or the shell ([Stand secrets](agents.md#secrets)). There are no live charts in this mode. The result
is `runs/smoke-<date-time>/` with `run.yaml`, `smoke.yaml` (scenario → step → `ok` or the error code), `summary.yaml`,
`errors.jsonl` when a step failed, and `agents/local/`. Run it after editing a suite, before a long run.

## Run in the terminal {#cli}

```bash
glafira-load check <project> [--suite <name>]
glafira-load smoke <project> --suite <name> --env <stand> [--out <dir>]
glafira-load run   <project> --profile <name> --env <stand> [--suite <name>] [--agents] [--max-in-flight N] [--out <dir>]
```

`run` without `--agents` executes the load in its own process with the same engine: for CI, for a single machine and
for trying a suite. `--agents` uses the agents of the stand with the tokens from `GLAFIRA_LOAD_AGENT_TOKEN_<ID>`
([Token](agents.md#token)). `--suite` is needed when the project has more than one suite. Stand secrets of a local run
and of `smoke` come from `GLAFIRA_LOAD_SECRET_<NAME>` in the shell. The console prints a line per second with the
total RPS of all operations, p95, errors and dropped iterations (`t=12s rps=598 p95=1ms err=0 dropped=0`), the changes
of the agent states with `--agents`, and the summary table and the status at the end. Install the CLI as described in
[Installing an agent](agents.md#install); `glafira-load --version` prints its version.

## Exit codes {#exit-codes}

| Code | Meaning |
|---|---|
| 0 | completed, also with the status `incomplete` |
| 2 | stopped (`Ctrl+C`) |
| 3 | start error: wrong arguments, the project cannot be read, `SECRET_MISSING`, `TOKEN_MISSING`, an agent unreachable, busy or of another version, the check of the suite found errors (`run`); or the run failed |
| 4 | `check` found errors, or `smoke` found broken steps or errors of the check |
