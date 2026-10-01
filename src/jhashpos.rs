//! Sample hash table mapping 32-bit hash keys to 64-bit file positions,
//! ported 1:1 from C++ `src/JHashPos.cpp` + `headers/JHashPos.h` (spec §7).
//!
//! Only samples from the original file are stored; samples from the new file
//! are looked up. There is one slot per bucket: `add` overwrites the bucket
//! when the collision-credit counter reaches the current threshold, and `get`
//! is an exact-key match at `key % prime` — there is no probing.
//!
//! # Debug prints (spec §14, `debug` feature)
//!
//! The `#if debug` sites are ported with their exact C++ format strings:
//! the constructor's "Hash Ini" line (`JHashPos.cpp:66-72`), the per-store
//! "Hash Add" lines (`JHashPos.cpp:124-130`), the per-hash "Hash Key" lines
//! (`JHashPos.h:111-116`) and the audit helpers [`JHashPos::print`] /
//! [`JHashPos::dist`] (`JHashPos.cpp:163-223`; `print` has no call site in
//! the C++ — dead-code parity). The "Hash Ini" addresses are printed from the
//! two vectors' allocations like the C++ `mzHshTblPos`/`mkHshTblHsh` bounds;
//! as everywhere `%p` values are non-reproducible and only the line shape is
//! pinned.
//!
//! # Example
//!
//! ```
//! use jojodiff_cli_rs::jhashpos::JHashPos;
//!
//! let mut tbl = JHashPos::new(65536);
//! let mut key = 0u32;
//! for b in b"the quick brown fox" {
//!     tbl.hash(*b as i32, &mut key);
//! }
//! tbl.add(key, 4242, 0); // store the sample (highest quality)
//!
//! let mut pos = 0i64;
//! assert!(tbl.get(key, &mut pos));
//! assert_eq!(pos, 4242);
//! ```

use crate::defs::{GIPME, SMPSZE};
#[cfg(feature = "debug")]
use crate::jdebug::{DBGHSH, DBGHSK, c_chr, dbg, dbg_print};

/// Override when the collision counter exceeds this threshold
/// (`JHashPos.cpp:33`).
pub const COLLISION_THRESHOLD: i32 = 4;

/// Rate at which high-quality samples should override (`JHashPos.cpp:34`).
pub const COLLISION_HIGH: i32 = 4;

/// Rate at which low-quality samples should override (`JHashPos.cpp:35`).
pub const COLLISION_LOW: i32 = 1;

/// Hashtable of file positions for JDiff (`JHashPos.h:88`).
///
/// The C++ original allocates one block holding the `off_t` and `hkey` arrays
/// and zero-initializes it with `memset` (`JHashPos.cpp:63-78`); an untouched
/// bucket therefore answers `get(0)` with `(true, 0)`. The two zeroed vectors
/// below replicate this exactly.
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
    /// Current number of subsequent collisions (`miHshColCnt`).
    col_cnt: i32,
    /// Reliability: decreases as the overloading grows (`miHshRlb`).
    rlb: i32,
    /// Load-counter (`miLodCnt`).
    load_cnt: i32,
    /// Number of hits found by this hashtable (`miHshHit`).
    hits: i32,
}

