//! Buffered look-ahead `JFile` reader — the default input mode of both tools
//! (C++ `JFileAhead`, spec §10).
//!
//! 1:1 port of `reference/jojodiff-cpp/src/JFileAhead.cpp:69-346`
//! (`JFileIStreamAhead.cpp` implements the same algorithm, spec §15.6). A
//! circular buffer of `buf_sze` bytes is refilled in `blk_sze` chunks:
//! sequential reads are served from the buffer without touching the file,
//! scroll-back reads are served from history still in the buffer, and only
//! true out-of-buffer accesses seek. Soft-ahead reads outside the buffer
//! return [`EOB`] before any file access.
//!
//! Seek modes (`JFileAhead.cpp:108`): 0 = append, 1 = seek & reset, 2 = scroll
//! back. NB: the C++ declares the mode variable as `bool` (`liSek`) and passes
//! it to an `int` parameter, so any non-zero mode arrives as exactly 1: the
//! mode-2 scroll-back arm is compiled but unreachable in every real JojoDiff
//! 0.8.1 build (spec §15.8). This port replicates that collapse explicitly at
//! the `get_frombuffer` → `get_outofbuffer` boundary; the mode-2 arm is
//! retained, mirroring the dead C++ code.
//!
//! # Debug prints (spec §14, `debug` feature)
//!
//! The `#if debug` sites are ported verbatim: the DBGBUF `ufFabOpn` open line
//! (`JFileAhead.cpp:50-54`) and `ufFabGet: Seek` line (`:270-273`), and the
//! DBGRED `ufFabGet` fast-path, in-buffer, EOF and EOB lines (`:76-81`,
//! `:118-122`, `:146-151`, `:165-170`) plus the store-fill pair in
//! `get_outofbuffer` (`:283-298`). `%p` values print the equivalent Rust
//! addresses (buffer start/end, per-byte buffer slot, fid string pointer);
//! only the line *shape* is oracle-pinnable. Two sites pass the fid STRING to
//! `%p` in the C++ (`:165-170`, `:283-288`) — the port preserves the shape by
//! printing the fid's address (`String::as_ptr`).

use std::io::{Read, Seek, SeekFrom};
use std::process;

use super::{JFile, ReadType};
#[cfg(feature = "debug")]
use crate::defs::p8;
use crate::defs::{EOB, EOF, EXI_SEK};
#[cfg(feature = "debug")]
use crate::jdebug::{DBGBUF, DBGRED, dbg, dbg_print};

/// Buffered look-ahead byte source, 1:1 with C++ `JFileAhead`
/// (`src/JFileAhead.cpp`).
///
/// State (C++ names in comments): `buf`/`mpBuf..mpMax` is a circular buffer,
/// `pos_inp`/`mzPosInp` is the file offset of the next unread chunk byte,
/// `ptr_inp`/`mpInp` its buffer index, `buf_usd`/`miBufUsd` the number of
/// valid bytes, `pos_red`/`mzPosRed` + `ptr_red`/`mpRed` + `red_sze`/`miRedSze`
/// track the sequential fast path, and `pos_eof`/`mzPosEof` is the known end
/// of file (`i64::MAX` = unknown).
pub struct JFileAhead<R: Read + Seek> {
    file: R,
    /// File identifier for debug prints (C++ `msFid`); read by the DBGBUF/
    /// DBGRED sites (`JFileAhead.cpp:52,78,119,147,166,285,295`).
    #[cfg_attr(not(feature = "debug"), allow(dead_code))]
    fid: String,
    buf_sze: i64,
    blk_sze: i64,
    buf: Vec<u8>,
    red_sze: i64,
    buf_usd: i64,
    ptr_inp: usize,
    ptr_red: usize,
    pos_inp: i64,
    pos_red: i64,
    pos_eof: i64,
    seeks: i64,
}

