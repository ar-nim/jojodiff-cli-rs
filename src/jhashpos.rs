//! Sample hash table mapping 32-bit hash keys to 64-bit file positions,
//! ported 1:1 from C++ `src/JHashPos.cpp` + `src/JHashPos.h` 0.8.5 (spec
//! §18.E).
//!
//! Only samples from the original file are stored; samples from the new file
//! are looked up. There is one slot per bucket: `add` overwrites the bucket
//! when the down-counting collision counter reaches 0, and `get` is an
//! exact-key match at `key % prime` — there is no probing.
//!
//! # Constructor sizing (0.8.5, spec §21.18)
//!
//! The 0.8.5 constructor takes the size in **MB** (`JHashPos.cpp:48-61`,
//! `main.cpp:283` default 32) and converts it to an element count with the
//! port variant's element size `sizeof(hkey) + sizeof(off_t) = 4 + 8 = 12`
//! (u32 hkey, §2/§15.12; spec §21.18 amended the earlier stock-LP64 `/16`
//! ruling during the final review): default 32 MB → `32*1024*1024/12 =
//! 2796202` elements → prime 2796181 via [`crate::defs::get_lower_prime`] —
//! the same table the 32-bit-hkey oracle build and the acceptance tests use.
//! The size in bytes (`miHshSze`) is that same footprint `prime * 12`.
//!
//! # Zero-init deviation (spec §21.5)
//!
//! 0.8.5 allocates the table with plain `malloc` and **no** `memset`
//! (`JHashPos.cpp:65-67`): reading an untouched bucket is C++ UB, and for
//! the multi-MB tables Linux serves zero pages, so the observable oracle
//! behavior is that of a zeroed table. This port zero-initializes both
//! vectors (deterministic, matches the observable behavior); documented in
//! [`JHashPos::get`]'s zero-bucket answer and pinned by the tests.
//!
//! # The hash function (0.8.5)
//!
//! 0.8.5 moved the hash function out of `JHashPos` into `JDiff::hash`
//! (`JDiff.cpp:361-371`), ported as the pure function
//! [`crate::jdiff::hash_key`] which adds the equal-run counter into the
//! value. This module stores and looks up keys only; the 0.8.1
//! `JHashPos::hash` shim and its DBGHSK trace were retired with the Task 17
//! engine rewire (0.8.5 has zero DBGHSK sites, spec §18.G).
//!
//! # Debug prints (spec §14, `debug` feature)
//!
//! The `#if debug` sites are ported with their exact C++ format strings:
//! the constructor's "Hash Ini" line (`JHashPos.cpp:70-76`), the per-store
//! "Hash Add" lines (`JHashPos.cpp:127-132`) and the audit helpers
//! [`JHashPos::print`] / [`JHashPos::dist`] (`JHashPos.cpp:176-238`;
//! `print` has no call site in the C++ — dead-code parity). The "Hash Ini"
//! addresses are printed from the two vectors' allocations like the C++
//! `mzHshTblPos`/`mkHshTblHsh` bounds; as everywhere `%p` values are
//! non-reproducible and only the line shape is pinned.
//!
//! # Example
//!
//! ```
//! use jojodiff_cli_rs::jhashpos::JHashPos;
//!
//! let mut tbl = JHashPos::new(1); // 1 MB: 87381 elements -> prime 87359
//! let mut key = 0u32;
//! let mut old = -1i32;
//! let mut eql = 0i32;
//! for b in b"the quick brown fox" {
//!     key = jojodiff_cli_rs::jdiff::hash_key(key, &mut old, *b as i32, &mut eql);
//! }
//! tbl.add(key, 4242, 0); // store the sample (highest quality)
//!
//! let mut pos = 0i64;
//! assert!(tbl.get(key, &mut pos));
//! assert_eq!(pos, 4242);
//! ```

use crate::defs::{SMPSZE, get_lower_prime};
use crate::jdebug::dbg_print;
#[cfg(feature = "debug")]
use crate::jdebug::{DBGHSH, dbg};