impl JHashPos {
    /// Create a new hash-table with size not larger than the given size
    /// (`JHashPos.cpp:45-61`).
    ///
    /// The actual size is based on the highest prime below the highest power
    /// of 2 lower or equal to the specified size, e.g. `8192` creates a
    /// hashtable of 8191 elements.
    pub fn new(requested: i32) -> Self {
        let mut idx = 0usize;
        while idx < 19 && GIPME[idx] > requested {
            idx += 1;
        }
        let prime = GIPME[idx];
        // miHshSze = prime * (sizeof(off_t) + sizeof(hkey)) on the 64-bit
        // off_t / 32-bit hkey build = prime * (8 + 4).
        let size_bytes = prime * 12;
        let tbl = JHashPos {
            tbl_pos: vec![0i64; prime as usize],
            tbl_hsh: vec![0u32; prime as usize],
            prime,
            size_bytes,
            col_max: COLLISION_THRESHOLD,
            col_cnt: COLLISION_THRESHOLD,
            rlb: 48,
            load_cnt: 0,
            hits: 0,
        };

        /* Debug: allocation bounds like the C++ `mzHshTblPos` /
         * `mkHshTblHsh` start/end pointers (JHashPos.cpp:66-72); `%p` values
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

    /// The hash function: generate a new hash value by adding a new byte
    /// (`JHashPos.h:109-117`). Old bytes are shifted out in such a way that
    /// the value corresponds to a sample of 32 bytes; the u32 arithmetic
    /// wraps exactly like the 32-bit C++ `hkey` of the oracle build.
    pub fn hash(&self, byte: i32, cur: &mut u32) {
        *cur = cur.wrapping_mul(2).wrapping_add(byte as u32);

        /* Debug: hash-function trace (JHashPos.h:111-116). */
        #[cfg(feature = "debug")]
        if dbg(DBGHSK) {
            dbg_print(format_args!(
                "Hash Key {:x} {:x} {}\n",
                cur,
                byte as u32,
                c_chr(byte)
            ));
        }
    }

