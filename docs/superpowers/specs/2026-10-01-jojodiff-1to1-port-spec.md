# JojoDiff → Rust 1:1 Port — Functional Specification

**Source of truth:** https://github.com/vibhorkalley/jojodiff (C++ rewrite of JojoDiff v0.8.1 by
Joris Heirbaut). Every `headers/*.h` and `src/*.cpp` file was read in full while writing this
spec. C++ references below use `path:line` into that tree; the port vendors the tree at
`reference/jojodiff-cpp/` so line numbers stay valid.

**Deliverable:** package **`jojodiff-cli-rs`** (library target `jojodiff_cli_rs`) producing
binaries **`jdiff`** and **`jptch`** plus a reusable
library, building and passing tests on Windows, Linux and macOS (std-only, zero runtime
dependencies). Naming rationale: encodes brand (`jojodiff`), form (`-cli` standalone tools, unlike
the apply-only library) and language (`-rs` port, unlike the C++ original) while conflicting with
neither the `jojodiff` crate (francisdb) nor its `jojodiff-rs` GitHub repo — verified free on
crates.io 2026-10-01. Non-affiliation with the francisdb project must be stated in the crates.io
description, repo About box, and README.

**Verification oracle:** the C++ tree built with default `make` (serial, `_FILE_OFFSET_BITS=64`,
no `_DEBUG`, no `-fopenmp`) **plus the 2-line Linux fix** described in §15.1. That fixed build
round-trips both bundled test pairs; byte-comparing Rust vs. fixed-C++ patches across the option
matrix of §16 is the acceptance gate.

**Prior art:** https://github.com/francisdb/jojodiff-rs (MIT, library-only, apply-side only,
known edge-case divergences). Decision (user, 2026-10-01): **fresh port**; the francisdb crate is
used only as an optional cross-validation consumer in tests (§16.4).

---

## 1. Inventory of the C++ tree (what must be ported)

| C++ file | LoC | Contents | Rust module (task) |
|---|---|---|---|
| `headers/JDefs.h` | 134 | constants, opcodes, exit codes, off_t/hkey plumbing | `src/defs.rs` (T1) |
| `src/main.cpp` | 614 | `jdiff` CLI: options, greeting/help, file open, stats, exit codes | `src/bin/jdiff.rs` (T9) |
| `src/jpatch.cpp` | 426 | `jptch` CLI + patch decoder `jpatch()` + `ufGetInt()` | `src/bin/jptch.rs` (T10) |
| `src/JDiff.cpp` + `.h` | 584+267 | diff engine: `jdiff()`, `ufFndAhd()`, `ufFndAhdGet()`, `ufFndAhdScn()`, `ufPutEql()` | `src/jdiff.rs` (T8) |
| `src/JHashPos.cpp` + `.h` | 224+175 | hash table: `hash()`, `add()`, `get()`, `print()`, `dist()`, prime selection | `src/jhashpos.rs` (T4) |
| `src/JMatchTable.cpp` + `.h` | 462+137 | match table: `add()`, `get()`, `cleanup()`, `check()` | `src/jmatchtable.rs` (T5) |
| `src/JFileAhead.cpp` + `.h` | 347+107 | buffered look-ahead reader (MinGW path; canonical semantics) | `src/jfile/ahead.rs` (T3) |
| `src/JFileIStream.cpp` + `.h` | 60+75 | unbuffered in-memory reader (`-m 0`) | `src/jfile/mem.rs` (T2) |
| `src/JFileIStreamAhead.cpp` + `.h` | 346+108 | istream clone of JFileAhead (identical algorithm) | covered by T3 (one impl) |
| `src/JOutBin.cpp` + `.h` | 226+64 | binary patch writer incl. length codec + ESC escaping | `src/jout/bin.rs` (T6) |
| `src/JOutAsc.cpp` + `.h` | 127+52 | `-l` byte-by-byte ASCII listing | `src/jout/asc.rs` (T7) |
| `src/JOutRgn.cpp` + `.h` | 97+53 | `-lr` region listing | `src/jout/rgn.rs` (T7) |
| `src/JOut.h` | 71 | abstract output + 6 stats counters | `src/jout/mod.rs` (T6) |
| `src/JFile.h` | 57 | abstract `get(pos,typ)`/`seekcount()` | `src/jfile/mod.rs` (T2) |
| `src/JDebug.cpp` + `.h` | 31+64 | debug flags/destination (`-do`, `-dhsh`…`-ddst`) | `src/jdebug.rs` (T1+T11) |
| `Makefile` | 61 | targets `all`, `debug`(`-D_DEBUG`), `runtest`, `gentest` | `scripts/runtest.sh`, CI (T12) |
| `tests/*.{fil,txt}` | 4 files | two golden file pairs (see §16) | `tests/fixtures/` (T12) |
| `run.sh`, `generate_testFile.sh` | 14 | test helpers | `scripts/` (T12) |
| `COPYING` | — | GPLv3 | `LICENSE` (T1) |
| `readme.htm`, `README.md` | — | user docs (no extra behavior) | `README.md` (T12) |

Nothing else exists in the tree (`bin/.empty`, `tests/.empty`, `.gitignore` are placeholders).
`JSYNC` mentioned in docs is *not* in this repo — out of scope.

## 2. Type and constant mapping (global)

| C++ | Rust | Notes |
|---|---|---|
| `off_t` (64-bit) | `i64` | `JDefs.h:44`; MAX_OFF_T = `i64::MAX` |
| `hkey` = `unsigned long` (32-bit build) | `u32` with wrapping ops | `_LARGESAMPLE` is **not** set by the Makefile |
| byte-or-sentinel `int` | `i32` | byte `0..=255`, `EOF = -1`, `EOB = EOF-1 = -2` (`JDefs.h:110`) |
| `SMPSZE` | `32` | `sizeof(hkey)*8` in C++; literal in Rust (matches oracle build) |
| `MCH_PME` / `MCH_MAX` | `127` / `256` | `JMatchTable.h:29-30` |
| opcodes | `ESC=0xA7, MOD=0xA6, INS=0xA5, DEL=0xA4, EQL=0xA3, BKT=0xA2` | `JDefs.h:127-132` |
| exit codes | `EXI_DIF=0, EXI_EQL=1, EXI_ARG=2, EXI_FRT=3, EXI_SCD=4, EXI_OUT=5, EXI_SEK=6, EXI_LRG=7, EXI_RED=8, EXI_WRI=9, EXI_MEM=10, EXI_ERR=20` | `JDefs.h:111-122` |
| version strings | `"0.8.1 (beta) December 2011"` / `"Copyright (C) 2002-2005,2009,2011 Joris Heirbaut"` | `JDefs.h:93-94`, byte-exact |
| `atoi()` | helper `c_atoi(&str) -> i32` | C semantics: leading whitespace skipped, optional sign, leading digits, garbage-tail ignored, overflow/none → returns parsed prefix (0 if none). **Must** be replicated because `-m abc` ⇒ 0 ⇒ in-memory mode. |
| `P8zd` (release build) | `{:>12}` for i64 | `%12lld` (`JDefs.h:84`). Debug builds use `{:>10}` (`%10lld`) — see §14. |

