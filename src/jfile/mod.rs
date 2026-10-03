//! `JFile` abstraction layer: the file byte-source interface used by the diff
//! and patch engines (C++ `JFile`/`JFile.cpp` at 0.8.5; spec §18.E, §21.11).
//!
//! The C++ base class carries the sequential fast-path state
//! (`mzPosRed/miRedSze/mpRed`, `JFile.h:177-181`) and dispatches `get` through
//! it (`JFile.h:73-81`); Rust traits cannot hold state, so the fast path lives
//! inside each implementation and the trait carries the interface.

pub mod ahead;
pub mod mem;

pub use crate::defs::ReadType;
pub use ahead::JFileAhead;
pub use mem::JFileMem;

/// The result of a [`JFile::get`] read: a byte, end of input, the
/// soft-ahead end-of-buffer, or an error. Replaces the raw `i32` channel
/// (`0..=255` / `EOF` / `EOB` / `EXI_SEK` / `EXI_RED`) — same information,
/// typed.
///
/// Only `Debug` is derived: the [`ByteOrEof::Err`] payload ends in
/// `std::io::Error`, which is neither `Clone`/`Copy` nor `PartialEq`/`Eq`,
/// so those traits are provided manually below instead.
#[derive(Debug)]
#[non_exhaustive] // API-07; crate-internal matches stay exhaustive
pub enum ByteOrEof {
    /// A data byte.
    Byte(u8),
    /// `EOF` — end of input, or a gated negative position (§21.17).
    Eof,
    /// `EOB` — soft look-ahead past the buffer window (soft-ahead only).
    Eob,
    /// A read-side error (`EXI_SEK`/`EXI_RED`).
    Err(crate::error::JDiffError),
}

/// Structural equality: `Err` payloads compare by their engine code
/// (`std::io::Error` is not `PartialEq`, so [`JDiffError`] cannot derive it;
/// the `EXI_*` codes are pairwise distinct, so the discriminant determines
/// equality exactly).
impl PartialEq for ByteOrEof {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ByteOrEof::Byte(a), ByteOrEof::Byte(b)) => a == b,
            (ByteOrEof::Eof, ByteOrEof::Eof) | (ByteOrEof::Eob, ByteOrEof::Eob) => true,
            (ByteOrEof::Err(a), ByteOrEof::Err(b)) => a.exit_code() == b.exit_code(),
            _ => false,
        }
    }
}

impl Eq for ByteOrEof {}

impl ByteOrEof {
    /// Reconstructs the legacy `i32` channel value. Boundary use only
    /// (`pch_get`, `JOut::put` detail args); semantic branches should
    /// `match` instead.
    pub(crate) fn to_i32(&self) -> i32 {
        match self {
            ByteOrEof::Byte(b) => i32::from(*b),
            ByteOrEof::Eof => crate::defs::EOF,
            ByteOrEof::Eob => crate::defs::EOB,
            ByteOrEof::Err(e) => e.exit_code(),
        }
    }

    /// Maps a legacy `get` return onto the typed form.
    pub(crate) fn from_raw(v: i32) -> ByteOrEof {
        match v {
            0..=255 => ByteOrEof::Byte(v as u8),
            x if x == crate::defs::EOF => ByteOrEof::Eof,
            x if x == crate::defs::EOB => ByteOrEof::Eob,
            x if x == crate::defs::EXI_SEK => ByteOrEof::Err(crate::error::JDiffError::Seek),
            x if x == crate::defs::EXI_RED => ByteOrEof::Err(crate::error::JDiffError::Read),
            other => unreachable!("legacy get channel held {other}"),
        }
    }
}

/// Byte-source abstraction for the diff and patch engines, 1:1 with the C++
/// `JFile` interface (`src/JFile.h`).
///
/// `get` returns the requested byte, or the typed form of the C `i32`
/// sentinels: [`ByteOrEof::Eof`] at end of input, [`ByteOrEof::Eob`] for
/// look-ahead reads past the buffer window (spec §10; the in-memory reader
/// never returns it) and [`ByteOrEof::Err`] for the raw negative `EXI_*`
/// error sentinels (since 0.8.5, `JFileAhead.h:115`: `SeekError = EXI_SEK`,
/// `ReadError = EXI_RED`).
pub trait JFile {
    /// Returns the byte at `pos`, or the typed sentinel: [`ByteOrEof::Eof`]
    /// when `pos` is negative or at/past the end of the file,
    /// [`ByteOrEof::Eob`] (`EOF - 1`) for soft-ahead reads past the buffer
    /// window and [`ByteOrEof::Err`] for the read-side `EXI_SEK`/`EXI_RED`
    /// errors. `typ` distinguishes plain reads from hard/soft look-ahead;
    /// see [`ReadType`].
    fn get(&mut self, pos: i64, typ: ReadType) -> ByteOrEof;

    /// Number of seek operations performed (C++ `seekcount`, `JFile.h:114`).
    fn seekcount(&self) -> i64;

