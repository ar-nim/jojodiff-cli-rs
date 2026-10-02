# Provenance of the vendored C++ reference trees

## `reference/jojodiff-cpp/` — JojoDiff v0.8.1

Snapshot of https://github.com/vibhorkalley/jojodiff (the C++ class rewrite of JojoDiff
v0.8.1 by Joris Heirbaut), cloned 2026-10-01 with `.git` removed. Layout: `headers/` +
`src/` split, `src/jpatch.cpp` standalone patcher, `tests/` fixtures. This is the source
of truth for the original 0.8.1 port (spec §1, plan Tasks 1–12, shipped as package
version 0.8.1).

## `reference/jojodiff-0.8.5/` — JojoDiff v0.8.5

Snapshot of the author's upstream repository at commit **66a2806** ("085ac tuning
internal parameters", 2020-10-29 — the state matching the SourceForge 0.8.5 release
files dated 2020-10-29), exported via `git archive` from
https://sourceforge.net/p/jojodiff/code/ci/master/tree/
(`git clone https://git.code.sf.net/p/jojodiff/code`) on 2026-10-02, `.git` excluded,
upstream file bytes preserved (CRLF line endings, Eclipse `.cproject`/`.project`
included). Layout: flat `src/` with headers next to sources, `tst/` shell harness, no
test fixtures, no standalone patcher (`jpatch.cpp` was removed upstream; patching is
`jdiff -u` / `JPatcht` — see spec Part II §18.B).

This is the source of truth for the 0.8.5 re-target (spec Part II, plan Tasks 13+).

Upstream history between the two trees (16 commits, `git log --oneline`):
`333cfba/cdc1931 v0.8.1` → `0087d15/ba33334 v0.8.2` (jfopen/LARGEFILE wrappers) →
`63b37be` (virtual destructors) → `5554402 083c` → `137ff46 v083d` → `1329591 083j` →
`c5480f0 083m` → `75e4450/1b7bd85 083r` (dynamic matching table) → `51e50fb/3691a2b
083s` (stdin + `-s`) → `3a046ce 083t` (JPatcht integration) → `87735de/9b6052a 083v`
(sequential files) → `a1e87f4 084b` → `0eaf1cf/6a56732 084c` → `63a2691 085aw` →
`e258d78 085bl` → `66a2806 085ac` (HEAD).

The full verified change analysis backing the re-target lives at
`docs/superpowers/research/2026-10-02-jojodiff-0.8.5-analysis.md`.