impl<R: Read + Seek> JFileAhead<R> {
    /// Buffers `file` with a `buf_sze`-byte circular buffer read in
    /// `blk_sze`-byte chunks (C++ `JFileAhead.cpp:34-55`). A non-positive
    /// `buf_sze` falls back to one block; the C++ never passes 0 here.
    pub fn new(file: R, fid: &str, buf_sze: i64, blk_sze: i32) -> Self {
        let buf_sze = if buf_sze <= 0 {
            i64::from(blk_sze)
        } else {
            buf_sze
        };
        let fab = JFileAhead {
            file,
            fid: fid.to_string(),
            buf_sze,
            blk_sze: i64::from(blk_sze),
            buf: vec![0_u8; buf_sze as usize],
            red_sze: 0,
            buf_usd: 0,
            ptr_inp: 0,
            ptr_red: 0,
            pos_inp: 0,
            pos_red: 0,
            pos_eof: i64::MAX, // MAX_OFF_T
            seeks: 0,
        };

        /* Debug: open trace (JFileAhead.cpp:50-54); the pointers are the
         * buffer allocation's start/end like the C++ `mpBuf`/`mpMax`. */
        #[cfg(feature = "debug")]
        if dbg(DBGBUF) {
            let range = fab.buf.as_ptr_range();
            dbg_print(format_args!(
                "ufFabOpn({}):(buf={:p},max={:p},sze={})\n",
                fab.fid, range.start, range.end, fab.buf_sze,
            ));
        }

        fab
    }

    /// C `fread` at the current file position: fills `buf[idx..idx+want]`,
    /// looping until `want` bytes, end of file or error. A read error counts
    /// as a short read, like a failing C `fread` (JFileAhead.cpp:282).
    fn read_cur(&mut self, idx: usize, want: i64) -> Result<i64, i32> {
        let mut done = 0_i64;
        while done < want {
            match self
                .file
                .read(&mut self.buf[(idx + done as usize)..(idx + want as usize)])
            {
                Ok(0) => break,
                Ok(n) => done += n as i64,
                Err(_) => break,
            }
        }
        Ok(done)
    }

    /// C `jfseek` + `fread` (JFileAhead.cpp:276-282): a failed seek yields the
    /// `-EXI_SEK` sentinel, a failed read a short read.
    fn read_chunk(&mut self, file_pos: i64, idx: usize, want: i64) -> Result<i64, i32> {
        // C `fseek` fails on negative offsets; `Cursor` would accept them.
        if file_pos < 0 || self.file.seek(SeekFrom::Start(file_pos as u64)).is_err() {
            return Err(-EXI_SEK);
        }
        self.read_cur(idx, want)
    }