All jdiff engine arithmetic that can wrap (the `h*2+byte` hash) uses `wrapping_mul`/`wrapping_add`
on `u32`. Offsets use checked-free `i64` exactly like C (UB-free by construction on 64-bit).

## 3. Patch file format (wire format — must remain byte-compatible)

Stream = sequence of records; all multi-byte lengths big-endian (`JOutBin.cpp:32-63`,
`jpatch.cpp:107-117`):

```
<ESC><opcode>[<length>|<data>...]
```

* Data runs (MOD/INS) last until the next `<ESC><opcode>`; a data byte equal to `ESC` is followed
  by another `ESC` iff the next byte is in `BKT..ESC` (0xA2–0xA7) — see §11.1 escaping rules.
* Length encoding (`JOutBin::ufPutLen`, `JOutBin.cpp:64-103`):

| value range | encoding | bytes |
|---|---|---|
| 1..=252 | `len-1` | 1 |
| 253..=508 | `252, len-253` | 2 |
| 509..=65535 | `253, hi, lo` | 3 |
| 65536..=0xFFFFFFFF | `254, b32..b0` (4 BE) | 5 |
| > 0xFFFFFFFF | `255, b63..b0` (8 BE) | 9 |

`jptch` decoder `ufGetInt` (`jpatch.cpp:70-105`) mirrors this; the 9-byte form is always enabled
(oracle build defines `JDIFF_LARGEFILE`). An empty patch (0 bytes) means "files identical".

## 4. `jdiff` CLI specification (`main.cpp`)

Invocation: `jdiff [options] <original> <new> [<output>]`; missing output or `"-"` ⇒ stdout.

### 4.1 Option parsing loop (`main.cpp:205-315`) — exact rules

Options are scanned left-to-right and **must precede** the first non-option argument; the first
non-option token ends option parsing and is re-queued as the first filename (`main.cpp:311-314`).
An option that expects a value consumes the next token only if one remains
(`if (aiArgCnt > liOptArgCnt)`), silently keeping the default otherwise.

| Option | Effect (exact) | line |
|---|---|---|
| `-v` / `-vv` / `-vvv` | verbose level 1 / 2 / 3 | 207-211 |
| `-h` | help flag | 213 |
| `-a size` | `liAhdMax = atoi/2*1024` (int division **then** ×1024; `-a 1` ⇒ 0) | 216-220 |
| `-m size` | `llBufSze = atoi/2*1024` (i64; 0 ⇒ whole-file in-memory mode) | 221-225 |
| `-bs size` | `liBlkSze = atoi` | 226-230 |
| `-s size` | `liHshMbt = atoi; while (>1024) /=1024` | 231-236 |
| `-min count` | `liMchMin = atoi`, clamp to `MCH_MAX`(256) | 237-243 |
| `-max count` | `liMchMax = atoi`, clamp to 256 | 244-250 |
| `-l` | output type 1 (JOutAsc) | 252 |
| `-lr` | output type 2 (JOutRgn) | 254 |
| `-do` | debug/verbose stream = stdout | 283-284 |
| `-b` | preset: cmpAll=true, bufSze=4096Ki, srcBkt=true, srcScn=1, mchMin=16, mchMax=128, hshMbt=32 | 256-264 |
| `-f` | preset: cmpAll=false, bufSze=64Ki, srcBkt=true, srcScn=1, mchMin=8, mchMax=16, hshMbt=4 | 265-273 |
| `-ff` | preset: cmpAll=false, bufSze=4096Ki, srcBkt=true, srcScn=0, mchMin=4, mchMax=16, hshMbt=1 | 274-282 |
| `-dhsh -dahd -dcmp -dprg -dbuf -dhsk -dahh -dbkt -dred -dmch -ddst` | debug flags — only in debug-feature builds (§14) | 286-309 |

Defaults before options (`main.cpp:190-200`): outTyp=0 (JOutBin), verbose=0, srcBkt=true,
cmpAll=true, srcScn=1, mchMax=32, mchMin=8, hshMbt=8, bufSze=256*1024, blkSze=4096, ahdMax=0.
Because presets assign unconditionally, `-b -m 128` ≠ `-m 128 -b` — parse order matters and is
part of the contract.

### 4.2 Greeting, usage, exit-on-args

Greeting to stddbg (stderr default, stdout with `-do`) when `verbose>0 || help || nargs<3`
(`main.cpp:318-344`): version line, copyright line, blank, 10-line license block, blank, then

```
File adressing is 64 bit (files up to 8388608 TB), samples are 4 bytes.
```

computed as `((MAX_OFF_T>>30)+1)`; if that exceeds 1024, shift right by 10 and use `"TB"`, else
`"GB"` (exact digits at 64-bit: 8589934593 → 8388608 TB).

Usage/help text (`main.cpp:347-383`) is printed when `nargs<3 || help || verbose>2`; the
`-m size` line includes `, 0=no buffers` on non-MinGW builds (always include in Rust — matches
the Linux oracle). The `-min`/`-max` lines print the **current** (post-parse) values and
`MCH_MAX`. When `nargs<3 || help`: `exit(EXI_ARG)` (2).

### 4.3 File handling and engine wiring (`main.cpp:389-537`)

* `org = argv[1+optidx]`, `new = argv[2+optidx]`, out = argv[3+optidx] or `"-"`.
* Open org for reading; on failure: `Could not open first file %s for reading.\n` → exit 3.
  Then new (`Could not open second file %s for reading.\n` → exit 4). Then output (`"wb"`; stdout
  for `-`); failure: `Could not open output file %s for writing.\n` → exit 5.
