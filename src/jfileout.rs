//! `JFileOut`: JojoDiff's patch-phase output abstraction, 1:1 port of C++
//! `src/JFileOut.cpp` (spec §18.E/§21.14): [`JFileOut::putc`] writes one byte,
//! [`JFileOut::copyfrom`] copies a series of bytes from a [`JFile`] input,
//! first trying the `getbuf` fast path and falling back to a per-byte loop
//! whose stray discard read is ported as-is (§21.14).
//!
//! The C++ class wraps a `FILE *`; the Rust port is generic over any
//! `std::io::Write` sink so the CLI can hand it stdout or a file (Task 20).

use std::io::Write;

use crate::defs::{EOF, EXI_OK, EXI_RED, EXI_WRI};
use crate::jfile::{ByteOrEof, JFile, ReadType};

/// Patch-phase output file (`JFileOut`, `JFileOut.h:35-69`), generic over the
/// byte sink (the C++ `FILE * const mpFil`).
pub struct JFileOut<W: Write> {
    out: W,
}

impl<W: Write> JFileOut<W> {
    /// Create JFileOut on an output sink (`JFileOut::JFileOut`,
    /// `JFileOut.cpp:28-31`: "Stdio file, opened and ready for writing").
    pub fn new(out: W) -> Self {
        JFileOut { out }
    }

    /// Returns the underlying sink, discarding nothing (the Rust-side
    /// counterpart of dropping the C++ object; needed to flush/drop buffered
    /// writers such as `BufWriter`).
    pub fn into_inner(self) -> W {
        self.out
    }

    /// Write a byte to the output (`JFileOut::putc`, `JFileOut.cpp:82-84`):
    /// C `fputc` — returns the byte written (`dta & 0xff`) or
    /// [`crate::defs::EOF`] on a write error.
    pub fn putc(&mut self, ai_dta: i32) -> i32 {
        match self.out.write_all(&[ai_dta as u8]) {
            Ok(()) => ai_dta & 0xff,
            Err(_) => EOF,
        }
    }

    /// Copy a series of bytes from input to output (`JFileOut::copyfrom`,
    /// `JFileOut.cpp:33-75`). Returns [`EXI_OK`] on success,
    /// [`EXI_RED`] on a short read ("Error reading source file.") and
    /// [`EXI_WRI`] on a short write ("Error writing output file.").
    ///
    /// First tries buffered copying via [`JFile::getbuf`]; if that is not
    /// available, copies character by character. The byte-loop fallback ends
    /// each iteration with the C++ stray discard read (`apFilInp.get()`,
    /// `JFileOut.cpp:67` — reads at the advanced sequential cursor and throws
    /// the result away; spec §21.14: observable in the read cursor, ported
    /// as-is).
    pub fn copyfrom(&mut self, inp: &mut dyn JFile, mut az_pos: i64, mut az_len: i64) -> i32 {
        /* First try buffered copying (JFileOut.cpp:37-38). `lz_len` is the
         * C++ `lzLen`: on input the requested length, on output the number
         * of bytes the buffer can serve (possibly more than requested). */
        let mut lz_len: i64 = az_len;
        let mut lp_buf: Option<&[u8]> = inp.getbuf(az_pos, &mut lz_len, ReadType::Read);

        if lp_buf.is_some() {
            while az_len > 0 {
                /* In-loop null check for the re-issued getbuf
                 * (JFileOut.cpp:41-44). */
                let Some(buf) = lp_buf.take() else {
                    eprintln!("Error reading source file.");
                    return EXI_RED;
                };
                if lz_len > az_len {
                    lz_len = az_len;
                }
                if self.out.write_all(&buf[..lz_len as usize]).is_err() {
                    eprintln!("Error writing output file.");
                    return EXI_WRI;
                }
                az_len -= lz_len;
                az_pos += lz_len;
                if az_len > 0 {
                    lp_buf = inp.getbuf(az_pos, &mut lz_len, ReadType::Read);
                }
            }
            return EXI_OK;
        }

        /* Copy character by character (JFileOut.cpp:56-73). */
        while az_len > 0 {
            /* `lcVal <= EOF` (JFileOut.cpp:59): EOF and the error sentinels
             * end the copy as a short read. */
            let lc_val = match inp.getv(az_pos, ReadType::Read) {
                ByteOrEof::Byte(lc_val) => lc_val,
                _ => break,
            };
            if self.putc(i32::from(lc_val)) < 0 {
                eprintln!("Error writing output file.");
                return EXI_WRI;
            }
            /* Stray discard read (`JFileOut.cpp:67`): the C++ zero-argument
             * `get()` reads at the advanced sequential cursor — one past the
             * byte just output — and ignores the result. */
            let _ = inp.getv(az_pos + 1, ReadType::Read);
            az_len -= 1;
            az_pos += 1;
        }
        if az_len > 0 {
            eprintln!("Error reading source file.");
            return EXI_RED;
        }
        EXI_OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::{EXI_OK, EXI_RED, EXI_WRI};
    use crate::jfile::{JFile, JFileAhead, JFileMem, ReadType};
    use std::io::Cursor;

    /// A sink whose every write fails (`/dev/full` stand-in).
    struct FailWriter;
    impl Write for FailWriter {
        fn write(&mut self, _buf: &[u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("device full"))
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Deterministic file contents: byte `i` is `(i * 7 + 3) % 256`.
    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| ((i * 7 + 3) % 256) as u8).collect()
    }

    /// `putc` writes the low byte of its argument and returns it like C
    /// `fputc` (`JFileOut.cpp:82-84`).
    #[test]
    fn putc_writes_low_byte_and_returns_it() {
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.putc(i32::from(b'A')), 0x41);
        assert_eq!(o.putc(0xA7), 0xA7);
        assert_eq!(o.putc(-1), 0xFF); // fputc(int) casts to unsigned char
        assert_eq!(o.into_inner(), vec![0x41, 0xA7, 0xFF]);
    }

