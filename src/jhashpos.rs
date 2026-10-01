//! Sample hash table mapping 32-bit hash keys to 64-bit file positions,
//! ported 1:1 from C++ `src/JHashPos.cpp` + `headers/JHashPos.h` (spec §7).
//!
//! Only samples from the original file are stored; samples from the new file
//! are looked up. There is one slot per bucket: `add` overwrites the bucket
//! when the collision-credit counter reaches the current threshold, and `get`
//! is an exact-key match at `key % prime` — there is no probing.
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
        JHashPos {
            tbl_pos: vec![0i64; prime as usize],
            tbl_hsh: vec![0u32; prime as usize],
            prime,
            size_bytes,
            col_max: COLLISION_THRESHOLD,
            col_cnt: COLLISION_THRESHOLD,
            rlb: 48,
            load_cnt: 0,
            hits: 0,
        }
    }

    /// The hash function: generate a new hash value by adding a new byte
    /// (`JHashPos.h:109-117`). Old bytes are shifted out in such a way that
    /// the value corresponds to a sample of 32 bytes; the u32 arithmetic
    /// wraps exactly like the 32-bit C++ `hkey` of the oracle build.
    pub fn hash(&self, byte: i32, cur: &mut u32) {
        *cur = cur.wrapping_mul(2).wrapping_add(byte as u32);
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