* Buffered mode (`bufSze > 0`, the default): each input gets a look-ahead reader
  (`JFileAhead` semantics, §10) with `bufSze`, `blkSze`, ids `"Org"`/`"New"`.
* `-m 0` mode: read each file fully into memory and serve via the in-memory reader (§9).
* Output object: type 0 → binary, 1 → ASCII, 2 → region (§11).
* Engine: `JDiff::new(org, new, out, hsh_mbt*1024*1024, verbose, src_bkt, src_scn, mch_max,
  mch_min, ahd_max==0?bufSze:ahd_max, cmp_all)`.
* Verbose>1 pre-run lines (`main.cpp:539-541`):
  `Lookahead buffers: %lu kb. (%lu kb. per file).\n` (bufSze*2/1024, bufSze/1024) and
  `Hastable size    : %d kb. (%d samples).\n` ((hashsize_bytes+512)/1024, prime) — the
  `Hastable` typo is intentional and must be kept.

### 4.4 Post-run statistics

Verbose>1 (`main.cpp:546-562`), exact labels/widths (`%d` → i32, `%ld` → i64, P8zd → `{:>12}`):

```
Hashtable size          = {hashsize_bytes} samples, {(hashsize_bytes+512)/1024} KB, {((hashsize_bytes+512)/1024+512)/1024} MB
Hashtable prime         = {prime}
Hashtable hits          = {hash_hits}
Hashtable errors        = {hsh_err}          // 0 in release builds
Hashtable repairs       = {si_hsh_rpr}       // JMatchTable static
Hashtable overloading   = {colmax/3 - 1}
Reliability distance    = {reliability}
Random    accesses      = {org_seeks + new_seeks}
Delete    bytes         = {del:>12}
Backtrack bytes         = {bkt:>12}
Escape    bytes written = {esc:>12}
Control   bytes written = {ctl:>12}
```

Verbose>0 additionally (`main.cpp:563-567`):

```
Equal     bytes         = {eql:>12}
Data      bytes written = {dta:>12}
Overhead  bytes written = {ctl+esc:>12}
```

Note `hashsize_bytes = prime * 12` (`sizeof(off_t)+sizeof(hkey)`, `JHashPos.cpp:62`) — yes, the
label says "samples" while the value is bytes; replicate.

### 4.5 Exit codes (`main.cpp:589-613`)

Engine error returns map to: `-EXI_SEK` → `Seek error !`, `-EXI_LRG` → `64-bit offsets not
supported !`, `-EXI_RED` → `Error reading file !`, `-EXI_WRI` → `Error writing file !`,
`-EXI_MEM` → `Error allocating memory !`, `-EXI_ERR` → `Spurious error occured !` (printed
**without newline**, then exit with the code). Otherwise: `1` if `out.dta==0 && out.del==0`
(equal files), else `0`.

## 5. `jptch` CLI specification (`jpatch.cpp`)

Invocation: `jptch [options] <original> <patch> [<output>]`; any of the three names may be `-`
(stdin/stdout) (`jpatch.cpp:387-409`).

Options (`jpatch.cpp:316-335`): `-v` (1), `-vv` (2), `-vvv` (3), `-d` (stddbg=stdout), `-h`
(help), `-t` (verbose=2 **and** test flag — the flag is stored but never alters behavior in
0.8.1; keep it as an accepted no-op that raises verbose, exactly like C++).

Greeting when `verbose>0 || help || nargs<3` (`jpatch.cpp:338-357`): same version/license block,
then `File adressing is 64 bit.\n` (no sizes). Usage when `nargs<3 || help` → `exit(EXI_ARG)`
(`jpatch.cpp:359-376`); help text lines are byte-exact from `jpatch.cpp:360-373` (includes the
`-t  Test: no output file.` line).

Open errors (to stddbg, exit codes 3/4/5): `Could not open data file %s for reading.\n`,
`Could not open patch file %s for reading.\n`, `Could not open output file for writing.\n`
(note: no filename in the third). Success path always `exit(0)`.

### 5.1 Decoder semantics (`jpatch()` `jpatch.cpp:118-294`)

State: current operand `liOpr` (initially `ESC`), pending-skip counter `lzMod` (bytes written by
MOD that were **not** consumed from the original file), `lbEsc` (previous token was `ESC <xxx>`
with unknown xxx), `lbChg` (an opcode was just consumed). Loop reads one patch byte at a time:

