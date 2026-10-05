# CI benchmarks

`.github/workflows/benchmarks.yml` measures the Criterion microbenchmarks on a
GitHub-hosted runner and compares every run with the last release and the last
`main` run.

| Trigger | What happens | Published? |
|---|---|---|
| Merge to `main` | Benchmarks run, the *CI benchmarks* block in [`BENCHMARKS.md`](../BENCHMARKS.md) is rewritten (a `docs(bench)` commit by `github-actions[bot]`), the run is stored as `latest.json` on the `bench-data` branch | yes — `BENCHMARKS.md` + website "next release" preview |
| Release | `release.yml` calls the workflow in its own job after the GitHub Release exists, so PPA and crates.io publishing never wait for it. Results are attached to the release as `bench-results-<tag>.json` and `BENCHMARKS-<tag>.md`, appended to the release notes, and stored as `releases/<tag>.json` on `bench-data` | yes — on the release |
| *Actions → Benchmarks → Run workflow* | Benchmarks run and the comparison appears in the **job summary** (and log). Nothing is committed, uploaded to a release, or written to `bench-data` | no |
| Pull request | Same as the manual run | no |

## Reading the comparison

Each table has one column per baseline (*vs release vX*, *vs previous run*):

- 🟢 faster, 🔴 slower, ⚪ within the noise band. Lower time is better.
- The noise band is 5 % when both runs used the same CPU model, vCPU count,
  rustc and runner image, 10 % when the toolchain or image changed, and 25 %
  when the hardware differs, **plus** both runs' Criterion confidence
  intervals.
- Above the tables, a *Comparable* line states which of those cases applies.
  A GitHub-hosted runner is a shared VM whose CPU model and neighbours change
  between runs; treat absolute numbers as indicative and look for consistent
  changes across several runs.

Every result file records the CPU model, vCPU count, RAM, kernel, rustc and
runner image of the machine that produced it.

## Data

Results live on the orphan branch `bench-data` (written by
[`scripts/publish-bench-data.sh`](../scripts/publish-bench-data.sh)):

```
latest.json            newest run on main, overwritten on every merge
releases/<tag>.json    one file per release, never rewritten
```

[`scripts/bench_report.py`](../scripts/bench_report.py) collects Criterion
output, fetches the baselines, renders the Markdown and splices it into
`BENCHMARKS.md` / the release notes. It uses only the Python standard library.
To try it locally:

```bash
cargo bench --bench protocol -- --noplot && mv target/criterion crit-protocol
python3 scripts/bench_report.py collect --suite protocol=crit-protocol --kind manual --out results.json
python3 scripts/bench_report.py compare --current results.json
```

## Notes

- The release tag is **not** moved after the benchmark finished: the PPA
  upload and the crates.io package are built from the tagged commit, and
  re-pointing a published tag would make the tag disagree with them. The
  release numbers are therefore attached to the GitHub Release (and stored on
  `bench-data`) instead of living in the tagged source tree.
- Pushing the `BENCHMARKS.md` update to `main` needs `contents: write` and a
  `main` that accepts pushes from `github-actions[bot]`. If branch protection
  blocks it, the job prints a warning and the numbers are still in the job
  summary and on `bench-data`.
- `paths-ignore: BENCHMARKS.md` keeps the bot's own commit from retriggering
  the workflow.