    /// Sets the soft look-ahead base: soft reads will fail with
    /// [`crate::defs::EOB`] when reading after `base + buffer size`
    /// (C++ pure virtual, `JFile.h:102-104`).
    fn set_lookahead_base(&mut self, base: i64);

    /// Whether this file is sequential (non-seekable). Auto-detected at
    /// construction by the seek-EOF probe (`chkSeq`, `JFile.cpp:37-46`).
    fn is_sequential(&self) -> bool;

    /// Seek-EOF abstraction (C++ pure virtual, `JFile.h:161`): returns the
    /// EOF position (>= 0) or [`crate::defs::EXI_SEK`] on error. Protected in
    /// C++; a required trait method here because the trait cannot hold the
    /// `mbSeq`/`mzPosEof` state `chkSeq` maintains.
    fn jeofpos(&mut self) -> i64;

    /// First position in the buffer, `-1` = no buffering (C++ virtual with a
    /// `-1` default, `JFile.h:126`).
    fn get_buf_pos(&self) -> i64 {
        -1
    }

    /// Get access to the (fast) buffered read (C++ virtual with a null
    /// default, `JFile.h:144-146`).
    ///
    /// On success returns the buffer run starting at `pos`; its length is the
    /// number of available bytes (the run never wraps the ring). On failure
    /// (EOF, EOB or an `EXI_*` error, `JFileAhead.cpp:222-225`) returns
    /// `None`. The null default, like the C++ base implementation, leaves the
    /// caller to the per-byte fallback.
    fn getbuf(&mut self, _pos: i64, _typ: ReadType) -> Option<&[u8]> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{EOB, EOF, EXI_RED, EXI_SEK};
    use crate::error::JDiffError;

    /// `from_raw` maps every legacy channel value onto the typed form: the
    /// byte range (`0`, `255`), `EOF`, `EOB`, and the two read-side error
    /// codes (`EXI_SEK`/`EXI_RED`, `JFileAhead.h:115`).
    #[test]
    fn from_raw_maps_all_channels() {
        assert_eq!(ByteOrEof::from_raw(0), ByteOrEof::Byte(0));
        assert_eq!(ByteOrEof::from_raw(255), ByteOrEof::Byte(255));
        assert_eq!(ByteOrEof::from_raw(EOF), ByteOrEof::Eof);
        assert_eq!(ByteOrEof::from_raw(EOB), ByteOrEof::Eob);
        assert_eq!(
            ByteOrEof::from_raw(EXI_SEK),
            ByteOrEof::Err(JDiffError::Seek)
        );
        assert_eq!(
            ByteOrEof::from_raw(EXI_RED),
            ByteOrEof::Err(JDiffError::Read)
        );
    }

    /// `to_i32` round-trips every typed channel back to its legacy `i32`
    /// value, including the error codes.
    #[test]
    fn to_i32_round_trips_all_channels() {
        assert_eq!(ByteOrEof::Byte(0).to_i32(), 0);
        assert_eq!(ByteOrEof::Byte(255).to_i32(), 255);
        assert_eq!(ByteOrEof::Eof.to_i32(), EOF);
        assert_eq!(ByteOrEof::Eob.to_i32(), EOB);
        assert_eq!(ByteOrEof::Err(JDiffError::Seek).to_i32(), EXI_SEK);
        assert_eq!(ByteOrEof::Err(JDiffError::Read).to_i32(), EXI_RED);
    }

    /// `get` on `JFileMem` is fully typed: data bytes arrive as `Byte`, end
    /// of input and negative positions as `Eof` — the §21.17
    /// negative-position → EOF gate, with expectations copied from the
    /// `mem.rs` tests (`seq_nul_eof_seekcount`,
    /// `eob_never_returned_eof_repeats`). `JFileMem` never returns `Eob`.
    #[test]
    fn get_on_mem_typed_channels() {
        let mut f = JFileMem::new(vec![0u8, 1, 0, 3, 0xA7]);
        assert_eq!(f.get(0, ReadType::Read), ByteOrEof::Byte(0));
        assert_eq!(f.get(1, ReadType::Read), ByteOrEof::Byte(1));
        assert_eq!(f.get(2, ReadType::SoftAhead), ByteOrEof::Byte(0));
        assert_eq!(f.get(4, ReadType::Read), ByteOrEof::Byte(0xA7));
        assert_eq!(f.get(5, ReadType::Read), ByteOrEof::Eof); // pos == len
        assert_eq!(f.get(-1, ReadType::Read), ByteOrEof::Eof); // §21.17
        for typ in [ReadType::Read, ReadType::HardAhead, ReadType::SoftAhead] {
            assert_eq!(f.get(5, typ), ByteOrEof::Eof); // pos == len, repeats
            assert_eq!(f.get(105, typ), ByteOrEof::Eof); // pos == len + 100
            assert_eq!(f.get(-1, typ), ByteOrEof::Eof); // §21.17, every typ
        }
        assert_eq!(f.get(1, ReadType::Read), ByteOrEof::Byte(1)); // usable after
    }
}