* `ESC` → read next:
  * `MOD`/`INS`: set operand, `lbChg=true`; verbose==1 prints
    `%12lld %12lld MOD ...    \n` / `INS ...    \n` with (orgpos+lzMod-1, outpos).
  * `DEL`: `lzOff = ufGetInt()`; seek original **forward** `lzOff + lzMod` from current
    (absolute arithmetic identical because MOD didn't advance the original read position);
    `lzMod = 0`; verbose≥1: `%12lld %12lld DEL %lld\n`. Seek failure →
    `Could not position on original file (seek %lld + %lld).\n` to **stderr**, exit 6.
  * `EQL`: `lzOff = ufGetInt()`; if `lzMod>0` seek original forward `lzMod` (failure →
    `Could not position on original file (skip %lld).\n`, exit 6), `lzMod=0`; copy `lzOff`
    bytes original→output in ≤4096-byte blocks (`BLKSZE`, `jpatch.cpp:43`); short read →
    `Error reading original file.\n` (stderr, exit 8); short write →
    `Error writing output file.\n` (stderr, exit 9); verbose≥1 line
    `%12lld %12lld EQL %lld\n`.
  * `BKT`: `lzOff = ufGetInt()`; seek original `lzMod - lzOff` relative (i.e. backwards by
    `lzOff - lzMod`); failure → `Could not position on original file (seek back %lld -
    %lld).\n`, exit 6; `lzMod=0`; verbose≥1 `%12lld %12lld BKT %lld\n`.
  * `ESC`: literal data ESC (no state change); verbose>2 `ESC ESC` line.
  * anything else: `lbEsc=true`; verbose>2 `ESC XXX` line.
* Non-ESC byte with `lbChg==false`:
  * `MOD`: if `lbEsc`, write `ESC` byte first and `lzMod++`; then write the byte, `lzMod++`;
    verbose>2 `%12lld %12lld MOD %3o %c\n` (orgpos+lzMod-1, outpos-1) / `MOD %3o ESC` variant.
  * `INS`: if `lbEsc`, write `ESC` first (no lzMod change); write byte; verbose>2
    `%12lld %12lld INS %3o %c\n` (orgpos+lzMod-1, outpos — not decremented).
  * `DEL`/`EQL`/`BKT`/`ESC`(initial state): byte is silently ignored.
* `lbEsc=false` and `lbChg=false` at the end of each iteration.
* End of patch stream: verbose>1 prints `%12lld %12lld EOF` (**no trailing newline**).

Positions printed are logical: original position = bytes consumed from original file + `lzMod`.
`%c` renders 32..=127 as the byte else space; `%3o` is 3-wide octal.

`ufGetInt` (`jpatch.cpp:70-105`): b<252 → b+1; 252 → 253+next; 253 → 2-byte BE; 254 → 4-byte BE;
255 → 8-byte BE. The C++ non-largefile error path (`64-bit length numbers not supported!` +
exit 7) is dead in the oracle build but is kept for `-` compatibility only if reachable — port
the 8-byte form as live code (largefile always on), matching the oracle.

## 6. JDiff engine specification (`JDiff.cpp`)

Constructor (`JDiff.cpp:83-99`): `ahd_max = max(ahd_max, 1024)`; builds `JHashPos(hsh_sze)` and
`JMatchTable(hash, org, new, cmp_all)`.

### 6.1 `jdiff()` main loop (`JDiff.cpp:122-256`)

```
read lcOrg=org.get(0,0); lcNew=new.get(0,0); lzPosOrg=lzPosNew=0
lbEql=false; lzEql=0; lbFnd=false; lzAhd=lzSkpOrg=lzSkpNew=0
while lcNew >= 0:
  if lcOrg == lcNew:
      if lbEql: lzEql++
      else: lbEql = out.put(EQL,1,lcOrg,lcNew,lzPosOrg,lzPosNew)
      lcOrg=org.get(++lzPosOrg,0); lcNew=new.get(++lzPosNew,0); lzAhd--
  elif lzAhd > 0:
      ufPutEql(...)                       # flush accumulated equals
      if lcOrg < 0: out.put(INS,1,lcOrg,lcNew,...); lcNew=new.get(++lzPosNew,0)
      else:         out.put(MOD,1,lcOrg,lcNew,...); lcOrg=org.get(++lzPosOrg,0); lcNew=new.get(++lzPosNew,0)
      lzAhd--
  elif lbFnd && lzAhd == 0:
      lzAhd = 32 (SMPSZE); lbFnd = false  # anti-infinite-loop guard
  else:
      lbFnd = ufFndAhd(lzPosOrg,lzPosNew,&lzSkpOrg,&lzSkpNew,&lzAhd)
      if lbFnd < 0: return lbFnd          # error
      ufPutEql(...)
      if lzSkpOrg > 0:  out.put(DEL,lzSkpOrg,0,0,...); lzPosOrg+=lzSkpOrg; lcOrg=org.get(lzPosOrg,0)
      elif lzSkpOrg < 0: out.put(BKT,-lzSkpOrg,0,0,...); lzPosOrg+=lzSkpOrg; lcOrg=org.get(lzPosOrg,0)
      if lzSkpNew > 0:
          while lzSkpNew>0 && lcNew > EOF(-1):
              out.put(INS,1,0,lcNew,...); lzSkpNew--; lcNew=new.get(++lzPosNew,0)
flush: ufPutEql(...); out.put(ESC,0,0,0,lzPosOrg,lzPosNew)
if lcNew < EOB(-2) || lcOrg < EOB: return min(lcNew,lcOrg)
return 0
```

`ufPutEql` (`JDiff.cpp:261-268`): if `lzEql>0`: `out.put(EQL, lzEql, 0, 0, lzPosOrg-lzEql,
lzPosNew-lzEql)`; `lzEql=0`; **always** `lbEql=false`.

### 6.2 `ufFndAhd()` — find-ahead (`JDiff.cpp:285-488`)

1. If `src_scn==1`: run `ufFndAhdScn()` once (error propagates), set `src_scn=2`.
2. Lookahead budget `liMax` (`JDiff.cpp:313-324`): if `src_scn==2`:
   `mz_ahd_new==0 || mz_ahd_new < az_red_new` → `miAhdMax`; `mz_ahd_new > az_red_new+miAhdMax`
   → `miAhdMax`; else `miAhdMax - (mz_ahd_new-az_red_new)`. If `src_scn==0`: `INT_MAX/2`.
3. Look-back `liBck` (`JDiff.cpp:329-333`): `reliability < miAhdMax ? reliability/2 : miAhdMax/2`.
4. Org hash re-init if `src_scn==0 && (mz_ahd_org==0 || mz_ahd_org+liBck < az_red_org)`
   (`JDiff.cpp:340-352`): `mz_ahd_org = max(0, az_red_org-liBck)`; reset hash + equal-run
   counter; hash the next 31 bytes (`SMPSZE-1`) via `ufFndAhdGet` while value > EOF.
5. New hash re-init if `mz_ahd_new==0 || mz_ahd_new+liBck < az_red_new` (`JDiff.cpp:353-368`):
   same pattern; additionally `liMax += liBck` then `liMax--` per hashed byte.
6. If `mch.cleanup(az_red_new - reliability)` returns false (no space): the whole match-building
   block is skipped and control goes to step 8 (`JDiff.cpp:373`).
7. Build matches (`JDiff.cpp:374-437`): `lzBseOrg = src_bkt ? 0 : az_red_org`; if `src_scn>0`
   set `mi_val_org = EOB` (org not read live). While `liMax>0 && (val_new > EOF || val_org >
   EOF)`:
   * if `val_org > EOF`: `hash(val_org, ml_hsh_org)`; `hsh.add(ml_hsh_org, mz_ahd_org,
     eql_org)`; `ufFndAhdGet(org, ++mz_ahd_org, val_org, eql_org, 1)`.
   * if `val_new > EOF`: `hash(val_new, ml_hsh_new)`; if `hsh.get(ml_hsh_new, &lz_fnd_org)` and
     `lz_fnd_org > lzBseOrg`: `r = mch.add(lz_fnd_org, mz_ahd_new, az_red_new, eql_new)`
       * `0` (added, table full): if `liBck>0 && mch.cleanup(az_red_new)` made room → continue
         scanning; else `liMax=0; continue`.
       * `1` (added): if `mz_ahd_new > az_red_new`: `liFnd++`; if `liFnd==mch_max` →
         `liMax=0; continue`; elif `liFnd==mch_min && liMax>reliability` → `liMax=reliability`.
       * `2`/`-1`: no action.
     `ufFndAhdGet(new, ++mz_ahd_new, val_new, eql_new, 1)`; `liMax--`.
8. Error check: if `val_new < EOB || val_org < EOB` return min (negative error code)
   (`JDiff.cpp:442-444`).
9. Best-match offsets (`JDiff.cpp:449-487`): if `!mch.get(az_red_org, az_red_new, &lzFndOrg,
   &lzFndNew)` → `skp_org=0; skp_new=0; ahd=(mz_ahd_new-az_red_new)-reliability;
   if ahd<32 {ahd=32}; return 0`. Else (`lzFndOrg` vs `azRedOrg`):
   * `lzFndOrg >= azRedOrg`:
     * if `lzFndOrg-azRedOrg >= lzFndNew-azRedNew` (go forward on org):
       `skp_org = lzFndOrg-azRedOrg + azRedNew-lzFndNew; skp_new=0; ahd=lzFndNew-azRedNew`
     * else (forward on new):
       `skp_org=0; skp_new = lzFndNew-azRedNew + azRedOrg-lzFndOrg; ahd=lzFndOrg-azRedOrg`
   * else (backtrack on org): `d = azRedOrg-lzFndOrg + lzFndNew-azRedNew`;
     * if `d < azRedOrg`: `skp_new=0; skp_org=-d; ahd=lzFndNew-azRedNew`
     * else: `skp_new=d-azRedOrg; skp_org=-azRedOrg; ahd=(lzFndNew-azRedNew)-skp_new`
     * `mz_ahd_org = 0` (reset ahead position when backtracking)
   * `return 1`.

### 6.3 `ufFndAhdGet()` (`JDiff.cpp:504-513`)

Reads next byte at `pos+1` with read-type `1` (hard ahead); updates the sample equal-run
counter: new != prev → `if eql>0 {eql-=2}`; new == prev → `if eql<32 {eql+=1}`.

### 6.4 `ufFndAhdScn()` — prescan (`JDiff.cpp:519-583`)

Verbose>0 prints `Prescanning:\n`. Hash bytes 0..30 (31 bytes) as the rolling-hash seed, then
loop while value > EOF: `hash(); hsh.add(key, pos, eql); ufFndAhdGet();` — progress dots: a
counter increments per add; every `0x1000000` adds print `.`; at `0x40000000` print `.\n` and
reset; after loop print `.\n`. (The C++ OpenMP pragma is only active in the never-used `make
parallel` target and is a data race there; the oracle is the **serial** default build — the Rust
port is serial, period.) Debug builds print `hsh.dist(pos, 128)`. Return value: last byte if
< EOB (error), else 0.

## 7. JHashPos specification (`JHashPos.cpp`)

* Prime table `giPme[20] = {134217689, 67108859, 33554393, 16777213, 8388593, 4194301, 2097143,
  1048573, 524287, 262139, 131071, 65521, 32749, 16381, 8191, 4093, 2039, 1021, 509, 251}`
  (`JHashPos.cpp:38-43`). Selection (`JHashPos.cpp:58-61`): `idx=0; while idx<19 &&
  giPme[idx] > aiSze {idx++}`; `prime = giPme[idx]` — so e.g. requested 8388608 → 8388593;
  requested ≤ 509 → 251 floor; requested ≥ 134217689 → 134217689.
* `hash(b, &key)`: `key = key*2 + b` wrapping u32 (`JHashPos.h:109-117`).
* `add(key, pos, eql_cnt)` (`JHashPos.cpp:96-137`), in order:
  1. Load counter: if `load_cnt < prime {load_cnt++} else {load_cnt=0; col_max += 4; rlb += 4}`
     — counts **every** add call, stored or not.
  2. Collision credit: `col_cnt += (eql_cnt <= 28 ? 4 : 1)`.
  3. If `col_cnt >= col_max`: `idx = key % prime` (u32 mod); **overwrite** bucket `{key, pos}`;
     `col_cnt = 0`.
  Initial state (`JHashPos.cpp:55-56`): `col_max=4, col_cnt=4, rlb=48, load_cnt=0, hits=0`.
  Single-slot-per-bucket: no probing; `get` is exact-key match at `key % prime` only, and
  increments `hits` on match (`JHashPos.cpp:145-158`).
* Getters used by stats: `hash_prime`, `hash_size_bytes = prime*12`, `hash_colmax`, `hash_hits`,
  `reliability`.
* `print()` (`JHashPos.cpp:163-172`) and `dist(max, 128)` (`JHashPos.cpp:179-223`): debug-only
  printouts; formats byte-exact (`Hash Pnt %12d {:>12}-%08lx x\n`, the dist block). Ported under
  the debug feature (§14).

## 8. JMatchTable specification (`JMatchTable.cpp`)

Fixed pool of 256 nodes (`rMch`), free-list, 127 buckets keyed on `delta = fnd_org - fnd_new`:
`idx = delta % 127; if idx < 0 { idx = -idx }` (C trunc-mod, then negate — so −1 and +1 share
bucket 1). Node fields: `typ` (0 unknown, 1 colliding, −1 gliding), `cnt`, `beg_new`, `new`,
`org`, `delta`.

### 8.1 `add(fnd_org, fnd_new, base_new, eql_new)` (`JMatchTable.cpp:103-180`)

1. Gliding continuation: if a last-node exists and `delta == gld_delta`: mark it gliding
   (`typ=-1`), `cnt++`, `new=fnd_new`, `gld_delta--`, return 2. Else clear last-node pointer.
2. Colliding: walk bucket chain; if a node has `delta` equal: `cnt++; typ=1; new=fnd_new;
   org=fnd_org`; return 2.
3. New node from free-list (if any): fill `{org, new, beg=fnd_new, delta, cnt=1, typ=0}`,
   prepend to bucket, remember as potential gliding with `gld_delta = delta-1`; return
   `free_list_is_nonempty_after_pop as i32` (1 = still place, 0 = table now full).
4. No free node: return 0 (not added).

### 8.2 `get(red_org, red_new) -> Option<(best_org, best_new)>` (`JMatchTable.cpp:186-332`)

`rlb = max(reliability, 1024)`; `FZY = 0`. Iterate all 127 buckets and their chains:

* Skip empty (`cnt==0`) or old (`new + reliability < red_new`) nodes.
* Candidate filter — evaluate only if: `best.is_none() || (beg - rlb < best_new + FZY && (
  red_new < best_new + FZY || cnt_now > best_cnt))` where `cnt_now = (typ<0) ? 0 : cnt`.
* Test position (`JMatchTable.cpp:225-234`): `tst_new = beg - rlb`; if `tst_new >= red_new` →
  `dst = rlb`; else `tst_new = red_new; dst = max(beg - tst_new, rlb)`.
* Test position on org (`JMatchTable.cpp:237-257`): gliding (`typ<0`): if `tst_new >= beg` →
  `tst_org = org` else `tst_org = tst_new + delta` (clamped: if `tst_org<0` then
  `tst_new -= tst_org; tst_org = 0`). Colliding: `tst_org = tst_new + delta` with the same
  clamp.
* Verify: `cmp = check(tst_org, tst_new, dst, cmp_all ? 1 : 2)`.
* Soft-EOF recovery (`liCurCmp==1`): if `cnt < 2` → cmp = 7; else if `beg >= red_new` →
  `tst_new = beg`; elif `new >= red_new` → `tst_new = red_new`; else cmp = 7; and (when not 7)
  `tst_org = tst_new + delta`.
* False-match repair: `cmp >= 2` → `cnt--`, `si_hsh_rpr++` (global static).
* Accept (cmp ≤ 1) if: `best.is_none() || tst_new+FZY < best_new || (tst_new <= best_new+FZY &&
  cnt_now > best_cnt && cmp <= best_cmp)` → new best `{tst_org, tst_new, cnt_now, cmp}`.
* Return best as `Option`.

### 8.3 `cleanup(base_new) -> bool` (`JMatchTable.cpp:337-371`)

Remove from every chain nodes with `cnt==0 || new < base_new`, push them on the free-list;
return `!free_list.is_empty()`.

### 8.4 `check(&mut pos_org, &mut pos_new, len, soft) -> 0|1|2` (`JMatchTable.cpp:389-460`)

Searches for a run of 24 equal bytes (`SMPSZE-8`) within `len` bytes starting at the given
positions, hard (`soft=false`, read-type 1) or soft (read-type 2):

* Phase 1 — while `len > 24 && ret==0 && eql < 24`: read next byte from each file (advancing
  positions); equal → `eql++`; either < 0 → `ret=1`; else `eql=0` (mismatch does **not** fail).
* Phase 2 — while `len > 0 && ret==0 && eql < 24`: same read; equal → `eql++`; either < 0 →
  `ret=1`; mismatch → `ret=2` (fail).
* Post: `ret==0` → rewind both positions by `eql` (optimization anchor); `ret==1` → if last
  org/new byte was exactly `EOF` (-1, hard end) → `ret=2`; else advance both positions by the
  remaining `len` (soft end).

## 9. In-memory JFile (`-m 0`) — intended `JFileIStream` semantics

Serves bytes from a `Vec<u8>`: `get(pos, typ)` ignores `typ` (never returns EOB); returns the
byte or `EOF` when `pos >= len` or `pos < 0`. Sequential-read fast path: keep `pos_inp`; when
`pos != pos_inp` increment `seekcount` (that is all the counter is used for). This matches
`JFileIStream.cpp:46-58` minus the C++ `istringstream(char*)` NUL-truncation defect (§15.3).

## 10. Buffered look-ahead JFile (`JFileAhead.cpp`) — default mode

Circular buffer `buf: Vec<u8>` of `buf_sze` bytes read in `blk_sze` chunks from a `Read+Seek`.
State: `pos_inp` (file offset of next unread chunk byte), `buf_usd` (valid bytes in buffer),
`pos_red`/`ptr_red` (last served position + index, sequential fast path via `red_sze`), `pos_eof`
(initially `i64::MAX`). Semantics, exactly (`JFileAhead.cpp:69-346`):

* `get(pos, typ)`:
  1. If `red_sze > 0 && pos == pos_red`: serve `buf[ptr_red]`, advance `ptr_red` (wrap),
     `pos_red++`, `red_sze--`.
  2. Else `get_frombuffer(pos, typ)`:
     * `pos < pos_inp`:
       * if `pos >= pos_inp - buf_usd` → in-buffer: compute index (wrap), set
         `pos_red=pos+1; ptr_red=idx+1 (wrap); red_sze = (ptr_red > ptr_inp ? buf_len -
         ptr_red : pos_inp - pos_red)`, return byte.
       * else (before buffer): if `pos + blk_sze >= pos_inp - buf_usd` → seek-mode 2 (scroll
         back) else seek-mode 1 (reset).
     * `pos >= pos_eof` → reset read cursor (`pos_red=-1, red_sze=0`), return `EOF`.
     * `pos >= pos_inp + blk_sze` → seek-mode 1 (reading far after buffer).
     * otherwise seek-mode 0 (append: buffer already covers it or can extend).
     * If `typ == 2` (soft) and seek-mode ≠ 0 → return `EOB` **without** any file access.
  3. `get_outofbuffer(pos, typ, seek_mode)`:
     * mode 0 (append): read `min(buf_len - ptr_inp, blk_sze)` bytes at `ptr_inp` from file
       offset `pos_inp` (no seek performed).
     * mode 1 (reset): reset buffer (`ptr_inp=0; pos_inp=pos; ptr_red=0; pos_red=pos;
       buf_usd=0; red_sze=0`); seek file to `pos`; read `blk_sze` bytes at index 0;
       `seekcount++`.
     * mode 2 (scroll back): make room: `drop = buf_usd + blk_sze - buf_len` (if >0:
       `buf_usd-=drop; pos_inp-=drop; ptr_inp-=drop (wrap)`). Then `lz_pos = pos_inp - buf_usd`;
       `todo = min(blk_sze, lz_pos)`; placement by the 4 pointer cases (`JFileAhead.cpp:236-253`):
       `lp = ptr_inp - buf_usd`; if `lp == 0` → `lp = buf_len - todo` (case 1); elif `lp > 0` →
       if `lp - todo >= 0` → `lp -= todo` (case 4) else `todo = lp; lp = 0` (case 3); else (lp
       wrapped negative) → `lp += buf_len - todo` (case 2). `buf_usd += todo; lz_pos -= todo`;
       seek to `lz_pos`, read `todo` at `lp`; `seekcount++`; reset read cursor
       (`ptr_red=null, pos_red=-1, red_sze=0`). Unreachable in every real build:
       the C++ `bool liSek` collapses mode 2 to 1 at the call (see §15.8); the
       port replicates the collapse and keeps this arm as dead code.
     * Partial read (`done < todo`): `pos_eof = lz_pos + done`; if `done == 0` return `EOF`.
     * Bookkeeping: mode 2 partial → repair buffer (`ptr_inp = lp+done (wrap); pos_inp =
       lz_pos+done; ptr_red=lp; pos_red=lz_pos; buf_usd=done; red_sze=done`); mode 2 full →
       seek back to `pos_inp` (`seekcount++` again). Modes 0/1 → `pos_inp += done; ptr_inp +=
       done (wrap; if past end → **panic exit code 6** with `Buffer out of bounds on position
       %lld)!` to stderr — faithful to `JFileAhead.cpp:329`); `buf_usd = min(buf_usd+done,
       buf_len)`; `red_sze += done`; `if ptr_red == buf_len {ptr_red = 0}`.
     * Recurse: `return get(pos, typ)` (now served from buffer).
* `seekcount()` returns the counter.

## 11. Output layer specification

### 11.1 `JOutBin` (`JOutBin.cpp:26-225`)

State: `opr_cur=ESC`, `eql_cnt=0`, `eql_buf=[0;4]`, `out_esc=false`; writes to a `Write`.

* `put_len(len)` per §3 table; each form adds its byte count to `ctl`.
* `put_opr(opr)`: if `out_esc` → write `ESC ESC`, `out_esc=false`, `esc++`, `dta++` (a pending
  data-ESC is flushed as data followed by its escape twin). If `opr != ESC` → write
  `ESC, opr`, `ctl += 2`.
* `put_byte(b)`: if `out_esc` { `out_esc=false`; if `BKT <= b <= ESC` { write extra `ESC`;
  `esc++` } write the pending `ESC` as data (`dta++`) } then if `b == ESC` { `out_esc=true` }
  else { write `b`; `dta++` }.
* `put(opr, len, org, new, ...)`:
  1. Flush pending equals when `opr != EQL && eql_cnt > 0`: if `eql_cnt > 4 || (opr_cur != MOD
     && opr != MOD)` → emit `EQL` opcode + `put_len(eql_cnt)`, `eql_stat += eql_cnt`; else →
     ensure `MOD` opcode and emit the buffered bytes via `put_byte`. `eql_cnt=0`.
  2. `ESC` → `put_opr(ESC); opr_cur=ESC`.
     `MOD`/`INS` → if `opr_cur != opr` emit opcode; `put_byte(new)`.
     `DEL` → `put_opr(DEL); put_len(len); opr_cur=DEL; del+=len`.
     `BKT` → `put_opr(BKT); put_len(len); opr_cur=BKT; bkt+=len`.
     `EQL` → if `eql_cnt < 4` { `eql_buf[eql_cnt++]=org`; return `eql_cnt >= 4` } else
     { `eql_cnt += len`; return true }.
  3. Return false otherwise (caller keeps sending byte-wise EQL).

### 11.2 `JOutAsc` (`JOutAsc.cpp:35-126`) — `-l`

Ignores `ESC` calls. Per call prints `{:>12} {:>12} ` (pos_org, pos_new) then:

```
MOD {org:03o} {new:03o} {c(org)}-{c(new)}\n     # c(b) = byte if 32..=127 else ' '
INS     {new:03o}  -{c(new)}\n
DEL {len}\n
BKT {len}\n
EQL {org:03o} {new:03o} {c(org)}-{c(new)}\n
```

Stats: MOD/INS `ctl+=2` on operand change, `dta++`, `esc++` when the byte == ESC; DEL/BKT
`ctl += 2 + put_sze(len)`, `del/bkt += len` where `put_sze` = 1/2/3/5/9 by the §3 ranges; EQL
`ctl += 2+4` on change, `eql++` per byte. Always returns false (byte-wise detail).

### 11.3 `JOutRgn` (`JOutRgn.cpp:32-95`) — `-lr`

Accumulates `cnt` per operand (EQL/DEL/BKT/MOD/INS add `len`; MOD/INS with byte == ESC also
`esc++`). On operand change, emits the **previous** operand's line and stats (`ctl+=2`;
`dta+=cnt` MOD/INS; `del+=cnt` DEL; `bkt+=cnt` BKT; `eql+=cnt` EQL):

```
MOD {pos_org-cnt:>12} {pos_new-cnt:>12} MOD {cnt}\n
INS {pos_org:>12} {pos_new-cnt:>12} INS {cnt}\n
DEL {pos_org-cnt:>12} {pos_new:>12} DEL {cnt}\n
BKT {pos_org+cnt:>12} {pos_new:>12} BKT {cnt}\n
EQL {pos_org-cnt:>12} {pos_new-cnt:>12} EQL {cnt}\n
```

Always returns true (length-mode). The engine's final `put(ESC,…)` flushes the last region.

## 12. Cross-platform requirements

* Std-only (`std` crate); no `unsafe`; no libc; no platform `#[cfg]` in behavior paths.
* Binary mode I/O everywhere (Rust default; C++ `"rb"`/`"wb"` equivalence).
* Use `std::env::args_os` + `OsString` file names so non-UTF-8 paths work on Windows/Linux.
* stdin/stdout via `"-"` (jptch: all three files; jdiff: output). jptch original-file on stdin:
  read fully into memory (Cursor) — see §15.4.