    /// Hashtable add (`JHashPos.cpp:96-137`).
    ///
    /// `key`: hash key to add, `pos`: position to add, `eql_cnt`: quality of
    /// the sample (equal-character count; `<= SMPSZE - 4` counts as high
    /// quality).
    pub fn add(&mut self, key: u32, pos: i64, eql_cnt: i32) {
        // Every time the load factor increases by 1:
        // - increase col_max: the ratio at which we store values to achieve a
        //   uniform distribution of samples,
        // - increase rlb: the number of bytes to verify (reliability range)
        //   to be sure there is no match.
        if self.load_cnt < self.prime {
            self.load_cnt += 1;
        } else {
            self.load_cnt = 0;
            self.col_max += COLLISION_THRESHOLD;
            self.rlb += 4; // try to keep a reliability of +/- 99%
        }

        // Increase the collision strategy counter:
        // - HIGH for "good" samples,
        // - LOW for low-quality samples.
        if eql_cnt <= SMPSZE - 4 {
            self.col_cnt += COLLISION_HIGH;
        } else {
            self.col_cnt += COLLISION_LOW; // reduce overrides by low-quality samples
        }

        // Store key and value when the collision counter reaches the
        // collision threshold.
        if self.col_cnt >= self.col_max {
            // Calculate the index in the hashtable for the given key.
            let idx = (key % self.prime as u32) as usize;

            /* Debug: per-store trace, before the store like the C++
             * (JHashPos.cpp:124-130); `%c` is `.` for an empty bucket, `!`
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
            self.col_cnt = 0; // reset subsequent lost collisions counter
        }
    }

    /// Hashtable lookup (`JHashPos.cpp:145-158`): exact-key match at
    /// `key % prime` only; increments the hit counter on match.
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
    /// (`JHashPos::dist`, `JHashPos.cpp:176-223`); `max` is the largest
    /// position to find. Debug builds only; called from the prescan under
    /// DBGDST (`JDiff.cpp:574-577`).
    ///
    /// The C++ quirks are preserved: positions beyond the last bucket are
    /// *not* counted (`liIdx >= aiBck` only assigns `liIdx = 0`, the increment
    /// is in the `else`), and the `Avg/Min/Max` line divides by `liMax`
    /// without a zero check — on an empty distribution the C++ dies with
    /// SIGFPE, this port panics on the same division. (For files smaller than
    /// the bucket count the divisor `liHshDiv` is 0 as well; the prescan of
    /// such files stores nothing, so the fill loop never divides.)
    #[cfg(feature = "debug")]
    pub fn dist(&self, max: i64, bck: i32) {
        dbg_print(format_args!(
            "Hash Dist Overload    = {}\n",
            self.col_max / 3
        ));
        dbg_print(format_args!("Hash Dist Reliability = {}\n", self.rlb));

        // Bucket counters (the C++ mallocs aiBck ints and memsets them).
        let mut bck_cnt = vec![0i32; bck as usize];

        // Fill the buckets (JHashPos.cpp:195-209).
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

        // Printout (JHashPos.cpp:212-222).
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
            "Hash Dist Avg/Min/Max/% = {}/{}/{}/{}\n",
            sum / bck,
            min,
            max_cnt,
            100 - (min * 100 / max_cnt),
        ));
        dbg_print(format_args!(
            "Hash Dist Load           = {}/{}={}\n",
            sum,
            self.prime,
            i64::from(sum) * 100 / i64::from(self.prime)
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prime selection loop (`JHashPos.cpp:58-61` with loop bound 19).
    #[test]
    fn prime_selection() {
        assert_eq!(JHashPos::new(8 * 1024 * 1024).hash_prime(), 8388593);
        assert_eq!(JHashPos::new(8388593).hash_prime(), 8388593);
        assert_eq!(JHashPos::new(8388592).hash_prime(), 4194301);
        assert_eq!(JHashPos::new(1).hash_prime(), 251); // floor
        assert_eq!(JHashPos::new(0).hash_prime(), 251);
        assert_eq!(JHashPos::new(i32::MAX).hash_prime(), 134217689); // ceiling
    }

    /// `hash` multiplies by 2 and adds the byte, wrapping on u32
    /// (`JHashPos.h:111`).
    #[test]
    fn hash_wraps_u32() {
        let tbl = JHashPos::new(251);
        let mut h = 0u32;
        let mut k = 0u32;
        for b in 0u32..300 {
            tbl.hash(b as i32, &mut h);
            k = k.wrapping_mul(2).wrapping_add(b);
        }
        assert_eq!(h, k);
    }

    /// Adds 32-byte-window keys (as the diff engine samples files), then
    /// verifies the stored positions, the hit counter, and misses for
    /// overwritten and absent keys.
    #[test]
    fn add_then_get_roundtrip_and_hits() {
        const PRIME: u32 = 251;
        let mut tbl = JHashPos::new(PRIME as i32);
        let mut pos = -1i64;

        // The C++ table is zero-initialized (memset, `JHashPos.cpp:78`): an
        // untouched bucket answers get(0) with (true, 0). Replicated 1:1.
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
        // window i gets the incremental hash of data[i..i + 32].
        let mut keys = Vec::with_capacity(1000);
        for i in 0..1000i64 {
            let mut key = 0u32;
            for &b in &data[i as usize..i as usize + 32] {
                tbl.hash(i32::from(b), &mut key);
            }
            tbl.add(key, i, 0); // highest quality
            keys.push(key);
        }

        // Spec-derived oracle (spec §7 = `JHashPos.cpp:96-137`): every add
        // counts toward the load (rollover every 252nd add raises col_max by
        // 4), a high-quality add adds 4 collision credit, and a store
        // overwrites bucket key % 251 when the credit reaches col_max.
        let mut col_max = 4;
        let mut col_cnt = 4;
        let mut load_cnt = 0;
        let mut oracle_key = [0u32; PRIME as usize];
        let mut oracle_pos = [0i64; PRIME as usize];
        let mut stores = 0usize;
        for (p, &k) in keys.iter().enumerate() {
            if load_cnt < PRIME as i32 {
                load_cnt += 1;
            } else {
                load_cnt = 0;
                col_max += 4;
            }
            col_cnt += 4; // eql_cnt = 0 <= SMPSZE - 4
            if col_cnt >= col_max {
                let idx = (k % PRIME) as usize;
                oracle_key[idx] = k;
                oracle_pos[idx] = p as i64;
                col_cnt = 0;
                stores += 1;
            }
        }
        assert!(stores < 1000); // overwrites really occurred

        // Every lookup answers exactly per the oracle, and the hit counter
        // matches (including the get(0) above).
        let mut hits = 1;
        for &k in &keys {
            let idx = (k % PRIME) as usize;
            let found = tbl.get(k, &mut pos);
            assert_eq!(found, k == oracle_key[idx], "key {k} bucket {idx}");
            if found {
                assert_eq!(pos, oracle_pos[idx], "key {k} bucket {idx}");
                hits += 1;
            }
        }
        assert_eq!(tbl.hash_hits(), hits);
        assert!(hits < 1000); // misses really occurred

        // A key that was never stored answers false.
        assert!(!tbl.get(u32::MAX, &mut pos));
    }

    /// Quality credit (`eql_cnt <= 28 ? 4 : 1`), overwrite semantics, and the
    /// load-counter rollover (`JHashPos.cpp:101-135`).
    #[test]
    fn quality_and_load_counters() {
        let mut tbl = JHashPos::new(251);
        let mut pos = 0i64;

        // High-quality sample (eql_cnt <= 28): initial col_cnt 4 + 4 = 8
        // >= col_max 4, so it stores on the first add.
        tbl.add(10, 100, 0);
        assert!(tbl.get(10, &mut pos));
        assert_eq!(pos, 100);

        // Overwrite semantics on the same bucket with a different key:
        // 261 % 251 == 10, but the exact-key lookup only answers key 261
        // while bucket 10 holds it.
        tbl.add(261, 200, 0);
        assert!(tbl.get(261, &mut pos));
        assert_eq!(pos, 200);
        assert!(!tbl.get(10, &mut pos)); // key 10 was overwritten

        // Low-quality sample (eql_cnt = 32 > 28): +1 credit only. On a fresh
        // table the initial col_cnt 4 gives 4 + 1 = 5 >= 4, so the first
        // low-quality add stores too; col_cnt then resets and three further
        // low-quality adds are lost until the fourth one reaches col_max 4.
        let mut lo = JHashPos::new(251);
        lo.add(261, 300, 32); // 5 >= 4: stored
        assert!(lo.get(261, &mut pos));
        assert_eq!(pos, 300);
        lo.add(261, 301, 32); // 1 < 4: lost
        lo.add(261, 302, 32); // 2 < 4: lost
        lo.add(261, 303, 32); // 3 < 4: lost
        assert!(lo.get(261, &mut pos));
        assert_eq!(pos, 300);
        lo.add(261, 304, 32); // 4 >= 4: stored
        assert!(lo.get(261, &mut pos));
        assert_eq!(pos, 304);

        // Load-counter rollover: every add counts, stored or not. After 251
        // adds load_cnt equals prime 251; the next add (the 252nd) resets it
        // and raises col_max 4 -> 8 and reliability 48 -> 52.
        let mut roll = JHashPos::new(251);
        for i in 0u32..251 {
            roll.add(i, i64::from(i), 32); // mostly lost, still counted
        }
        assert_eq!(roll.hash_colmax(), 4);
        assert_eq!(roll.reliability(), 48);
        roll.add(0, 0, 32); // 252nd add: rollover
        assert_eq!(roll.hash_colmax(), 8);
        assert_eq!(roll.reliability(), 52);
    }

    /// Initial reliability 48 (`JHashPos.cpp:55-56`), growing by 4 on every
    /// load-counter rollover (`JHashPos.cpp:106`); with prime 251 the
    /// rollover fires on the 252nd, 504th, ... add.
    #[test]
    fn reliability_grows_by_4() {
        let mut tbl = JHashPos::new(251);
        assert_eq!(tbl.reliability(), 48);
        assert_eq!(tbl.hash_colmax(), 4);

        for i in 0u32..252 {
            tbl.add(i, i64::from(i), 0);
        }
        assert_eq!(tbl.reliability(), 52);
        assert_eq!(tbl.hash_colmax(), 8);

        for i in 252u32..504 {
            tbl.add(i, i64::from(i), 0);
        }
        assert_eq!(tbl.reliability(), 56);
        assert_eq!(tbl.hash_colmax(), 12);
    }
}
