//! In-memory `JFile` reader for `-m 0` (`JFileIStream` semantics, spec §9).
//!
//! Serves bytes from a `Vec<u8>`; `get` ignores the `ReadType` and never
//! returns `EOB`. Unlike the C++ original, which builds an
//! `istringstream(char*)` and thereby truncates at the first NUL byte
//! (spec §15.3), NUL bytes are served like any other byte.

use super::{JFile, ReadType};
use crate::defs::EOF;

/// In-memory byte source, 1:1 with C++ `JFileIStream` (`src/JFileIStream.cpp`,
/// spec §9): `data` holds the whole file, `pos_inp` is the position of the
/// next sequential byte (sequential fast path), `seeks` counts out-of-order
/// accesses.
pub struct JFileMem {
    data: Vec<u8>,
    pos_inp: i64,
    seeks: i64,
}

impl JFileMem {
    /// Buffers `data` as the file contents (`-m 0`: each file is read fully
    /// into memory).
    pub fn new(data: Vec<u8>) -> Self {
        JFileMem {
            data,
            pos_inp: 0,
            seeks: 0,
        }
    }
}

impl JFile for JFileMem {
    fn get(&mut self, pos: i64, _typ: ReadType) -> i32 {
        if pos != self.pos_inp {
            self.seeks += 1;
        }
        self.pos_inp = pos + 1;
        if pos < 0 || pos as usize >= self.data.len() {
            EOF
        } else {
            i32::from(self.data[pos as usize])
        }
    }

    fn seekcount(&self) -> i64 {
        self.seeks
    }

    /// C++ `JFileIStream::set_lookahead_base` (`src/JFileIStream.cpp:63-70`):
    /// "no need to do anything" — ported as the no-op it is.
    fn set_lookahead_base(&mut self, _base: i64) {}

    /// In-memory data is always seekable: `chkSeq` keeps `mbSeq` false when
    /// `jeofpos` succeeds (`JFile.cpp:37-46`, `JFileIStream.cpp:30-34`).
    fn is_sequential(&self) -> bool {
        false
    }

    /// EOF position = data length (`JFileIStream::jeofpos` end-seek result,
    /// `JFileIStream.cpp:43-57`).
    fn jeofpos(&mut self) -> i64 {
        self.data.len() as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{EOB, EOF, ReadType};

    /// Brief step-1 test, verbatim: sequential reads return bytes in order,
    /// NUL bytes survive, `seekcount` stays 0 on the sequential run and
    /// increments once per out-of-order `get`; end and negative positions
    /// return `EOF`.
    #[test]
    fn seq_nul_eof_seekcount() {
        let mut f = JFileMem::new(vec![0u8, 1, 0, 3, 0xA7]);
        assert_eq!(f.get(0, ReadType::Read), 0);
        assert_eq!(f.get(1, ReadType::Read), 1);
        assert_eq!(f.get(2, ReadType::SoftAhead), 0); // soft never EOBs
        assert_eq!(f.seekcount(), 0);
        assert_eq!(f.get(4, ReadType::Read), 0xA7);
        assert_eq!(f.seekcount(), 1); // jumped 2→4
        assert_eq!(f.get(5, ReadType::Read), EOF);
        assert_eq!(f.get(-1, ReadType::Read), EOF);
    }

    /// `EOB` must never be returned, for any `ReadType`, at any position;
    /// `EOF` is returned at `len` and far beyond it, repeatedly; negative
    /// positions return `EOF` for all `ReadType`s. The reader stays usable
    /// afterwards.
    #[test]
    fn eob_never_returned_eof_repeats() {
        let mut f = JFileMem::new(vec![0u8, 2]);
        for typ in [ReadType::Read, ReadType::HardAhead, ReadType::SoftAhead] {
            assert_eq!(f.get(0, typ), 0);
            assert_eq!(f.get(1, typ), 2);
            assert_ne!(f.get(2, typ), EOB); // pos == len
            assert_eq!(f.get(2, typ), EOF);
            assert_ne!(f.get(102, typ), EOB); // pos == len + 100
            assert_eq!(f.get(102, typ), EOF);
            assert_ne!(f.get(-1, typ), EOB);
            assert_eq!(f.get(-1, typ), EOF);
        }
        assert_eq!(f.get(1, ReadType::Read), 2);
    }

    /// `seekcount` stays 0 during a sequential run and increments exactly
    /// once per out-of-order `get` (the counter has no other effect).
    #[test]
    fn seekcount_once_per_out_of_order_get() {
        let mut f = JFileMem::new(vec![0u8, 1, 2, 3, 4, 5]);
        assert_eq!(f.get(0, ReadType::Read), 0);
        assert_eq!(f.get(1, ReadType::HardAhead), 1);
        assert_eq!(f.get(2, ReadType::SoftAhead), 2);
        assert_eq!(f.seekcount(), 0); // sequential run
        assert_eq!(f.get(0, ReadType::Read), 0); // seek back
        assert_eq!(f.seekcount(), 1);
        assert_eq!(f.get(4, ReadType::Read), 4); // seek forward
        assert_eq!(f.seekcount(), 2);
        assert_eq!(f.get(4, ReadType::Read), 4); // pos_inp has moved past 4
        assert_eq!(f.seekcount(), 3);
        assert_eq!(f.get(5, ReadType::Read), 5); // sequential again
        assert_eq!(f.seekcount(), 3);
        assert_eq!(f.get(5, ReadType::Read), 5); // pos_inp has moved past 5:
        assert_eq!(f.seekcount(), 4); // out-of-order, byte still served
        assert_eq!(f.get(6, ReadType::Read), EOF); // pos == len, sequential
        assert_eq!(f.seekcount(), 4);
        assert_eq!(f.get(6, ReadType::Read), EOF); // EOF repeatedly
        assert_eq!(f.seekcount(), 5);
    }

    /// `ReadType` discriminants are 0/1/2 (`0=read, 1=hard ahead, 2=soft
    /// ahead`, `JFileIStream.cpp:46-48`); the `jfile` re-export path works.
    #[test]
    fn readtype_discriminants() {
        use crate::jfile::ReadType as ViaJfile;
        assert_eq!(ViaJfile::Read as i32, 0);
        assert_eq!(ViaJfile::HardAhead as i32, 1);
        assert_eq!(ViaJfile::SoftAhead as i32, 2);
    }
}