* CI matrix: `ubuntu-latest`, `windows-latest`, `macos-latest` (cargo fmt --check, clippy
  `-D warnings`, `cargo test --all-features`, release build). Oracle-compare job runs only on
  ubuntu (needs g++).

## 13. Test corpus

* Bundled pairs vendored from the C++ repo `tests/`: `bkocomu.0000.fil`/`bkocomu.0009.fil`
  (~1.8 MB binary, contains NULs) and `test2.001.txt`/`test2.002.txt` (72 KB CRLF text).
* Generated pairs (deterministic PRNG, fixed seeds): identical files, empty original, empty new,
  pure-insert, pure-delete, modify runs, repeats triggering BKT, data bytes 0xA2–0xA7 (escaping),
  lengths crossing 252/253/508/509/65535/65536 boundaries, > 4 GiB-length EQL is covered by a
  unit test on `put_len` only (no such fixture on disk).
* Option matrix for oracle byte-compare (on both bundled pairs): default, `-f`, `-ff`, `-b`,
  `-s 1`, `-s 32`, `-bs 512`, `-m 64`, `-m 1`(→0→in-memory), `-min 1 -max 1`, `-a 16`,
  `-l`, `-lr`, and `-m 0` on the text pair (NUL-free — reference `-m 0` is broken on NUL data,
  §15.3).

