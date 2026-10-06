# CI benchmarks

`.github/workflows/benchmarks.yml` measures the Criterion microbenchmarks on a
GitHub-hosted runner and compares every run with the last release and the last
`main` run.

| Trigger | What happens | Published? |
|---|---|---|
| Merge to `main` | Benchmarks run, the *CI benchmarks* block in [`BENCHMARKS.md`](../BENCHMARKS.md) is rewritten (a `docs(bench)` commit by `github-actions[bot]`), the run is stored as `bench/latest.json` in the same commit | yes — `BENCHMARKS.md` + website "next release" preview |
| Release | `release.yml` calls the workflow in its own job after the GitHub Release exists, so PPA and crates.io publishing never wait for it. Results are attached to the release as `bench-results-<tag>.json` and `BENCHMARKS-<tag>.md`, appended to the release notes, and recorded as `bench/releases/<tag>.json` on `main` | yes — on the release |
| *Actions → Benchmarks → Run workflow* | Benchmarks run and the comparison appears in the **job summary** (and log). Nothing is committed, uploaded to a release, or recorded under `bench/` | no |
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

Results are plain files on `main` (committed by
[`scripts/push-bench-data.sh`](../scripts/push-bench-data.sh)):

```
bench/latest.json            newest run on main, overwritten on every merge
bench/releases/<tag>.json    one file per release, never rewritten
```

openrtmp.org reads `latest.json` to show the newest CI run next to its release snapshots.

[`scripts/bench_report.py`](../scripts/bench_report.py) collects Criterion
output, fetches the baselines, renders the Markdown and splices it into
`BENCHMARKS.md` / the release notes. It uses only the Python standard library.
To try it locally:

```bash
cargo bench --bench protocol -- --noplot && mv target/criterion crit-protocol
python3 scripts/bench_report.py collect --suite protocol --kind manual   # reads crit-protocol/, writes results.json
python3 scripts/bench_report.py compare                                  # reads results.json, writes comparison.md
cat comparison.md
```

## Notes

- The release tag is **not** moved after the benchmark finished: the PPA
  upload and the crates.io package are built from the tagged commit, and
  re-pointing a published tag would make the tag disagree with them. The
  release numbers are therefore attached to the GitHub Release (and recorded under
  `bench/releases/` on `main`) instead of living in the tagged source tree.
- Pushing the `BENCHMARKS.md` and `bench/` updates to `main` needs `contents: write` and a
  `main` that accepts pushes from `github-actions[bot]`. If branch protection
  blocks it, the job prints a warning and the numbers are still in the job
  summary.
- `paths-ignore: BENCHMARKS.md, bench/**` keeps the bot's own commit from retriggering
  the workflow.
