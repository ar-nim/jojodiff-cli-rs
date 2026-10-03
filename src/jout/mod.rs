//! `JOut` output layer: the patch-writer abstraction used by the diff engine
//! (C++ `headers/JOut.h`; spec §11 for the implementations).
//!
//! [`JOut`] mirrors the C++ abstract `JOut` class; [`OutStats`] carries the
//! six public statistic counters (`gzOutByt*`, `JOut.h:53-57`).

pub mod asc;
pub mod bin;
pub mod rgn;
pub mod wire;

pub use asc::JOutAsc;
pub use bin::JOutBin;
pub use rgn::JOutRgn;

/// Statistics about operations (`JOut.h:53-57`). All counters are `off_t`
/// (`i64`) in the original.
#[derive(Default, Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutStats {
    /// Number of data bytes written (`gzOutBytDta`).
    pub dta: i64,
    /// Number of control bytes written, i.e. overhead (`gzOutBytCtl`).
    pub ctl: i64,
    /// Number of data bytes deleted (`gzOutBytDel`).
    pub del: i64,
    /// Number of data bytes backtracked (`gzOutBytBkt`).
    pub bkt: i64,
    /// Number of escape bytes written, i.e. overhead (`gzOutBytEsc`).
    pub esc: i64,
    /// Number of data bytes not written, i.e. gain (`gzOutBytEql`).
    pub eql: i64,
}

/// Abstract output routine for the diff engine, 1:1 with the C++ `JOut`
/// class (`headers/JOut.h:32-48`).
///
/// `put` is called by the engine to output one operand; it returns `false`
/// (continue sending byte by byte) or `true` (permission to send a length,
/// which is faster).
pub trait JOut {
    /// Outputs one operand.
    ///
    /// * `opr` — operand: [`ESC`](crate::defs::ESC),
    ///   [`INS`](crate::defs::INS), [`DEL`](crate::defs::DEL),
    ///   [`EQL`](crate::defs::EQL), [`BKT`](crate::defs::BKT) or
    ///   [`MOD`](crate::defs::MOD).
    /// * `len` — length of operand for DEL and BKT.
    /// * `org` — character from the original file.
    /// * `new` — character from the new file.
    /// * `pos_org` — position within the original file.
    /// * `pos_new` — position within the new file.
    ///
    /// Returns `false` = continue sending byte by byte, `true` = permission
    /// to send length (faster).
    fn put(&mut self, opr: i32, len: i64, org: i32, new: i32, pos_org: i64, pos_new: i64) -> bool;

    /// Statistics about the operations performed so far.
    fn stats(&self) -> OutStats;
}