## 14. Debug feature (parity with `make debug` / `-D_DEBUG`)

Feature `debug` (off by default, mirroring the default `make` build). Enables: the 11 CLI flags
`-dhsh -dahd -dcmp -dprg -dbuf -dhsk -dahh -dbkt -dred -dmch -ddst` mapping to
`DBGHSH..DBGDST` (`JDebug.h:37-47`), the `gbDbg` flag array, and every `#if debug` print site —
the full site list with exact format strings (note debug builds use `P8zd = %10lld`, i.e.
`{:>10}`): `main.cpp` has none (only option parsing); `JDiff.cpp` sites at lines 145-148
(DBGPRG input), 189-191, 204-208, 216-222 (DBGAHD/DBGPRG), 388-392 & 546-550 (DBGAHH),
`JHashPos.cpp:66-72` (DBGHSH ini), 124-130 (DBGHSH add), `JMatchTable.cpp:166-170` (DBGMCH add),
174-176 (full), 302-310 & 314-319 & 325-328 (DBGMCH table), 398-402 & 430-437 (DBGCMP),
`JFileAhead.cpp:50-54, 76-81, 118-122, 146-151, 165-170, 270-273, 283-288, 294-298` (DBGBUF/
DBGRED). `-do`/`-d` and `stddbg` exist in **all** builds. Also `JHashPos::print()`/`dist()` and
the debug-only `Hash miss!` / `liErr` logic in `jdiff()` (`JDiff.cpp:137-209`).