    /// Tries to get data from the buffer; calls
    /// [`get_outofbuffer`](Self::get_outofbuffer) if that is not possible
    /// (C++ `get_frombuffer`, `JFileAhead.cpp:103-174`).
    fn get_frombuffer(&mut self, pos: i64, typ: ReadType) -> i32 {
        // C++ `bool liSek` (JFileAhead.cpp:108): reposition on file?
        // 0=no, 1=yes, 2=scroll back. The C++ type collapses 2 to `true`
        // at assignment; the int conversion happens at the call boundary
        // below (see spec §15.8).
        let mut li_sek = 0_i32;

        /* Get data from buffer? */
        if pos < self.pos_inp {
            if pos >= self.pos_inp - self.buf_usd {
                // compute position in buffer (JFileAhead.cpp:114-116)
                let mut lp = self.ptr_inp as i64 - (self.pos_inp - pos);
                if lp < 0 {
                    lp += self.buf_sze;
                }

                // prepare next reading position (but do not increase lp!!!)
                // (JFileAhead.cpp:124-134)
                self.pos_red = pos + 1;
                let mut pr = lp + 1;
                if pr == self.buf_sze {
                    pr = 0;
                }
                self.ptr_red = pr as usize;
                self.red_sze = if pr > self.ptr_inp as i64 {
                    self.buf_sze - pr // mpRed > mpInp: run ends at mpMax
                } else {
                    self.pos_inp - self.pos_red
                };

                /* Debug: in-buffer trace (JFileAhead.cpp:118-122). */
                #[cfg(feature = "debug")]
                if dbg(DBGRED) {
                    let mem = self.buf.as_ptr().wrapping_add(lp as usize);
                    dbg_print(format_args!(
                        "ufFabGet({},{},{})->{:2x} (mem {:p}).\n",
                        self.fid,
                        p8(pos),
                        typ as i32,
                        self.buf[lp as usize] as u32,
                        mem,
                    ));
                }

                // return data (JFileAhead.cpp:137)
                return self.buf[lp as usize] as i32;
            } else {
                // Seek & reset when reading before the buffer
                // (JFileAhead.cpp:139-143): the C++ assigns 2 ("just before
                // buffer") to a bool here, which stores `true` (spec §15.8).
                if pos + self.blk_sze >= self.pos_inp - self.buf_usd {
                    li_sek = 2; /* reading just before buffer */
                } else {
                    li_sek = 1; /* seek & reset buffer */
                }
            }
        } else if pos >= self.pos_eof {
            // eof (JFileAhead.cpp:145-157)
            /* Debug: EOF trace (JFileAhead.cpp:146-151). */
            #[cfg(feature = "debug")]
            if dbg(DBGRED) {
                dbg_print(format_args!(
                    "ufFabGet({},{},{})->EOF (mem).\n",
                    self.fid,
                    p8(pos),
                    typ as i32,
                ));
            }

            self.pos_red = -1;
            self.ptr_red = 0; // C++: mpRed = null; kept safe by red_sze == 0
            self.red_sze = 0;

            return EOF;
        } else if pos >= self.pos_inp + self.blk_sze {
            // Reset when reading "after" the buffer (JFileAhead.cpp:158-161)
            li_sek = 1;
        }

        // Soft ahead: continue only if no seek (JFileAhead.cpp:163-171)
        if typ == ReadType::SoftAhead && li_sek != 0 {
            /* Debug: end-of-buffer trace (JFileAhead.cpp:165-170). The C++
             * passes the fid STRING to `%p` (formatting quirk); the port
             * preserves the shape with the fid's address. */
            #[cfg(feature = "debug")]
            if dbg(DBGRED) {
                dbg_print(format_args!(
                    "ufFabGet({:p},{},{})->EOB.\n",
                    self.fid.as_ptr(),
                    p8(pos),
                    typ as i32,
                ));
            }
            return EOB;
        }

        // C++ passes `liSek` — declared `bool` (JFileAhead.cpp:108) — to
        // `get_outofbuffer(const int aiSek, ...)` (JFileAhead.cpp:173): the
        // bool→int conversion delivers any non-zero mode as exactly 1, so the
        // case-(2) scroll-back arm never executes in any real build (spec
        // §15.8). Replicated explicitly; the arm is retained as dead-code
        // parity.
        let li_sek: i32 = if li_sek != 0 { 1 } else { 0 };
        self.get_outofbuffer(pos, typ, li_sek)
    }