/// Override when the collision counter exceeds this threshold
/// (`JHashPos.cpp:33`).
pub const COLLISION_THRESHOLD: i32 = 4;

/// Rate at which high-quality samples should override (`JHashPos.cpp:34`).
pub const COLLISION_HIGH: i32 = 4;

/// Rate at which low-quality samples should override (`JHashPos.cpp:35`).
pub const COLLISION_LOW: i32 = 1;

/// Hashtable of file positions for JDiff (`JHashPos.h:109`).
///
/// The C++ 0.8.5 allocates one block with plain `malloc` and no `memset`
/// (`JHashPos.cpp:65-67`) — reading an untouched bucket is UB there; see the
/// module doc for the zero-init deviation (spec §21.5). The two zeroed
/// vectors below make the port deterministic while matching the observable
/// oracle behavior (Linux zero pages).
pub struct JHashPos {
    /// Positions within the original file (`mzHshTblPos`).
    tbl_pos: Vec<i64>,
    /// Hash keys (`mkHshTblHsh`).
    tbl_hsh: Vec<u32>,
    /// Prime number for size and hashing (`miHshPme`).
    prime: i32,
    /// Actual size in bytes of the hashtable (`miHshSze`).
    size_bytes: i32,
    /// Max number of collisions before override (`miHshColMax`).
    col_max: i32,
    /// Current number of subsequent collisions (`miHshColCnt`); 0.8.5 counts
    /// this **down** and stores at `<= 0`.
    col_cnt: i32,
    /// Reliability: decreases as the overloading grows (`miHshRlb`).
    rlb: i32,
    /// Load-counter (`miLodCnt`); 0.8.5 starts it at the prime and counts
    /// **down**.
    load_cnt: i32,
    /// Number of hits found by this hashtable (`miHshHit`).
    hits: i32,
}

impl JHashPos {
    /// Create a new hash-table with a size (in **MB**) not larger than the
    /// given size (`JHashPos.cpp:48-82`).
    ///
    /// The MB count converts to an element count with the port variant's
    /// element size 12 — `mb * 1024 * 1024 / 12` (`JHashPos.cpp:58-59`
    /// divides by `sizeof(hkey) + sizeof(off_t)`; u32 hkey + i64 off_t, spec
    /// §21.18: default 32 MB → 2796202 elements) — and the actual prime is
    /// the nearest lower prime ([`get_lower_prime`], `JDefs.cpp:53-67`).
    /// `aiSze < 1` behaves like 1 (`JHashPos.cpp:54-57`).
    ///
    /// Initial state (`JHashPos.cpp:49-50,68`): `col_max = col_cnt =
    /// COLLISION_THRESHOLD` (4), reliability seed `SMPSZE + SMPSZE/2` (48 at
    /// the port's SMPSZE 32, spec §21.9), load counter at the prime, hits 0.
    ///
    /// MB values above 2047 overflow the C++ `int` product
    /// `aiSze*1024*1024` (undefined behavior there); the port computes in
    /// i64 and clamps to `i32::MAX` for determinism.
    pub fn new(mb: i32) -> Self {
        /* get largest prime < elements (JHashPos.cpp:53-61) */
        let sze: i64 = if mb < 1 { 1 } else { i64::from(mb) };
        let elements = (sze * 1024 * 1024 / 12).min(i64::from(i32::MAX)) as i32;
        let prime = get_lower_prime(elements);

        // miHshSze = prime * (sizeof(off_t) + sizeof(hkey)) on the port's
        // 64-bit off_t / 32-bit hkey build = prime * (8 + 4).
        let size_bytes = prime * 12;
        let tbl = JHashPos {
            tbl_pos: vec![0i64; prime as usize],
            tbl_hsh: vec![0u32; prime as usize],
            prime,
            size_bytes,
            col_max: COLLISION_THRESHOLD,
            col_cnt: COLLISION_THRESHOLD,
            rlb: SMPSZE + SMPSZE / 2,
            load_cnt: prime, // miLodCnt = miHshPme (JHashPos.cpp:68)
            hits: 0,
        };

        /* Debug: allocation bounds like the C++ `mzHshTblPos` /
         * `mkHshTblHsh` start/end pointers (JHashPos.cpp:70-76); `%p` values
         * are non-reproducible, only the shape is pinned. */
        #[cfg(feature = "debug")]
        if dbg(DBGHSH) {
            let pos = tbl.tbl_pos.as_ptr_range();
            let hsh = tbl.tbl_hsh.as_ptr_range();
            dbg_print(format_args!(
                "Hash Ini sizeof={:2}+{:2}={:2}, {} samples, {} bytes, address={:p}-{:p},{:p}-{:p}.\n",
                4,  // sizeof(hkey), 32-bit oracle build
                8,  // sizeof(off_t)
                12, // sizeof(hkey) + sizeof(off_t)
                tbl.prime,
                tbl.size_bytes,
                pos.start,
                pos.end,
                hsh.start,
                hsh.end,
            ));
        }

        tbl
    }