## 15. Documented deviations (exhaustive — everything else is 1:1)

1. **Linux `fillBuffer` bug not ported.** `main.cpp:441-463` pre-reads both files on pthreads,
   leaving the ifstreams at EOF in fail state, so the stock Linux build emits empty patches for
   the default mode (verified: 0-byte patch, exit 1, wrong jptch output). The Rust port opens
   files directly — behavior equals the C++ build with the 2-line fix
   (`liFilOrg->clear(); liFilOrg->seekg(0);` and same for new) **or** the MinGW build. This is
   the intended JojoDiff behavior and is what the oracle fixture set uses.
2. **OpenMP not ported.** `JDiff.cpp:541` pragma is only compiled by the unused `make parallel`
   target (where it is a data race). Port is serial and deterministic — identical to the default
   oracle build.
3. **`-m 0` NUL-truncation fixed.** `main.cpp:169` builds `istringstream(char*)`, truncating at
   the first NUL byte (verified: `-m 0` on bkocomu pair produces a 3-byte patch). The Rust
   in-memory reader serves the full file. Byte-compare against the C++ reference for `-m 0` is
   therefore restricted to NUL-free inputs (text pair); NUL inputs are verified by round-trip
   only.
4. **jptch stdin buffering.** C++ seeks the stdin `FILE*`; pipes fail there. The Rust port reads
   stdin fully then serves from memory — strictly more permissive, identical output for
   seekable stdin.