    /// Read data from the file into the buffer, then read from the buffer
    /// (C++ `get_outofbuffer`, `JFileAhead.cpp:179-346`).
    ///
    /// `sek`: 0 = append, 1 = seek & reset, 2 = scroll back. Mode 2 never
    /// arrives through `get_frombuffer` (bool collapse, spec §15.8); the arm
    /// is kept as dead-code parity with the C++.
    fn get_outofbuffer(&mut self, pos: i64, typ: ReadType, sek: i32) -> i32 {
        // Set reading position: lz_pos (position to seek), lp_inp (place in
        // buffer to read to) and li_tdo (number of bytes to read)
        // (JFileAhead.cpp:189-268).
        let (lz_pos, lp_inp, li_tdo) = match sek {
            0 => {
                /* How many bytes can we read ? */
                let mut tdo = self.buf_sze - self.ptr_inp as i64;
                if tdo > self.blk_sze {
                    tdo = self.blk_sze;
                }
                (self.pos_inp, self.ptr_inp, tdo)
            }

            1 => {
                /* reset buffer */
                self.ptr_inp = 0;
                self.pos_inp = pos;
                self.ptr_red = 0;
                self.pos_red = pos;
                self.buf_usd = 0;
                self.red_sze = 0;

                /* set position */
                (pos, 0, self.blk_sze)
            }

            // Dead in every real build (spec §15.8): `get_frombuffer` only
            // ever delivers 0 or 1 across the bool bottleneck; kept for 1:1
            // parity with the compiled-but-dead C++ case (2).
            2 => {
                /* make room in buffer */
                let drop = self.buf_usd + self.blk_sze - self.buf_sze;
                if drop > 0 {
                    self.buf_usd -= drop;
                    self.pos_inp -= drop;
                    let mut pi = self.ptr_inp as i64 - drop;
                    if pi < 0 {
                        pi += self.buf_sze;
                    }
                    self.ptr_inp = pi as usize;
                }

                /* scroll back on buffer */
                /* case 1: [^***********$-----------Tdo]   */
                /* case 2: [************$----Tdo^******]   */
                /* case 3: [Td^*********$--------------]   */
                /* case 4: [--Tdo^*****$---------------]   */
                let mut lz = self.pos_inp - self.buf_usd;
                let mut tdo = self.blk_sze;
                if lz < tdo {
                    tdo = lz;
                }
                let mut lp = self.ptr_inp as i64 - self.buf_usd;
                if lp == 0 {
                    /* case 1 */
                    lp = self.buf_sze - tdo;
                } else if lp > 0 {
                    if lp - tdo >= 0 {
                        /* case 4 */
                        lp -= tdo;
                    } else {
                        /* case 3 */
                        tdo = lp;
                        lp = 0;
                    }
                } else {
                    /* case 2 */
                    lp += self.buf_sze - tdo;
                }
                self.buf_usd += tdo;
                lz -= tdo;

                /* reset read position buffer */
                self.ptr_red = 0; // C++: mpRed = null; kept safe by red_sze == 0
                self.pos_red = -1;
                self.red_sze = 0;

                (lz, lp as usize, tdo)
            }

            // The C++ default arm only silences an uninitialized warning
            // ("TODO make aiSek an enum"); get_frombuffer yields 0, 1, 2.
            _ => unreachable!("get_frombuffer only yields seek modes 0, 1 and 2"),
        };

        if sek != 0 {
            /* Debug: repositioning trace, before the seek like the C++
             * (JFileAhead.cpp:270-273); prints the original `azPos`
             * UNPADDED (`%"PRIzd"`, not `P8zd`). */
            #[cfg(feature = "debug")]
            if dbg(DBGBUF) {
                dbg_print(format_args!("ufFabGet: Seek {}.\n", pos));
            }
            self.seeks += 1;
        } /* if liSek */

        // Read a chunk of data (JFileAhead.cpp:281-293); mode 0 appends at the
        // current file position without seeking.
        let read = if sek == 0 {
            self.read_cur(lp_inp, li_tdo)
        } else {
            self.read_chunk(lz_pos, lp_inp, li_tdo)
        };
        let done = match read {
            Ok(done) => done,
            Err(sentinel) => return sentinel, // -EXI_SEK
        };
        if done < li_tdo {
            // End of file reached (JFileAhead.cpp:283-293). The C++ prints
            // the fid STRING with `%p` here (formatting quirk, shape kept).
            /* Debug: short-read EOF trace (JFileAhead.cpp:284-288). */
            #[cfg(feature = "debug")]
            if dbg(DBGRED) {
                dbg_print(format_args!(
                    "ufFabGet({:p},{},{})->EOF.\n",
                    self.fid.as_ptr(),
                    p8(pos),
                    typ as i32,
                ));
            }
            self.pos_eof = lz_pos + done;
            if done == 0 {
                return EOF;
            }
        }

        /* Debug: store-fill trace with the byte just read and its buffer
         * address (`*mpInp`/`mpInp`, JFileAhead.cpp:294-298). */
        #[cfg(feature = "debug")]
        if dbg(DBGRED) {
            let sto = self.buf.as_ptr().wrapping_add(lp_inp);
            dbg_print(format_args!(
                "ufFabGet({},{},{})->{:2x} (sto {:p}).\n",
                self.fid,
                p8(pos),
                typ as i32,
                self.buf[lp_inp] as u32,
                sto,
            ));
        }

        match sek {
            2 => {
                if done < li_tdo {
                    /* repair buffer (JFileAhead.cpp:302-311) */
                    let mut pi = lp_inp as i64 + done;
                    if pi >= self.buf_sze {
                        pi -= self.buf_sze;
                    }
                    self.ptr_inp = pi as usize;
                    self.pos_inp = lz_pos + done;
                    self.ptr_red = lp_inp;
                    self.pos_red = lz_pos;
                    self.buf_usd = done;
                    self.red_sze = done;
                } else {
                    /* Restore input position (JFileAhead.cpp:312-319) */
                    self.seeks += 1;
                    if self
                        .file
                        .seek(SeekFrom::Start(self.pos_inp as u64))
                        .is_err()
                    {
                        return -EXI_SEK;
                    }
                }
            }

            _ => {
                /* Advance input position (JFileAhead.cpp:322-341) */
                self.pos_inp += done;
                let pi = self.ptr_inp as i64 + done;
                if pi == self.buf_sze {
                    self.ptr_inp = 0;
                } else if pi > self.buf_sze {
                    // Faithful to JFileAhead.cpp:329, including the stray ')'
                    // and the missing newline.
                    eprint!("Buffer out of bounds on position {}!)!", pos);
                    process::exit(6);
                } else {
                    self.ptr_inp = pi as usize;
                }
                // C++: clamp up to mlBufSze, never over it.
                self.buf_usd = (self.buf_usd + done).min(self.buf_sze);
                self.red_sze += done;
                if self.ptr_red == self.buf.len() {
                    self.ptr_red = 0;
                }
            }
        } /* switch aiSek */

        /* read it again (JFileAhead.cpp:344-345) */
        self.get(pos, typ)
    }
}