    /// Hashtable add (`JHashPos.cpp:99-139`).
    ///
    /// `key`: hash key to add, `pos`: position to add, `eql_cnt`: quality of
    /// the sample (equal-character count; `<= SMPSZE * 2` counts as high
    /// quality — the 0.8.1 gate was `SMPSZE - 4`; the low-quality branch is
    /// unreachable from the engine because the hash caps `eql` at SMPSZE,
    /// ported as written, spec §21.13).
    pub fn add(&mut self, key: u32, pos: i64, eql_cnt: i32) {
        // Every time the load factor increases by 1:
        // - increase col_max: the ratio at which we store values to achieve a
        //   uniform distribution of samples,
        // - increase rlb: the number of bytes to verify (reliability range)
        //   to be sure there is no match.
        // 0.8.5 counts the load down from the prime (`JHashPos.cpp:68,104-110`).
        if self.load_cnt > 0 {
            self.load_cnt -= 1;
        } else {
            self.load_cnt = self.prime;
            self.col_max += COLLISION_THRESHOLD;
            self.rlb += 4; // try to keep a reliability of +/- 99%
        }

        // Increase the collision strategy counter:
        // - HIGH for "good" samples,
        // - LOW for low-quality samples.
        // 0.8.5 counts down (`JHashPos.cpp:116-119`) and stores at `<= 0`.
        if eql_cnt <= SMPSZE * 2 {
            self.col_cnt -= COLLISION_HIGH;
        } else {
            self.col_cnt -= COLLISION_LOW; // reduce overrides by low-quality samples
        }

        // Store key and value when the collision counter reaches the
        // collision threshold.
        if self.col_cnt <= 0 {
            // Calculate the index in the hashtable for the given key.
            let idx = (key % self.prime as u32) as usize;

            /* Debug: per-store trace, before the store like the C++
             * (JHashPos.cpp:127-132); `%c` is `.` for an empty bucket, `!`
             * for an override. */
            #[cfg(feature = "debug")]
            if dbg(DBGHSH) {
                dbg_print(format_args!(
                    "Hash Add {:8} {} {:8x} {}\n",
                    idx as i32,
                    crate::defs::p8(pos),
                    key,
                    if self.tbl_hsh[idx] == 0 { '.' } else { '!' },
                ));
            }

            self.tbl_hsh[idx] = key;
            self.tbl_pos[idx] = pos;
            self.col_cnt = self.col_max; // reset subsequent lost collisions counter
        }
    }

    /// Hashtable reset: consider the table to be empty
    /// (`JHashPos::reset`, `JHashPos.cpp:144-149`).
    ///
    /// The C++ declares this method but never calls it — dead-code parity
    /// (spec §21.13); kept public in matching dead form.
    pub fn reset(&mut self) {
        self.load_cnt = self.prime;
        self.col_max = COLLISION_THRESHOLD;
        self.col_cnt = COLLISION_THRESHOLD;
        self.rlb = SMPSZE + SMPSZE / 2;
    }

