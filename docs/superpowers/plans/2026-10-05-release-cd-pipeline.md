# Release CD Pipeline + v0.9.0 Tag Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A GitHub Actions workflow that builds signed-off zips (jdiff + jptch) for Linux/macOS/Windows on every `v*` tag and publishes them as a GitHub release — then tag `v0.9.0` on main (which already carries the 0.9.0 idiomatic refactor, merged via PR #2) to trigger it.

**Architecture:** One new workflow `release.yml` with a 3-OS `build` matrix (mirrors `ci.yml`'s test matrix, plus `cargo build --release --target <triple>`) and a gated `release` job that merges the three zip artifacts, generates `SHA256SUMS.txt`, and publishes via `softprops/action-gh-release`. A `workflow_dispatch` trigger allows a smoke run before the first real tag; the release job is tag-gated so smoke runs never publish. No version bump and no merge happen in this plan: origin/main already carries the finished 0.9.0 idiomatic refactor (PR #2, merge `ba57d57`; local main fast-forwarded 2026-10-05) and its `Cargo.toml` already says `0.9.0` — the first release is `v0.9.0`, with the release body taken from the in-repo notes draft by convention.

**Tech Stack:** GitHub Actions (actions/checkout@v4, dtolnay/rust-toolchain@stable, actions/upload-artifact@v4, actions/download-artifact@v4, softprops/action-gh-release@v2), `zip`/`Compress-Archive`/`sha256sum` (all preinstalled on runners), `gh` CLI (authenticated as `ar-nim` on this machine).

**Spec:** User request, 2026-10-05 (inlined below — no prior spec doc exists; this plan is the spec of record):

> 1. every time a version is tagged, github actions will build a release for all 3 OS, jdiff and jptch duplicate binary (or in Linux, a symlink) in a zip
> 2. Tag the current code base to 0.9.0 so that the CD pipeline is triggered.
>
> **Amended 2026-10-05, twice (user follow-ups):** the version lineage is 0.8.5 = 1:1 port, 0.9.0 = idiomatic refactor with the Rust goodies — first amended to tag `v0.8.5` while 0.9.0 stayed reserved, then re-amended the same day: the idiomatic refactor is complete and already merged to `origin/main` (PR #2, merge `ba57d57` — local main was 66 commits behind until `git fetch --prune` + `git pull --ff-only`). `Cargo.toml` on main is already `0.9.0`, so the release tag is **`v0.9.0`**, exactly as the lineage intends.

## Global Constraints

- **Byte-contract (non-negotiable):** `src/defs.rs:100` `JDIFF_VERSION = "0.8.5 (beta) 2020"` stays exactly as is — it mirrors upstream's greeting (`jdiff -v`) and is pinned by the unit test at `src/defs.rs:296` (line numbers are post-refactor). The `Cargo.toml` `version` is the *packaging* version only. No `src/` file may be modified by this plan.
- **Version policy (user decision):** 0.8.x = port-parity lineage; **0.9.0 = the idiomatic refactor** (`docs/superpowers/specs/2026-10-03-idiomatic-rust-refactor-design.md`) — which is now complete and merged to main (PR #2). `Cargo.toml` there is already `version = "0.9.0"`, so this plan performs **no version bump and no merge**; it tags `v0.9.0` on main.
- **The workflow adds no new tooling** — it uses only preinstalled runner tools (`zip`, `Compress-Archive`, `sha256sum`) and the pinned actions listed above. The crate itself now depends on `thiserror`, `anyhow`, and `sysinfo` (the refactor's §13 dependency record in `Cargo.toml` `[dependencies]`); cargo fetches these normally during the test/build steps.
- **Tag format:** `v{Cargo.toml version}` (e.g. `v0.9.0`), annotated tags, matching the `v*` trigger filter.
- **Tag only a synced main:** the tag task guards with `git pull --ff-only origin main` first — local main sat 66 commits behind origin/main until the 2026-10-05 sync; never tag a stale main.
- **Platform coverage:** exactly 3 builds — `x86_64-unknown-linux-gnu` (ubuntu-latest), `aarch64-apple-darwin` (macos-latest, which is an arm64 runner), `x86_64-pc-windows-msvc` (windows-latest). Intel-Mac builds are an explicit non-goal (README points to source builds).
- **Zip contents contract:** Linux zip = `jdiff` + **relative symlink** `jptch → jdiff` (made with `ln -s jdiff` + `zip -y`); macOS and Windows zips = `jdiff`(/`.exe`) + **real duplicate copy** named `jptch`(/`.exe`). Files sit at the zip root, no wrapper folder.
- **Never commit** `.zcode/`, `.worktrees/`, `target/`, or `reference/` scratch. Scratch for verification steps goes under `target/` (gitignored), not `/tmp` (7.8G tmpfs; disk-quota cascade risk).
- **All shell commands run via `rtk`** (user AGENTS.md rule, applies to subagent sessions too).
- `gh` CLI is authenticated as `ar-nim` with HTTPS git protocol; `unzip` is at `/usr/sbin/unzip`. Local `zip` is NOT installed — never plan a local `zip` invocation; verify zips by downloading runner artifacts.

---

### Task 1: Author `.github/workflows/release.yml`

**Files:**
- Create: `.github/workflows/release.yml`

**Interfaces:**
- Consumes: nothing new.
- Produces: workflow `release.yml` (name field) usable by Task 2's `gh workflow run release.yml --ref main`; artifact names equal to the target triples (`x86_64-unknown-linux-gnu`, `aarch64-apple-darwin`, `x86_64-pc-windows-msvc`); zip filename pattern `jojodiff-cli-rs-${GITHUB_REF_NAME}-${target}.zip` (so on tag `v0.9.0`: `jojodiff-cli-rs-v0.9.0-<target>.zip`); release assets plus `SHA256SUMS.txt`; release body from `docs/superpowers/notes/<date>-<version>-release-notes.md` when one exists for the tagged version (generated notes appended).

Rationale for design choices (for the reviewer):
- `cargo test --all-features` runs before every release build — a release is never published from a red build. Note `ci.yml` no longer fires on tags (its push trigger is `branches: [main]` only), so this in-workflow test run is the **only** CI the tagged commit gets.
- The release body prefers the curated notes draft committed in-repo (located by the `docs/superpowers/notes/*-<version>-release-notes.md` convention at publish time); GitHub's generated notes append as a supplement/fallback.
- Explicit `--target ${{ matrix.target }}` makes the binary path uniformly `target/<triple>/release/` on all OSes and bakes the triple into artifact names.
- Windows uses `Compress-Archive` (no `zip` CLI on windows runners); `Compress-Archive -Path stage/*` stores `jdiff.exe`/`jptch.exe` at the zip root.
- The `release` job carries `permissions: contents: write` only (build jobs keep the default read token).
- `workflow_dispatch` + the `if: startsWith(github.ref, 'refs/tags/')` gate on `release` give a full end-to-end smoke path (build + package + artifacts) that can never publish.

- [ ] **Step 1: Write the workflow file**

Create `.github/workflows/release.yml` with exactly this content:

```yaml
name: release

on:
  push:
    tags:
      - 'v*'
  # Smoke-test path (Task 2): builds and packages zips on demand, but the
  # tag-gate on the release job means these runs never publish anything.
  workflow_dispatch:

env:
  CARGO_TERM_COLOR: always

jobs:
  build:
    name: build (${{ matrix.target }})
    strategy:
      fail-fast: false
      matrix:
        include:
          - os: ubuntu-latest
            target: x86_64-unknown-linux-gnu
          - os: windows-latest
            target: x86_64-pc-windows-msvc
          - os: macos-latest
            target: aarch64-apple-darwin
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - run: cargo test --all-features
      - run: cargo build --release --target ${{ matrix.target }}

      - name: Package zip (Linux — jptch as relative symlink)
        if: runner.os == 'Linux'
        run: |
          set -euo pipefail
          mkdir stage
          cp "target/${{ matrix.target }}/release/jdiff" stage/jdiff
          ln -s jdiff stage/jptch
          # -y stores jptch as a symlink instead of following it into a copy
          (cd stage && zip -y "../jojodiff-cli-rs-${GITHUB_REF_NAME}-${{ matrix.target }}.zip" jdiff jptch)

      - name: Package zip (macOS — jptch as duplicate copy)
        if: runner.os == 'macOS'
        run: |
          set -euo pipefail
          mkdir stage
          cp "target/${{ matrix.target }}/release/jdiff" stage/jdiff
          cp "target/${{ matrix.target }}/release/jdiff" stage/jptch
          (cd stage && zip "../jojodiff-cli-rs-${GITHUB_REF_NAME}-${{ matrix.target }}.zip" jdiff jptch)

      - name: Package zip (Windows — jptch as duplicate copy)
        if: runner.os == 'Windows'
        shell: pwsh
        run: |
          $ErrorActionPreference = 'Stop'
          New-Item -ItemType Directory -Force -Path stage | Out-Null
          Copy-Item "target/${{ matrix.target }}/release/jdiff.exe" stage/jdiff.exe
          Copy-Item "target/${{ matrix.target }}/release/jdiff.exe" stage/jptch.exe
          Compress-Archive -Path stage/* -DestinationPath "jojodiff-cli-rs-$env:GITHUB_REF_NAME-${{ matrix.target }}.zip"

      - name: Upload zip artifact
        uses: actions/upload-artifact@v4
        with:
          name: ${{ matrix.target }}
          path: jojodiff-cli-rs-*.zip
          if-no-files-found: error

  release:
    needs: build
    # Tag pushes only: workflow_dispatch smoke runs build and package but
    # must not create or publish a GitHub release.
    if: startsWith(github.ref, 'refs/tags/')
    runs-on: ubuntu-latest
    permissions:
      contents: write
    steps:
      - uses: actions/checkout@v4   # the curated release-notes draft (body_path) lives in-repo
      - name: Download zips from all three builds
        uses: actions/download-artifact@v4
        with:
          path: dist
          merge-multiple: true

      - name: Generate checksums
        run: cd dist && sha256sum *.zip > SHA256SUMS.txt

      - name: Locate curated release notes (optional, by convention)
        run: |
          ver="${GITHUB_REF_NAME#v}"
          notes=$(ls docs/superpowers/notes/*-"${ver}"-release-notes.md 2>/dev/null | head -n 1 || true)
          echo "BODY_PATH=${notes}" >> "$GITHUB_ENV"

      - name: Publish GitHub release
        uses: softprops/action-gh-release@v2
        with:
          body_path: ${{ env.BODY_PATH }}
          files: |
            dist/*.zip
            dist/SHA256SUMS.txt
          generate_release_notes: true
```

- [ ] **Step 2: Commit (workflow + this plan doc)**

```bash
rtk git add .github/workflows/release.yml docs/superpowers/plans/2026-10-05-release-cd-pipeline.md
rtk git commit -m "ci(release): add tag-triggered release pipeline (linux/mac/windows zips)"
```

(No local YAML validation is possible — no pyyaml, no actionlint, no local `zip`. The dispatch smoke run in Task 2 IS the syntax + behavior test; that is why Task 2 exists.)

---

### Task 2: Smoke-test the workflow via `workflow_dispatch`

**Files:**
- No file changes (operates on the committed workflow from Task 1).

**Interfaces:**
- Consumes: `release.yml` on branch `main` (workflow_dispatch only appears once the file is on the default branch).
- Produces: verified zips under `target/rel-smoke/` and green build jobs — proof the packaging contract holds before anything is published. Task 4 relies on this being green.

Note: on a dispatch run from `main`, `GITHUB_REF_NAME` is `main`, so smoke zips are named `jojodiff-cli-rs-main-<target>.zip`. That is expected; real names materialize on the tag.

- [ ] **Step 1: Push the workflow to main**

```bash
rtk git push origin main
```

Expected: fast-forward push of 1 commit (`ci(release): ...`).

- [ ] **Step 2: Trigger the smoke run and wait for it**

```bash
rtk gh workflow run release.yml --ref main
sleep 10
rtk gh run list --workflow=release.yml --limit 1
```

Expected: one run with event `workflow_dispatch`, status `in_progress` or `queued`. Then watch it (substitute the run id from the list output):

```bash
rtk gh run watch <run-id> --exit-status
```

Expected: exit 0; `build (x86_64-unknown-linux-gnu)`, `build (x86_64-pc-windows-msvc)`, `build (aarch64-apple-darwin)` all success; `release` job shown as **skipped** (tag gate).

If the workflow does not appear at all, the YAML failed to parse: re-read the file from Task 1 Step 1 against the Actions tab's "Invalid workflow file" annotation, fix, amend/commit, push, and re-dispatch.

- [ ] **Step 3: Download artifacts and verify the zip contract**

```bash
rtk gh run download <run-id> -n x86_64-unknown-linux-gnu -D target/rel-smoke/linux
rtk gh run download <run-id> -n aarch64-apple-darwin -D target/rel-smoke/macos
rtk gh run download <run-id> -n x86_64-pc-windows-msvc -D target/rel-smoke/windows
```

Then verify each zip:

```bash
# Linux: jptch must extract as a relative symlink to jdiff
rtk unzip -o target/rel-smoke/linux/jojodiff-cli-rs-main-x86_64-unknown-linux-gnu.zip -d target/rel-smoke/x-linux
test -L target/rel-smoke/x-linux/jptch && readlink target/rel-smoke/x-linux/jptch
```

Expected: prints `jdiff` (relative link target).

```bash
# Windows: two regular .exe entries, no symlink
rtk unzip -l target/rel-smoke/windows/jojodiff-cli-rs-main-x86_64-pc-windows-msvc.zip
```

Expected: entries `jdiff.exe` and `jptch.exe` at the zip root, roughly equal sizes (duplicate copy).

```bash
# macOS: two regular entries, roughly equal sizes
rtk unzip -l target/rel-smoke/macos/jojodiff-cli-rs-main-aarch64-apple-darwin.zip
```

Expected: entries `jdiff` and `jptch` at the zip root, roughly equal sizes.

- [ ] **Step 4: Confirm no release was published**

```bash
rtk gh release list
```

Expected: empty (or, after Task 5 later, still empty at this point).

No commit in this task.

---

### Task 3: Document prebuilt binaries in the README

**Files:**
- Modify: `README.md` (new section inserted immediately before the `## Building` heading at README.md:176 — line number re-verified post-refactor-merge)

**Interfaces:**
- Consumes: nothing from earlier tasks (independent of Tasks 1–2).
- Produces: README copy whose zip filenames match the workflow's output pattern `jojodiff-cli-rs-v<version>-<target>.zip` — the first release is `v0.9.0`.

Context for the implementer: there is deliberately **no version bump** in this plan — main already carries the finished 0.9.0 idiomatic refactor (merged via PR #2) with `Cargo.toml` at `0.9.0` (see Global Constraints: Version policy). The refactor already brought dependency/versioning documentation to the README; what is still missing is the prebuilt-binaries section added here. The README sentence below also explains the crate-version vs ported-engine distinction to users.

- [ ] **Step 1: Add the README section**

Insert the following immediately **before** the `## Building` heading (README.md:165), with one blank line on each side:

```markdown
## Prebuilt binaries

Each [`v*` tag](https://github.com/ar-nim/jojodiff-cli-rs/releases) is built by CI
into three zips, alongside a `SHA256SUMS.txt` digest list:

| Zip | Contents |
| --- | ------- |
| `jojodiff-cli-rs-v<version>-x86_64-unknown-linux-gnu.zip` | `jdiff` + relative symlink `jptch → jdiff` (extract with `unzip`, which preserves symlinks) |
| `jojodiff-cli-rs-v<version>-aarch64-apple-darwin.zip` | `jdiff` + duplicate copy `jptch` |
| `jojodiff-cli-rs-v<version>-x86_64-pc-windows-msvc.zip` | `jdiff.exe` + duplicate copy `jptch.exe` |

The crate version is the packaging version; the ported engine is JojoDiff
**0.8.5**, which is what the `jdiff -v` greeting reports. Intel-Mac and other
targets: build from source (below).
```

- [ ] **Step 2: Commit**

```bash
rtk git add README.md
rtk git commit -m "docs: document prebuilt release zips"
```

---

### Task 4: Tag `v0.9.0`, push, and verify the published release

**Files:**
- No working-tree changes (git tag + push + remote verification only).

**Interfaces:**
- Consumes: main synced with origin and already at `version = "0.9.0"` (the merged refactor — guards below assert both), green smoke run from Task 2, workflow from Task 1 (must already be pushed to `main` — Task 2 Step 1 did that).
- Produces: git tag `v0.9.0` on origin and GitHub release `v0.9.0` with 4 assets, body led by the curated notes draft.

This task performs outward-facing, hard-to-reverse actions (public tag + public release). It is explicitly requested by the user (trigger the CD pipeline by tagging the current codebase — `v0.9.0`, the merged idiomatic refactor), so proceed without further confirmation; if any guard below fails, STOP and report instead of pushing.

- [ ] **Step 1: Guards**

```bash
rtk git pull --ff-only origin main
rtk grep '^version = "0.9.0"' Cargo.toml
rtk git status --short
rtk git log --oneline -5
```

Expected: the pull reports `Already up to date.` (local main was 66 commits behind origin until the 2026-10-05 sync — this guard prevents ever tagging a stale main); the grep prints line 3; `git status --short` shows at most `?? .zcode/`; the log shows the Task 1 and Task 3 commits on top of the PR #2 merge (`ba57d57`).

- [ ] **Step 2: Push main, then create and push the annotated tag**

Push `main` first so the tagged commit exists on the remote branch before the tag's CI checks it out.

```bash
rtk git push origin main
rtk git tag -a v0.9.0 -m "jojodiff-cli-rs v0.9.0 (idiomatic-Rust refactor of the JojoDiff 0.8.5 port)"
rtk git push origin v0.9.0
```

Expected: both pushes succeed; the tag push immediately queues the `release` workflow. `ci.yml` does not run on tags (its push trigger is `branches: [main]` only) — the release workflow's own test step is the gate.

- [ ] **Step 3: Watch the release run to completion**

```bash
rtk gh run list --workflow=release.yml --limit 2
rtk gh run watch <release-run-id> --exit-status
```

Expected: exit 0 — three `build` jobs green and the `release` job executed (not skipped) this time.

- [ ] **Step 4: Verify the published release**

```bash
rtk gh release view v0.9.0 --json assets -q '.assets[].name'
```

Expected, exactly these 4 assets:

```
SHA256SUMS.txt
jojodiff-cli-rs-v0.9.0-aarch64-apple-darwin.zip
jojodiff-cli-rs-v0.9.0-x86_64-pc-windows-msvc.zip
jojodiff-cli-rs-v0.9.0-x86_64-unknown-linux-gnu.zip
```

- [ ] **Step 5: Verify the shipped Linux zip carries the symlink**

```bash
rtk gh release download v0.9.0 -D target/rel-verify   # fetches all 4 assets
(cd target/rel-verify && rtk sha256sum -c SHA256SUMS.txt)
rtk unzip -o target/rel-verify/jojodiff-cli-rs-v0.9.0-x86_64-unknown-linux-gnu.zip -d target/rel-verify/x
test -L target/rel-verify/x/jptch && readlink target/rel-verify/x/jptch
```

Expected: `sha256sum -c` prints `OK` for all three zips and exits 0; readlink prints `jdiff`; the extracted `jdiff` binary exists and is executable.

- [ ] **Step 6: Report**

Report the release URL (`gh release view v0.9.0 --json url -q .url`), the 4 asset names, and confirm the release body leads with the curated draft (`# jojodiff-cli-rs 0.9.0` from `docs/superpowers/notes/2026-10-03-0.9.0-release-notes.md`) with GitHub's generated notes appended.

---

## Notes / Non-goals

- **No Intel-Mac (`x86_64-apple-darwin`) build** — "all 3 OS" is interpreted as one build per OS; macos-latest is arm64. Adding a `macos-13` matrix row later is a one-line change if ever needed.
- **No code signing / notarization** — out of scope; artifacts are unsigned.
- **No draft-release stage** — the release publishes immediately on a green tag run (that is what "CD" means here). If a bad release ships, delete the release + tag, fix, re-tag.
- **First release ever** — the repo has no prior tags. The curated 0.9.0 draft leads the release body; GitHub's generated notes (every commit to date, since there is no previous tag) append below it.
- **Release-body convention** — future releases: commit `docs/superpowers/notes/<date>-<version>-release-notes.md` and the workflow picks it up by convention; without a draft the release falls back to generated notes alone.
