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

/// Byte-source abstraction for the diff and patch engines, 1:1 with the C++
/// `JFile` interface (`src/JFile.h`).
///
/// `get` returns the requested byte as an `i32` in `0..=255`, or
/// [`crate::defs::EOF`] at end of input. [`crate::defs::EOB`] (`EOF - 1`) is
/// reserved for look-ahead implementations and is only ever returned for
/// soft-ahead reads (spec §10); the in-memory reader never returns it. Since
/// 0.8.5 the implementations may also return the raw negative `EXI_*` error
/// sentinels (`JFileAhead.h:115`: `SeekError = EXI_SEK`, `ReadError =
/// EXI_RED`).
pub trait JFile {
    /// Returns the byte at `pos` (`0..=255`) or [`crate::defs::EOF`] when
    /// `pos` is negative or at/past the end of the file. `typ` distinguishes
    /// plain reads from hard/soft look-ahead; see [`ReadType`].
    fn get(&mut self, pos: i64, typ: ReadType) -> i32;

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

    /// Size of the buffer, `-1` = no buffering (C++ virtual with a `-1`
    /// default, `JFile.h:133`).
    fn get_buf_sze(&self) -> i64 {
        -1
    }

    /// Get access to the (fast) buffered read (C++ virtual with a null
    /// default, `JFile.h:144-146`).
    ///
    /// On success returns the buffer run starting at `pos` and sets `len` to
    /// the number of available bytes (the run never wraps the ring). On
    /// failure returns `None` and sets `len` to [`crate::defs::EOF`],
    /// [`crate::defs::EOB`], [`crate::defs::EXI_SEK`] or
    /// [`crate::defs::EXI_RED`] (`JFileAhead.cpp:222-225`). The null default
    /// leaves `len` untouched, exactly like the C++ base implementation.
    fn getbuf(&mut self, _pos: i64, _len: &mut i64, _typ: ReadType) -> Option<&[u8]> {
        None
    }
}