    /// Hashtable lookup (`JHashPos.cpp:158-171`): exact-key match at
    /// `key % prime` only; increments the hit counter on match. On a
    /// zero-filled untouched bucket (`key == 0`) this answers `(true, 0)` —
    /// the C++ UB reads zero pages in practice (spec §21.5 deviation).
    pub fn get(&mut self, key: u32, pos: &mut i64) -> bool {
        // Calculate the index in the hashtable for the given key.
        let idx = (key % self.prime as u32) as usize;

        // Lookup value into hashtable for new file.
        if self.tbl_hsh[idx] == key {
            self.hits += 1;
            *pos = self.tbl_pos[idx];
            return true;
        }
        false
    }

    /// Return the reliability range: the estimated number of bytes to verify
    /// before deciding that regions do not match (`JHashPos.h:119-125`).
    pub fn reliability(&self) -> i32 {
        self.rlb
    }

    /// Return the hashtable prime number (`get_hashprime`,
    /// `JHashPos.h:143`).
    pub fn hash_prime(&self) -> i32 {
        self.prime
    }

    /// Return the hashtable size in bytes (`get_hashsize`,
    /// `JHashPos.h:146`): `prime * (sizeof(off_t) + sizeof(hkey))` on the
    /// 64-bit `off_t` build, i.e. `prime * 12`. The verbose-stats label says
    /// "samples" while the value is bytes — replicate (spec §4.4).
    pub fn hash_size_bytes(&self) -> i32 {
        self.size_bytes
    }

    /// Return the hashtable collision override threshold (`get_hashcolmax`,
    /// `JHashPos.h:149`).
    pub fn hash_colmax(&self) -> i32 {
        self.col_max
    }

    /// Return the number of hits found by this hashtable (`get_hashhits`,
    /// `JHashPos.h:152`).
    pub fn hash_hits(&self) -> i32 {
        self.hits
    }

    /// Print the hashtable content (`JHashPos::print`, `JHashPos.cpp:160-172`).
    ///
    /// Debug builds only. The C++ declares this method but never calls it —
    /// dead-code parity, ported for the audit format alone.
    #[cfg(feature = "debug")]
    pub fn print(&self) {
        for idx in 0..self.prime as usize {
            if self.tbl_pos[idx] != 0 {
                dbg_print(format_args!(
                    "Hash Pnt {:12} {}-{:08}x\n",
                    idx as i32,
                    crate::defs::p8(self.tbl_pos[idx]),
                    self.tbl_hsh[idx],
                ));
            }
        }
    }