    /// Buffered copying takes the `getbuf` fast path (`JFileOut.cpp:37-55`)
    /// when the input has a buffer: exact bytes over one run and across many
    /// `getbuf` rounds (the copied chunk count shrinks `lzLen` like the C++
    /// reuses the in/out variable).
    #[test]
    fn copyfrom_buffered_fast_path_exact_bytes() {
        let src = data(600);

        // One getbuf run (buffer 1024, block 16).
        let mut f = JFileAhead::new(Cursor::new(src.clone()), "Tst", 1024, 16);
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.copyfrom(&mut f, 10, 300), EXI_OK);
        assert_eq!(o.into_inner(), src[10..310]);

        // 512 bytes through 32 getbuf rounds (Append fills one 16-byte block
        // per round; the copied chunk count shrinks `lz_len` like the C++
        // reuses the in/out variable). The copy must end before EOF: once the
        // buffer window reaches the end, getbuf returns null and the copy
        // reports a short read (JFileOut.cpp:41-44) — covered below.
        let mut f = JFileAhead::new(Cursor::new(src.clone()), "Tst", 1024, 16);
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.copyfrom(&mut f, 0, 512), EXI_OK);
        assert_eq!(o.into_inner(), src[0..512]);
    }

    /// Without a `getbuf` implementation the byte-loop fallback runs
    /// (`JFileOut.cpp:56-73`) and its stray discard read advances the
    /// sequential read cursor after every byte (spec §21.14): on `JFileMem`
    /// every addressed get after the first stray lands out of order, so
    /// `seekcount` ticks and the cursor ends one past the last stray read.
    #[test]
    fn copyfrom_fallback_stray_discard_advances_cursor() {
        let src = data(8);
        let mut f = JFileMem::new(src.clone());
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.copyfrom(&mut f, 0, 4), EXI_OK);
        assert_eq!(o.into_inner(), src[0..4]);

        // Per iteration: addressed get(pos) then stray get(pos+1). The stray
        // pulls the cursor ahead, so the next iteration's addressed read is
        // out of order: 3 seeks for a 4-byte copy. (Without the stray read,
        // JFileMem would count 0.)
        assert_eq!(
            f.seekcount(),
            3,
            "stray discard read must advance the cursor"
        );
        // Cursor sits at 5 (after the stray get(4)): reading position 4 is
        // out of order again (4th seek) but still served.
        assert_eq!(f.get(4, ReadType::Read), i32::from(src[4]));
        assert_eq!(f.seekcount(), 4);
    }

    /// A copy that runs past the end of the input reports a short read
    /// (`JFileOut.cpp:69-72`: "Error reading source file.", `EXI_RED`) — in
    /// both the fallback loop and the buffered path.
    #[test]
    fn copyfrom_short_read_is_exi_red() {
        let src = data(4);

        // Fallback loop (JFileMem): get(4) returns EOF -> break -> azLen > 0.
        let mut f = JFileMem::new(src.clone());
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.copyfrom(&mut f, 0, 100), EXI_RED);

        // Buffered path (JFileAhead): getbuf eventually returns null with
        // *len == EOF, the in-loop null check fires (JFileOut.cpp:41-44).
        let mut f = JFileAhead::new(Cursor::new(src), "Tst", 1024, 16);
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.copyfrom(&mut f, 0, 100), EXI_RED);
    }

    /// A failing sink is `EXI_WRI` ("Error writing output file.") on both
    /// paths: the checked `fwrite` of the buffered loop and the checked
    /// `putc` of the fallback (`JFileOut.cpp:47-50,63-66`).
    #[test]
    fn copyfrom_write_error_is_exi_wri() {
        let src = data(64);

        let mut f = JFileAhead::new(Cursor::new(src.clone()), "Tst", 1024, 16);
        let mut o = JFileOut::new(FailWriter);
        assert_eq!(o.copyfrom(&mut f, 0, 32), EXI_WRI, "buffered fwrite check");

        let mut f = JFileMem::new(src);
        let mut o = JFileOut::new(FailWriter);
        assert_eq!(o.copyfrom(&mut f, 0, 32), EXI_WRI, "fallback putc check");
    }

    /// A zero-length copy writes nothing and succeeds on both paths (both
    /// C++ loops are skipped, `JFileOut.cpp:40,59`).
    #[test]
    fn copyfrom_zero_length_is_exi_ok() {
        let src = data(8);
        let mut f = JFileAhead::new(Cursor::new(src.clone()), "Tst", 1024, 16);
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.copyfrom(&mut f, 0, 0), EXI_OK);
        assert!(o.into_inner().is_empty());

        let mut f = JFileMem::new(src);
        let mut o = JFileOut::new(Vec::new());
        assert_eq!(o.copyfrom(&mut f, 3, 0), EXI_OK);
        assert!(o.into_inner().is_empty());
    }
}
