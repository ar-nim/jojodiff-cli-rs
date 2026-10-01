//! `JFile` abstraction layer: the file byte-source interface used by the diff
//! and patch engines (C++ `JFile` / `JFileIStream*`; spec §9 for `-m 0`, §10
//! for buffered look-ahead).

pub mod mem;

pub use crate::defs::ReadType;
pub use mem::JFileMem;

/// Byte-source abstraction for the diff and patch engines, 1:1 with the C++
/// `JFile` interface (`headers/JFileIStream.h`).
///
/// `get` returns the requested byte as an `i32` in `0..=255`, or
/// [`crate::defs::EOF`] at end of input. [`crate::defs::EOB`] (`EOF - 1`) is
/// reserved for look-ahead implementations and is only ever returned for
/// soft-ahead reads (spec §10); the in-memory reader never returns it.
pub trait JFile {
    /// Returns the byte at `pos` (`0..=255`) or [`crate::defs::EOF`] when
    /// `pos` is negative or at/past the end of the file. `typ` distinguishes
    /// plain reads from hard/soft look-ahead; see [`ReadType`].
    fn get(&mut self, pos: i64, typ: ReadType) -> i32;

    /// Number of out-of-order accesses performed so far.
    fn seekcount(&self) -> i64;
}