    /// Print the hashtable distribution over `bck` buckets
    /// (`JHashPos::dist`, `JHashPos.cpp:192-238`); `max` is the largest
    /// position to find. **Not debug-gated**: the 0.8.5 engine calls it from
    /// release-visible verbose>2 sites (`JDiff.cpp:324-327,784-787`,
    /// 10 buckets — the jdiff tail for incremental scanning and the
    /// buildFullIndex tail); the 0.8.1 DBGDST debug call site is gone
    /// (spec §18.G: `-d dst` has zero sites).
    ///
    /// The 0.8.5 quirks are preserved: positions beyond the last bucket are
    /// *not* counted (`liIdx >= aiBck` only assigns `liIdx = 0`, the increment
    /// is in the `else`), `Overload` is `colMax/COLLISION_THRESHOLD - 1`
    /// (`:202`), and the summary lines use the 0.8.5 guarded formulas
    /// `Avg/Min/Max/%` = `liMax > 0 ? 100 - (liMin / (liMax / 100)) : -1`
    /// and `Load` = `miHshPme > 0 ? liCnt / (miHshPme / 100) : -1` — both
    /// print a literal `%`. The guards only cover the zero cases: when
    /// `0 < liMax < 100` the inner `liMax / 100` is 0 and the C++ dies with
    /// SIGFPE (integer division by zero); this port panics on the same
    /// division. (0.8.5's own call sites pass 10 buckets over positions well
    /// above 1000, so the shipped binary never reaches it.)
    pub fn dist(&self, max: i64, bck: i32) {
        dbg_print(format_args!(
            "Hash Dist Overload    = {}\n",
            self.col_max / COLLISION_THRESHOLD - 1
        ));
        dbg_print(format_args!("Hash Dist Reliability = {}\n", self.rlb));

        // Bucket counters (the C++ mallocs aiBck ints and memsets them).
        let mut bck_cnt = vec![0i32; bck as usize];

        // Fill the buckets (JHashPos.cpp:210-221).
        let div = (max / i64::from(bck)) as i32;
        for idx in 0..self.prime as usize {
            if self.tbl_pos[idx] > 0 && self.tbl_pos[idx] <= max {
                let b = (self.tbl_pos[idx] / i64::from(div)) as i32;
                if b < bck {
                    bck_cnt[b as usize] += 1;
                }
                // else C++ assigns liIdx = 0 — and does not count (the
                // increment lives in the else arm).
            }
        }

        // Printout (JHashPos.cpp:224-236).
        let mut sum: i32 = 0;
        let mut min = i32::MAX;
        let mut max_cnt: i32 = 0;
        for (b, &cnt) in bck_cnt.iter().enumerate() {
            sum += cnt;
            if cnt < min {
                min = cnt;
            }
            if cnt > max_cnt {
                max_cnt = cnt;
            }
            dbg_print(format_args!(
                "Hash Dist {:8} Pos={}:{} Cnt={:8} Rlb={}\n",
                b as i32,
                crate::defs::p8(b as i64 * i64::from(div)),
                crate::defs::p8((b + 1) as i64 * i64::from(div)),
                cnt,
                if cnt == 0 { -1 } else { div / cnt },
            ));
        }
        dbg_print(format_args!(
            "Hash Dist Avg/Min/Max/% = {}/{}/{}/{}%\n",
            sum / bck,
            min,
            max_cnt,
            if max_cnt > 0 {
                100 - (min / (max_cnt / 100))
            } else {
                -1
            },
        ));
        dbg_print(format_args!(
            "Hash Dist Load          = {}/{}={}%\n",
            sum,
            self.prime,
            if self.prime > 0 {
                sum / (self.prime / 100)
            } else {
                -1
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    /// MB ctor → elements `mb*1024*1024/12` → [`get_lower_prime`] (spec
    /// §21.18: `JHashPos.cpp:53-61` with the port variant's element size
    /// `sizeof(hkey)+sizeof(off_t) = 4+8 = 12`); `aiSze < 1` behaves like 1
    /// (`JHashPos.cpp:54-57`).
    #[test]
    fn prime_selection_mb_ctor() {
        assert_eq!(JHashPos::new(32).hash_prime(), 2796181); // 0.8.5 default MB
        assert_eq!(JHashPos::new(8).hash_prime(), 699037);
        assert_eq!(JHashPos::new(2).hash_prime(), 174761);
        assert_eq!(JHashPos::new(1).hash_prime(), 87359); // floor at mb >= 1
        assert_eq!(JHashPos::new(0).hash_prime(), 87359); // aiSze < 1 -> 1 MB
        assert_eq!(JHashPos::new(-3).hash_prime(), 87359);
        // Element counts on get_lower_prime's default branch (downward
        // search): 128 MB -> 11184810 elements, 256 MB -> 22369621. (The
        // exact switch cases need MB multiples of 12 — 384 MB → 32M elements
        // etc. — too large to allocate here.)
        assert_eq!(JHashPos::new(128).hash_prime(), 11184799);
        assert_eq!(JHashPos::new(256).hash_prime(), 22369601);
        // Size in bytes stays prime * 12 (port hkey u32 + off_t i64).
        assert_eq!(JHashPos::new(32).hash_size_bytes(), 33554172);
        assert_eq!(JHashPos::new(8).hash_size_bytes(), 8388444);
    }

    /// The table is zero-initialized (spec §21.5 deviation: 0.8.5 `malloc`s
    /// without `memset`, Rust zero-fills for determinism): an untouched
    /// bucket answers `get(0)` with `(true, 0)`.
    #[test]
    fn zero_initialized_table() {
        let mut tbl = JHashPos::new(1);
        let mut pos = -1i64;
        assert!(tbl.get(0, &mut pos));
        assert_eq!(pos, 0);
    }

    /// Down-counting collision counter (`JHashPos.cpp:49,116-137`): col_cnt
    /// starts at col_max 4, a high-quality add decrements by COLLISION_HIGH
    /// and stores at `<= 0`, resetting col_cnt to col_max — so with col_max
    /// 4 the first high-quality add stores immediately and every further
    /// high-quality add stores as well.
    #[test]
    fn down_counter_store_cadence() {
        let mut tbl = JHashPos::new(1);
        let mut pos = 0i64;

        tbl.add(10, 100, 0); // 4 - 4 = 0 <= 0: stored, col_cnt reset to 4
        assert!(tbl.get(10, &mut pos));
        assert_eq!(pos, 100);

        // Overwrite semantics on the same bucket (87369 % 87359 == 10): the
        // exact-key lookup answers the new key only.
        tbl.add(87369, 200, 0);
        assert!(tbl.get(87369, &mut pos));
        assert_eq!(pos, 200);
        assert!(!tbl.get(10, &mut pos)); // key 10 was overwritten

        // Quality gate at SMPSZE * 2 = 64 (`JHashPos.cpp:116`): eql_cnt 64
        // is still high quality (the 0.8.1 gate was SMPSZE - 4 = 28).
        let mut hi = JHashPos::new(1);
        hi.add(10, 100, 64);
        assert!(hi.get(10, &mut pos));
        assert_eq!(pos, 100);

        // A low-quality add (eql_cnt 65 > 64) decrements by COLLISION_LOW 1:
        // on a fresh table 4 - 1 = 3 > 0, so it does NOT store (0.8.1's
        // up-counter stored the first low-quality add: 4 + 1 = 5 >= 4). Four
        // low-quality adds reach 0: 3, 2, 1, 0.
        let mut lo = JHashPos::new(1);
        lo.add(10, 100, 65);
        assert!(!lo.get(10, &mut pos));
        lo.add(10, 101, 65);
        assert!(!lo.get(10, &mut pos));
        lo.add(10, 102, 65);
        assert!(!lo.get(10, &mut pos));
        lo.add(10, 103, 65); // 0 <= 0: stored
        assert!(lo.get(10, &mut pos));
        assert_eq!(pos, 103);
    }

    /// Down-counting load counter (`JHashPos.cpp:68,104-110`): load_cnt
    /// starts at the prime, counts down, and the rollover add resets it to
    /// the prime while raising col_max and rlb by 4.
    #[test]
    fn load_rollover_counts_down() {
        let mut tbl = JHashPos::new(1);
        assert_eq!(tbl.reliability(), SMPSZE + SMPSZE / 2); // seed 48
        assert_eq!(tbl.hash_colmax(), 4);

        // The first 87359 adds (prime 87359) take load_cnt down to 0 without
        // rolling over.
        for i in 0..87359u32 {
            tbl.add(i, i64::from(i), 0);
        }
        assert_eq!(tbl.hash_colmax(), 4);
        assert_eq!(tbl.reliability(), 48);

        // The 87360th add finds load_cnt 0: resets it to the prime and does
        // col_max += 4, rlb += 4.
        tbl.add(87359, 87359, 0);
        assert_eq!(tbl.hash_colmax(), 8);
        assert_eq!(tbl.reliability(), 52);

        // At col_max 8 the high-quality cadence is every other add: the
        // store reset refills col_cnt to 8, the next add decrements to
        // 4 (> 0, lost), the one after to 0 (stored). Keys 90000/90001
        // land on buckets the loop above stored under other keys (2641/
        // 2642) — the exact-key lookup still answers per the last store.
        let mut pos = 0i64;
        tbl.add(90000, 1, 0); // 8 - 4 = 4 > 0: lost
        assert!(!tbl.get(90000, &mut pos));
        tbl.add(90001, 2, 0); // 4 - 4 = 0: stored
        assert!(tbl.get(90001, &mut pos));
        assert_eq!(pos, 2);
    }

    /// Adds 32-byte-window keys (as the diff engine samples files), then
    /// verifies the stored positions, the hit counter, and misses for
    /// overwritten and absent keys.
    #[test]
    fn add_then_get_roundtrip_and_hits() {
        const PRIME: u32 = 87359;
        let mut tbl = JHashPos::new(1);
        let mut pos = -1i64;

        // The table is zero-initialized (spec §21.5): an untouched bucket
        // answers get(0) with (true, 0). Replicated 1:1.
        assert!(tbl.get(0, &mut pos));
        assert_eq!(pos, 0);

        // Deterministic pseudo-random bytes (LCG, bits 8..=15 so the byte
        // period exceeds the sampled range) such that all 1000 window keys
        // are distinct.
        let mut seed = 1u32;
        let data: Vec<u8> = (0..1032)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                (seed >> 8) as u8
            })
            .collect();

        // Sample 32-byte windows over the data, as the diff engine will:
        // window i gets the incremental hash of data[i..i + 32] (0.8.5
        // hash: the equal-run counter rides in the key).
        let mut keys = Vec::with_capacity(1000);
        let mut key = 0u32;
        let mut old = -1i32;
        let mut eql = 0i32;
        let mut win: std::collections::VecDeque<i32> = std::collections::VecDeque::new();
        for (i, &b) in data.iter().enumerate() {
            key = crate::jdiff::hash_key(key, &mut old, i32::from(b), &mut eql);
            win.push_back(i32::from(b));
            if win.len() > 32 {
                win.pop_front();
                let wp = (i - 31) as i64; // window start position
                tbl.add(key, wp, 0); // highest quality
                keys.push((key, wp));
            }
        }

        // 0.8.5 oracle (`JHashPos.cpp:99-138`): col_cnt starts at col_max 4,
        // every high-quality add decrements to 0 and stores, resetting
        // col_cnt to col_max; with 1000 adds there is no load rollover
        // (prime 87359), so every add stores into bucket key % prime.
        let mut oracle_key = [0u32; PRIME as usize];
        let mut oracle_pos = [0i64; PRIME as usize];
        for &(k, wp) in &keys {
            let idx = (k % PRIME) as usize;
            oracle_key[idx] = k;
            oracle_pos[idx] = wp;
        }
        let occupied = keys
            .iter()
            .map(|&(k, _)| k % PRIME)
            .collect::<HashSet<_>>()
            .len();
        assert!(occupied < keys.len()); // overwrites really occurred

        // Every lookup answers exactly per the oracle, and the hit counter
        // matches (including the get(0) above).
        let mut hits = 1;
        for &(k, wp) in &keys {
            let idx = (k % PRIME) as usize;
            let found = tbl.get(k, &mut pos);
            assert_eq!(found, k == oracle_key[idx], "key {k} bucket {idx}");
            if found {
                assert_eq!(pos, oracle_pos[idx], "key {k} bucket {idx}");
                assert_eq!(pos, wp);
                hits += 1;
            }
        }
        assert_eq!(tbl.hash_hits(), hits);
        assert!(hits < keys.len() as i32); // misses really occurred

        // A key that was never stored answers false.
        assert!(!tbl.get(u32::MAX, &mut pos));
    }

    /// The 0.8.5 rolling key (`JDiff::hash`, `JDiff.cpp:361-371`): with eql
    /// pinned at 0 (alternating bytes) the value reduces to the pure
    /// `*2 + byte` accumulation wrapping on u32.
    #[test]
    fn rolling_key_wraps_u32() {
        let mut h = 0u32;
        let mut old = -1i32;
        let mut eql = 0i32;
        let mut k = 0u32;
        for b in 0u32..300 {
            let byte = (b % 2) as i32; // alternating: eql stays 0
            h = crate::jdiff::hash_key(h, &mut old, byte, &mut eql);
            k = k.wrapping_mul(2).wrapping_add(byte as u32);
        }
        assert_eq!(h, k);
        assert_eq!(eql, 0);
    }
}