impl<R: Read + Seek> JFile for JFileAhead<R> {
    /// Gets one byte from the look-ahead file (JFileAhead.cpp:69-85).
    fn get(&mut self, pos: i64, typ: ReadType) -> i32 {
        if self.red_sze > 0 && pos == self.pos_red {
            // Sequential fast path: serve from the buffer.
            self.pos_red += 1;
            self.red_sze -= 1;
            let byte = self.buf[self.ptr_red] as i32;

            /* Debug: fast-path trace with the byte's buffer address, printed
             * before `mpRed++` like the C++ (JFileAhead.cpp:76-81). */
            #[cfg(feature = "debug")]
            if dbg(DBGRED) {
                let mem = self.buf.as_ptr().wrapping_add(self.ptr_red);
                dbg_print(format_args!(
                    "ufFabGet({},{},{})->{:2x} (mem {:p}).\n",
                    self.fid,
                    p8(pos),
                    typ as i32,
                    byte as u32,
                    mem,
                ));
            }

            // C++ advances mpRed unwrapped; red_sze == 0 keeps it from ever
            // being dereferenced past mpMax, so wrapping here is equivalent.
            self.ptr_red = (self.ptr_red + 1) % self.buf.len();
            byte
        } else {
            self.get_frombuffer(pos, typ)
        }
    }

    /// Number of seeks performed (JFileAhead.cpp:64).
    fn seekcount(&self) -> i64 {
        self.seeks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Deterministic file contents: byte `i` is `(i * 7 + 3) % 256`.
    fn data(n: usize) -> Vec<u8> {
        (0..n).map(|i| ((i * 7 + 3) % 256) as u8).collect()
    }

    /// Expected value of byte `i` as `get` returns it.
    fn pat(i: i64) -> i32 {
        ((i * 7 + 3) % 256) as i32
    }

    fn mk(data: Vec<u8>) -> JFileAhead<Cursor<Vec<u8>>> {
        JFileAhead::new(Cursor::new(data), "Tst", 1024, 16)
    }

    /// Brief step-1 test: sequential reads are served by the fast path from
    /// the buffer; the file is only ever read in append mode (no seeks).
    #[test]
    fn sequential_read_uses_fast_path() {
        let mut f = mk(data(600));
        for i in 0..100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0);
    }