5. **MinGW/`__MINGW32__` ifdefs collapsed.** One uniform buffered implementation (JFileAhead
   semantics) on all platforms; the MinGW-only `-m 0 → buf_sze = blk_sze` special case
   (`main.cpp:398-402`) does not apply since in-memory mode is used everywhere.
6. **`JFileIStreamAhead` and `JFileAhead` are one implementation.** They are byte-for-byte the
   same algorithm (`JFileIStreamAhead.cpp` is a clone with istream calls); the Rust port has a
   single `JFileAhead` over `Read+Seek`.
7. **francisdb/jojodiff-rs edge cases are NOT authoritative.** Known divergences from C++
   jptch (bare leading bytes treated as MOD data vs. silently dropped; `ESC`+same-operand
   emitted as data vs. operand restart) exist there; the Rust port follows C++ everywhere.
8. **Scroll-back mode 2 is dead code.** `get_frombuffer` declares its seek-mode variable as
   `bool liSek` (`JFileAhead.cpp:108`, same in `JFileIStreamAhead.cpp:113`) and passes it to
   `get_outofbuffer(const int aiSek, ...)` (`JFileAhead.cpp:173`): the bool→int conversion
   makes any non-zero seek mode (including the assigned 2 for "just before buffer") arrive as
   exactly 1, so the `case (2)` scroll-back code never executes in any real JojoDiff 0.8.1
   build (MinGW and Linux, both twins). Observable effect: near-before-buffer reads take the
   seek-&-reset (mode 1) path — 1 seek instead of the scroll-back's 2, and buffer history is
   reset. The port replicates the collapse explicitly at the `get_frombuffer` →
   `get_outofbuffer` boundary and retains the mode-2 arm as dead-code parity.

## 16. Acceptance gates

1. **Round-trip:** for every fixture pair × every option in §13's matrix:
   `jdiff A B p && jptch A p out && cmp B out` succeeds; exit code of jdiff is 0 (differences)
   or 1 (identical files → patch is 0 bytes and jptch output is empty).
2. **Oracle byte-equality:** Rust `jdiff` output == fixed-C++ `jdiff` output (patch bytes;
   `-l`/`-lr` text) for the whole §13 matrix, and Rust `jptch` output == C++ `jptch` output for
   every oracle patch. Verbose/greeting/help output byte-equal on stderr (stdout with `-do`).
3. **Exit codes & messages:** every EXI path exercised (missing args, unopenable org/new/out,
   truncated patch, 6/8/9-class errors) with byte-exact messages.
4. **Cross-validation (optional but included):** apply Rust-produced patches with the
   `jojodiff` crate v0.1.2 (dev-dependency) on NUL-free fixtures — guards against accidental
   format drift beyond the C++ oracle.
5. **Cross-platform:** CI green on ubuntu/windows/macos (§12).