    /// Brief step-1 test: soft-ahead reads outside the buffer window return
    /// `EOB` before any file access (both far before and just before the
    /// window); the hard retry then seek-&-resets and returns the real byte.
    ///
    /// NB: the brief sketches this as "read to 200", but with the brief's own
    /// buffer size (1024) position 0 is still *inside* the window then, and
    /// the C++ serves it from the buffer — so this test also pins that
    /// in-buffer soft read before moving past the window.
    #[test]
    fn soft_ahead_before_buffer_returns_eob() {
        let mut f = mk(data(2048));
        for i in 0..200 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        // Window is [0, 208): position 0 is still buffered, even for soft.
        assert_eq!(f.get(0, ReadType::SoftAhead), pat(0));
        assert_eq!(f.seekcount(), 0);
        for i in 200..1100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        // Window is now [80, 1104): position 0 is far before it, 70 just
        // before it — both must EOB without touching the file.
        assert_eq!(f.get(0, ReadType::SoftAhead), EOB);
        assert_eq!(f.get(70, ReadType::SoftAhead), EOB);
        assert_eq!(f.seekcount(), 0, "EOB must not access the file");
        // The first byte inside the window still reads from the buffer.
        assert_eq!(f.get(80, ReadType::SoftAhead), pat(80));
        // A hard read takes the seek-&-reset path (1 seek): the C++ `bool
        // liSek` collapse makes near-before-buffer behave like far (§15.8).
        assert_eq!(f.get(70, ReadType::HardAhead), pat(70));
        assert_eq!(f.seekcount(), 1);
        // The reset dropped all history: position 69 is outside the new
        // window [70, 86), so even a soft read must EOB now. (Under the
        // unreachable scroll-back mode 2 the window would still cover it.)
        assert_eq!(f.get(69, ReadType::SoftAhead), EOB);
        assert_eq!(f.seekcount(), 1);
        // ... and the following bytes are served sequentially again.
        for i in 71..80 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 1);
    }

    /// Brief step-1 test: hard-ahead far past the buffer resets it, seeks and
    /// serves the byte; the counter shows each reset seek.
    #[test]
    fn hard_ahead_far_forward_resets() {
        let mut f = mk(data(6000));
        assert_eq!(f.get(5000, ReadType::HardAhead), pat(5000));
        assert_eq!(f.seekcount(), 1);
        assert_eq!(f.get(5001, ReadType::Read), pat(5001));
        assert_eq!(f.seekcount(), 1);
        // Back to the start: outside the window [5000, 5016), far before it.
        assert_eq!(f.get(0, ReadType::HardAhead), pat(0));
        assert_eq!(f.seekcount(), 2);
        assert_eq!(f.get(1, ReadType::Read), pat(1));
        assert_eq!(f.seekcount(), 2);
    }

    /// Brief step-1 test: soft-ahead beyond `pos_inp + blk_sze` returns `EOB`
    /// without seeking, while positions still within one block are served by
    /// an append read.
    #[test]
    fn soft_ahead_far_forward_returns_eob() {
        let mut f = mk(data(6000));
        for i in 0..16 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        // pos_inp == 16: position 32 == pos_inp + blk_sze is out of reach,
        // position 31 is not.
        assert_eq!(f.get(32, ReadType::SoftAhead), EOB);
        assert_eq!(f.get(5000, ReadType::SoftAhead), EOB);
        assert_eq!(f.seekcount(), 0, "EOB must not access the file");
        assert_eq!(f.get(31, ReadType::SoftAhead), pat(31));
        assert_eq!(f.seekcount(), 0, "append reads do not seek");
    }

    /// Brief step-1 test: end of file is reported at `len` and beyond,
    /// repeatedly; the last byte stays reachable afterwards.
    #[test]
    fn eof_at_end_only() {
        let mut f = mk(data(256));
        assert_eq!(f.get(256, ReadType::Read), EOF);
        assert_eq!(f.seekcount(), 1, "the first EOF probes the file once");
        assert_eq!(f.get(256, ReadType::Read), EOF);
        assert_eq!(f.get(300, ReadType::Read), EOF);
        assert_eq!(f.seekcount(), 1, "later EOFs come from pos_eof");
        assert_eq!(f.get(255, ReadType::Read), pat(255));
        assert_eq!(
            f.seekcount(),
            2,
            "seek-&-reset (bool collapse, §15.8): one repositioning seek"
        );
        assert_eq!(f.get(255, ReadType::Read), pat(255));
        assert_eq!(f.get(256, ReadType::Read), EOF);
    }

    /// Brief step-1 test: bytes already fetched stay available; re-reading
    /// them (also as soft-ahead, which must not `EOB` inside the buffer)
    /// performs no seeks.
    #[test]
    fn scroll_back_serves_history() {
        let mut f = mk(data(2048));
        for i in 0..600 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(5, ReadType::Read), pat(5));
        assert_eq!(f.get(5, ReadType::SoftAhead), pat(5));
        assert_eq!(f.get(599, ReadType::Read), pat(599));
        assert_eq!(f.seekcount(), 0);
    }

    /// Brief step-1 test: with a small wrapping buffer (64 bytes, 16-byte
    /// blocks), a full pass over a 256-byte file followed by a near-before-
    /// window read and a re-read of the first 64 bytes serves every byte
    /// exactly.
    #[test]
    fn wraparound_buffer_integrity() {
        let mut f = JFileAhead::new(Cursor::new(data(256)), "Tst", 64, 16);
        // Pass 1: plain sequential reads around the ring.
        for i in 0..256 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0);
        // Window is [192, 256): 190 is just before it, so the buffer takes
        // the seek-&-reset path (1 seek; §15.8 bool collapse — the retained
        // scroll-back arm would need 2). The window head is served from the
        // reset buffer, the rest appended through the wrapped ring, ending
        // in a partial read at the file end.
        assert_eq!(f.get(190, ReadType::Read), pat(190));
        assert_eq!(f.seekcount(), 1);
        for i in 191..256 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(256, ReadType::Read), EOF);
        assert_eq!(f.get(256, ReadType::Read), EOF);
        // Pass 2: far before the window, the buffer resets and the head is
        // re-read correctly through the wrapped ring.
        for i in 0..64 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 2);
    }

    /// Brief step-1 test: a failing `Seek` makes `get` return `-EXI_SEK`:
    /// both for the very first repositioning read and for any later
    /// out-of-window read (each mode-1 reset performs exactly one seek).
    #[test]
    fn seek_error_returns_neg_exi_sek() {
        // Every seek fails: append reads still work (no seek), resets fail.
        let mut f = JFileAhead::new(FlakySeek::failing(0), "Tst", 1024, 16);
        assert_eq!(f.get(0, ReadType::Read), pat(0));
        assert_eq!(f.get(5000, ReadType::HardAhead), -EXI_SEK);
        // The first seek succeeds, further seeks fail: the near-before-window
        // read spends its single seek-&-reset seek (§15.8 bool collapse), the
        // next out-of-window read then fails on its own seek.
        let mut f = JFileAhead::new(FlakySeek::failing(1), "Tst", 1024, 16);
        for i in 0..1100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(70, ReadType::Read), pat(70));
        assert_eq!(f.get(0, ReadType::Read), -EXI_SEK);
    }

    /// Cursor whose `seek` fails after `ok` successful seeks, like a `FILE*`
    /// whose `fseek` returns nonzero.
    struct FlakySeek {
        inner: Cursor<Vec<u8>>,
        ok: usize,
    }

    impl FlakySeek {
        fn failing(ok: usize) -> FlakySeek {
            FlakySeek {
                inner: Cursor::new(data(2048)),
                ok,
            }
        }
    }

    impl Read for FlakySeek {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.inner.read(buf)
        }
    }

    impl Seek for FlakySeek {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            if self.ok == 0 {
                Err(std::io::Error::other("fseek failed"))
            } else {
                self.ok -= 1;
                self.inner.seek(pos)
            }
        }
    }
}
