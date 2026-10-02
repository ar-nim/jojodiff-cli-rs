//! Buffered look-ahead `JFile` reader — the default input mode of both tools
//! (C++ `JFileAhead` at 0.8.5, spec §18.E).
//!
//! 1:1 port of `reference/jojodiff-0.8.5/src/JFileAhead.cpp:39-431` plus the
//! collapsed stdio adapter (`JFileAheadStdio.cpp`; one engine over seekable
//! AND sequential streams, spec §21.11). A circular buffer of `buf_sze` bytes
//! is refilled in `blk_sze` chunks by `readblocks` (`:392-431`); the
//! `get_fromfile` decision tree (`:269-307`) chooses between Append, Reset
//! and Scrollback (`eBufOpr`, `JFileAhead.h:114`): the 0.8.1 `bool liSek`
//! collapse is FIXED upstream (spec §20 row §15.8) — Scrollback preserves the
//! buffer history and costs two seeks (back, then forward again,
//! `:341-382`), Reset block-aligns (`:311-317`, the sequential variant keeps
//! the tail), Append fills forward. Before-buffer reads are impossible on
//! sequential files and return the raw sentinels of `eBufDne`
//! (`JFileAhead.h:115`: `SeekError = EXI_SEK`, `ReadError = EXI_RED`);
//! soft-ahead is bounded by the lookahead base `mzPosBse` (`:303-304`). EOF
//! is probed at construction (`chkSeq`, `JFile.cpp:37-46`) and re-latched by
//! short reads (`:415-423`). There is no `Buffer out of bounds` abort
//! anymore — 0.8.5 clamps and wraps (spec §18.E).
//!
//! # Deviation from the C++ (documented)
//!
//! **Negative positions read as EOF.** An EOF read resets the read cursor to
//! -1 (`get_frombuffer`, `JFileAhead.cpp:142-147`); the C++ `getbuf` EOF gate
//! (`azPos >= mzPosEof`) does not catch a re-read at that negative cursor, so
//! `JPatcht`'s zero-argument `get()` — which re-issues there after every EOF
//! — scrolls the buffer back and receives a stale byte as data. In release
//! builds the patch decoder then never sees EOF again and spins forever on
//! corrupt patches (reproduced byte-for-byte with the 0.8.5 oracle: a
//! DEL-only patch streams zero bytes, a trailing lone ESC cycles
//! `x ESC FF 00 ESC MOD`); debug builds trip the always-on `getbuf` assert
//! instead (exit 6). The port ends the file at any negative position
//! (`getbuf_off`: `pos < 0` → EOF), matching `JFileMem` and terminating the
//! decode.
//!
//! # Debug surface (spec §18.G, `debug` feature)
//!
//! Three sites are ported: the DBGBUF `ufFabOpn` open line
//! (`JFileAhead.cpp:79-83`), the DBGRED double-verify block in
//! `get_frombuffer` (`:149-186`, new at 0.8.5) and the **always-on**
//! `getbuf` invariant asserts (`:240-251`, `exit(-EXI_SEK)` on violation —
//! this is what fires in `-t` Test mode). All 0.8.1 `ufFabGet` trace sites
//! are gone at 0.8.5. `%p` values print the equivalent Rust addresses;
//! only the line *shape* is oracle-pinnable.

use std::io::{Read, Seek, SeekFrom};

use super::{JFile, ReadType};
#[cfg(feature = "debug")]
use crate::defs::p8;
use crate::defs::{EOB, EOF, EXI_OK, EXI_RED, EXI_SEK};
use crate::jdebug::dbg_print;
#[cfg(feature = "debug")]
use crate::jdebug::{DBGBUF, DBGRED, dbg};
#[cfg(feature = "debug")]
use std::process;
#[cfg(feature = "debug")]
use std::sync::Mutex;

/// The double-verify scratch buffer (`static jchar lcTst[1024*1024]`,
/// `JFileAhead.cpp:165`) — zero-initialized and shared, like the C++ static.
#[cfg(feature = "debug")]
static LC_TST: Mutex<[u8; 1024 * 1024]> = Mutex::new([0; 1024 * 1024]);

/// Buffer operation (`eBufOpr`, `JFileAhead.h:114`).
#[derive(Clone, Copy)]
enum BufOpr {
    Append,
    Reset,
    Scrollback,
}

/// Result of `get_fromfile` (`eBufDne`, `JFileAhead.h:115`): `EndOfFile =
/// EOF` (-1), `EndOfBuffer = EOB` (-2), `SeekError = EXI_SEK` (-6),
/// `ReadError = EXI_RED` (-8).
enum BufDone {
    Added,
    EndOfFile,
    EndOfBuffer,
    SeekError,
    ReadError,
}

/// Buffered look-ahead byte source, 1:1 with C++ `JFileAhead`
/// (`src/JFileAhead.h`).
///
/// State (C++ names in comments): `buf`/`mpBuf` is a circular buffer
/// (`mpMax` = its end), `ptr_inp`/`mpInp` the write position,
/// `buf_usd`/`miBufUsd` the number of valid bytes, `pos_inp`/`mzPosInp` the
/// file offset of the next unread chunk byte and `pos_bse`/`mzPosBse` the
/// soft look-ahead base; `pos_red`/`mzPosRed` + `ptr_red`/`mpRed` +
/// `red_sze`/`miRedSze` are the base-class sequential read cursor
/// (`JFile.h:177-181`), and `pos_eof`/`mzPosEof` the known end of file
/// (`i64::MAX` = `MAX_OFF_T` = unknown, i.e. sequential).
pub struct JFileAhead<R: Read + Seek> {
    file: R,
    /// File identifier for debug prints (C++ `msJid`, `JFile.h:175`).
    #[cfg_attr(not(feature = "debug"), allow(dead_code))]
    fid: String,
    seq: bool,      // mbSeq (JFile.h:176)
    red_sze: i64,   // miRedSze (JFile.h:177)
    ptr_red: usize, // mpRed (JFile.h:178)
    pos_inp: i64,   // mzPosInp (JFile.h:179)
    pos_red: i64,   // mzPosRed (JFile.h:180)
    pos_eof: i64,   // mzPosEof (JFile.h:181)
    seeks: i64,     // mlFabSek (JFile.h:183)
    buf_sze: i64,   // mlBufSze (JFileAhead.h:161)
    blk_sze: i64,   // miBlkSze (JFileAhead.h:162)
    buf_usd: i64,   // miBufUsd (JFileAhead.h:165)
    buf: Vec<u8>,   // mpBuf..mpMax (JFileAhead.h:166-167)
    ptr_inp: usize, // mpInp (JFileAhead.h:168)
    pos_bse: i64,   // mzPosBse (JFileAhead.h:169)
}

impl<R: Read + Seek> JFileAhead<R> {
    /// Buffers `file` with a `buf_sze`-byte circular buffer read in
    /// `blk_sze`-byte chunks (C++ `JFileAhead::JFileAhead`,
    /// `JFileAhead.cpp:39-84`; a zero `buf_sze` falls back to 1024, `:41`).
    /// This is the collapsed adapter constructor too: it ends with the
    /// chkSeq seek-EOF probe like `JFileAheadStdio.cpp:25-31`, so
    /// non-seekable streams are auto-detected as sequential (spec §21.11).
    pub fn new(file: R, fid: &str, buf_sze: i64, blk_sze: i32) -> Self {
        // Block size cannot be zero and cannot be larger than buffer size;
        // buffer size must be aligned on block size (JFileAhead.cpp:44-57).
        let mut buf_sze = if buf_sze == 0 { 1024 } else { buf_sze };
        let mut blk_sze = i64::from(blk_sze);
        if blk_sze == 0 {
            dbg_print(format_args!(
                "Warning: Block size cannot be zero: set to {}.\n",
                1
            ));
            blk_sze = 1;
        }
        if buf_sze % blk_sze != 0 {
            buf_sze -= buf_sze % blk_sze;
            dbg_print(format_args!(
                "Warning: Buffer size misaligned with block size: set to {}.\n",
                buf_sze
            ));
        }
        if buf_sze == 0 {
            buf_sze = blk_sze;
            dbg_print(format_args!(
                "Warning: Buffer size cannot be zero: set to {}.\n",
                buf_sze
            ));
        }

        // Allocate buffer + initialize buffer logic (JFileAhead.cpp:59-77).
        let mut fab = JFileAhead {
            file,
            fid: fid.to_string(),
            seq: false,
            red_sze: 0,
            ptr_red: 0,
            pos_inp: 0,
            pos_red: 0,
            pos_eof: i64::MAX, // MAX_OFF_T
            seeks: 0,
            buf_sze,
            blk_sze,
            buf_usd: 0,
            buf: vec![0_u8; buf_sze as usize],
            ptr_inp: 0,
            pos_bse: 0,
        };

        /* Debug: open trace (JFileAhead.cpp:79-83); the pointers are the
         * buffer allocation's start/end like the C++ `mpBuf`/`mpMax`. */
        #[cfg(feature = "debug")]
        if dbg(DBGBUF) {
            let range = fab.buf.as_ptr_range();
            dbg_print(format_args!(
                "ufFabOpn({}):(buf={:p},max={:p},sze={})\n",
                fab.fid, range.start, range.end, fab.buf_sze,
            ));
        }

        // chkSeq (JFile.cpp:37-46) — adapter ctor tail (JFileAheadStdio.cpp:30).
        fab.chk_seq();

        fab
    }

    /// `JFile::chkSeq` (`JFile.cpp:37-46`): check if the file can be seeked
    /// by seeking the EOF position; a failing probe marks the file sequential
    /// and leaves the EOF position unknown (`MAX_OFF_T`).
    fn chk_seq(&mut self) {
        if !self.seq {
            self.pos_eof = self.jeofpos();
            if self.pos_eof < 0 {
                self.seq = true;
                self.pos_eof = i64::MAX;
            }
        }
    }

    /// C `jfseek(mpFil, azPos, SEEK_SET)` (`JFileAheadStdio.cpp:43-48`):
    /// `EXI_OK` or `EXI_SEK`. A negative offset cannot succeed, like C
    /// `fseek` on most platforms (Rust's `Cursor` would accept it).
    fn jseek(&mut self, pos: i64) -> i32 {
        if pos < 0 || self.file.seek(SeekFrom::Start(pos as u64)).is_err() {
            EXI_SEK
        } else {
            EXI_OK
        }
    }
}

/// C `jfread` at the current file position (`JFileAheadStdio.cpp:70-72`):
/// fills `out` completely, looping until `out.len()` bytes, end of file or
/// error — like glibc `fread`, a read error surfaces as a short read.
fn jread<R: Read>(file: &mut R, out: &mut [u8]) -> usize {
    let mut done = 0_usize;
    while done < out.len() {
        match file.read(&mut out[done..]) {
            Ok(0) => break,
            Ok(n) => done += n,
            Err(_) => break,
        }
    }
    done
}

impl<R: Read + Seek> JFileAhead<R> {
    /// Tries to get data from the buffer; calls
    /// [`get_fromfile`](Self::get_fromfile) via
    /// [`getbuf_off`](Self::getbuf_off) if that is not possible (C++
    /// `get_frombuffer`, `JFileAhead.cpp:134-199`).
    fn get_frombuffer(&mut self, pos: i64, typ: ReadType) -> i32 {
        // lzLen is set on every null-return path of getbuf
        // (JFileAhead.cpp:222-225).
        let mut len: i64 = 0;
        match self.getbuf_off(pos, &mut len, typ) {
            None => {
                // EOF, EOB or any other problem (JFileAhead.cpp:142-147)
                self.pos_red = -1;
                self.ptr_red = 0; // C++: mpRed = null; kept safe by red_sze == 0
                self.red_sze = 0;
                len as i32
            }
            Some(off) => {
                #[cfg(feature = "debug")]
                self.verify_buffer(pos, len, off, typ);

                // prepare next reading position (but do not increase lpDta!!!)
                // (JFileAhead.cpp:188-194)
                self.pos_red = pos + 1;
                self.red_sze = len - 1;
                self.ptr_red = off + 1; // mpRed = lpDta + 1
                if self.ptr_red == self.buf.len() {
                    self.ptr_red = 0; // mpRed = mpBuf
                }

                // return data at current position (JFileAhead.cpp:197)
                self.buf[off] as i32
            }
        }
    }

    /// The DBGRED double-verify block (JFileAhead.cpp:149-186): re-read the
    /// requested position from the file and compare with the buffer, to
    /// detect buffer logic and contents failures.
    #[cfg(feature = "debug")]
    fn verify_buffer(&mut self, pos: i64, len: i64, off: usize, typ: ReadType) {
        if !dbg(DBGRED) {
            return;
        }
        let byte = self.buf[off];

        // detect buffer logic failure (JFileAhead.cpp:152-163)
        let mut lp_dbg = self.ptr_inp as i64 - (self.pos_inp - pos);
        if lp_dbg < 0 || lp_dbg >= self.buf_sze {
            lp_dbg += self.buf_sze;
        }
        if lp_dbg != off as i64 {
            dbg_print(format_args!(
                "JFileAhead({},{},{})->{}={:2x} (mem {:p}): pos-error !\n",
                self.fid,
                p8(pos),
                typ as i32,
                byte as char,
                byte as u32,
                self.buf.as_ptr().wrapping_add(off),
            ));
        }

        // detect buffer contents failure (JFileAhead.cpp:164-184)
        let mut lc_tst = LC_TST.lock().unwrap();
        let _ = self.jseek(pos); // result ignored, like the C++
        let li_len = if len > lc_tst.len() as i64 {
            lc_tst.len() as i64
        } else {
            len
        };
        let li_dne = jread(&mut self.file, &mut lc_tst[..li_len as usize]);
        if li_dne != li_len as usize {
            dbg_print(format_args!(
                "JFileAhead({},{},{})->{}={:2x} (mem {:p}): len-error !\n",
                self.fid,
                p8(pos),
                typ as i32,
                byte as char,
                byte as u32,
                self.buf.as_ptr().wrapping_add(off),
            ));
        }
        if lc_tst[..li_len as usize] != self.buf[off..off + li_len as usize] {
            dbg_print(format_args!(
                "JFileAhead({},{},{})->{}={:2x} (mem {:p}): buf-error !\n",
                self.fid,
                p8(pos),
                typ as i32,
                byte as char,
                byte as u32,
                self.buf.as_ptr().wrapping_add(off),
            ));
        }
        let _ = self.jseek(self.pos_inp); // restore, like the C++ jseek(mzPosInp)
    }

    /// Get access to buffered read: everything of C++ `JFileAhead::getbuf`
    /// (`JFileAhead.cpp:210-254`) except returning the slice — the offset
    /// into the buffer replaces the `jchar*`, so
    /// [`get_frombuffer`](Self::get_frombuffer) can derive `mpRed` from it.
    ///
    /// On success `*len` holds the number of available bytes; on failure it
    /// holds the EOF/EOB/EXI sentinel.
    fn getbuf_off(&mut self, pos: i64, len: &mut i64, typ: ReadType) -> Option<usize> {
        if pos >= self.pos_eof || pos < 0 {
            /* eof (JFileAhead.cpp:213-216). `pos < 0` is the port's
             * deviation 4 (module docs): the C++ gate is only
             * `azPos >= mzPosEof`, so the negative read cursor left by an
             * EOF read escapes it, scrolls the buffer back and serves a
             * stale byte — 0.8.5's release build then never sees EOF again
             * and spins forever (verified against the oracle), while its
             * debug build trips the getbuf assert (exit 6). Negative
             * positions cannot be valid in any file, so the port ends the
             * file there, like `JFileMem` (`pos < 0` → EOF). */
            *len = i64::from(EOF);
            return None;
        } else if pos < self.pos_inp && pos >= self.pos_inp - self.buf_usd {
            // Data is already in the buffer (JFileAhead.cpp:217-218)
        } else {
            // Get data from underlying file (JFileAhead.cpp:220-228)
            match self.get_fromfile(pos, typ) {
                BufDone::EndOfBuffer => {
                    *len = i64::from(EOB);
                    return None;
                }
                BufDone::EndOfFile => {
                    *len = i64::from(EOF);
                    return None;
                }
                BufDone::SeekError => {
                    *len = i64::from(EXI_SEK);
                    return None;
                }
                BufDone::ReadError => {
                    *len = i64::from(EXI_RED);
                    return None;
                }
                BufDone::Added => {} // data added
            }
        }

        // Calculate position of pos (JFileAhead.cpp:231-238)
        let mut az_len = self.pos_inp - pos;
        *len = az_len;
        let off: i64 = if az_len <= self.ptr_inp as i64 {
            self.ptr_inp as i64 - az_len
        } else {
            let off = self.ptr_inp as i64 + self.buf_sze - az_len;
            az_len = self.buf.len() as i64 - off; // azLen = mpMax - lpDta
            *len = az_len;
            off
        };

        #[cfg(feature = "debug")]
        {
            // Always-on invariant asserts (§18.G, JFileAhead.cpp:240-251).
            if off < 0 || off as usize >= self.buf.len() {
                dbg_print(format_args!(
                    "JFileAhead::getbuf({},{},{},{})->   (sto {:p}) out of bounds !\n",
                    self.fid,
                    pos,
                    az_len,
                    typ as i32,
                    self.buf.as_ptr().wrapping_add(off as usize),
                ));
                process::exit(-EXI_SEK);
            }
            if pos >= self.pos_inp || pos < self.pos_inp - self.buf_usd {
                dbg_print(format_args!(
                    "JFileAhead::getbuf({},{},{},{})->{:2x} (sto {:p}) failed !\n",
                    self.fid,
                    pos,
                    az_len,
                    typ as i32,
                    self.buf[off as usize] as u32,
                    self.buf.as_ptr().wrapping_add(off as usize),
                ));
                process::exit(-EXI_SEK);
            }
        }

        Some(off as usize)
    }

    /// Retrieve requested position into the buffer, trying to keep the buffer
    /// as large as possible (C++ `get_fromfile`, `JFileAhead.cpp:269-387`).
    fn get_fromfile(&mut self, pos: i64, typ: ReadType) -> BufDone {
        /* Preparation: Check what should be done and set liSek accordingly
         * (JFileAhead.cpp:277-307) */
        let li_sek: BufOpr;
        if pos < self.pos_inp - self.buf_usd {
            // Reading before the start of the buffer:
            // - not allowed in sequential nor soft-reading mode
            // - either cancel the whole buffer: easiest, but we loose all
            //   data in the buffer
            // - either scroll back the buffer: harder, but we may not loose
            //   all data in the buffer
            if typ == ReadType::SoftAhead {
                return BufDone::EndOfBuffer;
            } else if self.seq {
                if typ == ReadType::HardAhead {
                    return BufDone::EndOfBuffer;
                } else {
                    return BufDone::SeekError;
                }
            } else if pos + self.buf_sze - self.blk_sze > self.pos_inp - self.buf_usd {
                li_sek = BufOpr::Scrollback;
            } else {
                li_sek = BufOpr::Reset;
            }
        } else if pos >= self.pos_inp + self.buf_sze {
            // Advancing more than the size of the buffer:
            // - not allowed when soft-reading
            // - just reset
            if typ == ReadType::SoftAhead {
                return BufDone::EndOfBuffer;
            } else {
                li_sek = BufOpr::Reset;
            }
        } else {
            // Append to the buffer if possible
            if typ == ReadType::SoftAhead && pos > self.pos_bse + self.buf_sze - self.blk_sze {
                return BufDone::EndOfBuffer;
            } else {
                li_sek = BufOpr::Append;
            }
        }

        match li_sek {
            BufOpr::Reset => {
                // JFileAhead.cpp:310-333
                if !self.seq {
                    // Calculate position and length
                    self.pos_inp = (pos / self.blk_sze) * self.blk_sze;
                } else {
                    // In sequential mode: jump forward and then append, keep
                    // the buffer as large as possible
                    self.pos_inp =
                        ((pos - self.buf_sze + self.blk_sze) / self.blk_sze) * self.blk_sze;
                }

                // Reset buffer
                self.ptr_inp = 0; // mpInp = mpBuf
                self.pos_bse = self.pos_inp;
                self.buf_usd = 0;

                // Seek
                if self.jseek(self.pos_inp) != EXI_OK {
                    return BufDone::SeekError;
                }
                self.seeks += 1; // mlFabSek++

                // Read
                let (inp, pos, dne) = self.readblocks(self.ptr_inp, self.pos_inp, pos);
                self.ptr_inp = inp;
                self.pos_inp = pos;
                if dne == EOF {
                    return BufDone::EndOfFile;
                }
            }

            BufOpr::Append => {
                // JFileAhead.cpp:335-339
                let (inp, pos, dne) = self.readblocks(self.ptr_inp, self.pos_inp, pos);
                self.ptr_inp = inp;
                self.pos_inp = pos;
                if dne == EOF {
                    return BufDone::EndOfFile;
                }
            }

            BufOpr::Scrollback => {
                // JFileAhead.cpp:341-383
                // Calculate scrollback position
                let lz_pos = (pos / self.blk_sze) * self.blk_sze; // position to seek
                let mut lz_len = self.pos_inp - lz_pos; // new potential buffer length
                let mut lp_inp: i64 = self.ptr_inp as i64 - lz_len;
                if lz_len > self.ptr_inp as i64 {
                    lp_inp += self.buf_sze;
                }

                // Make room in the buffer for the scrollback
                if lz_len > self.buf_sze {
                    lz_len -= self.buf_sze;
                    self.buf_usd -= lz_len;
                    self.pos_inp = lz_pos + self.buf_sze;
                    if lz_len > self.ptr_inp as i64 {
                        lp_inp += self.buf_sze;
                    }
                    self.ptr_inp = lp_inp as usize; // mpInp = lpInp
                }

                // Seek
                if self.jseek(lz_pos) != EXI_OK {
                    return BufDone::SeekError;
                }
                self.seeks += 1;

                // Read loop
                let (lp_inp, lz_pos, dne) =
                    self.readblocks(lp_inp as usize, lz_pos, self.pos_inp - self.buf_usd - 1);
                if dne == EOF {
                    // A scrollback cannot issue an EOF unless there's a
                    // hardware error or the file is being truncated while
                    // we're reading it. In both cases, the outcome will
                    // probably be unusable. The buffer variables are set here
                    // just for the sake of "correctness".
                    // (JFileAhead.cpp:367-376)
                    self.ptr_inp = lp_inp; // mpInp = lpInp
                    self.pos_inp = lz_pos; // mzPosInp = lzPos
                    self.buf_usd = i64::from(dne); // miBufUsd = liDne (EOF = -1)
                    return BufDone::ReadError;
                }

                // @Seek (JFileAhead.cpp:378-381)
                if self.jseek(self.pos_inp) != EXI_OK {
                    return BufDone::SeekError;
                }
                self.seeks += 1;
            }
        }

        BufDone::Added // JFileAhead.cpp:386
    }

    /// Read blocks till the specified end (C++ `readblocks`,
    /// `JFileAhead.cpp:392-431`). The C++ in/out parameters `apInp`/`azInp`
    /// come back as the updated buffer offset and file position; `miBufUsd`
    /// and `mzPosEof` are members and are updated in place. Returns the last
    /// read's byte count, or [`EOF`] when the requested end is at/past the
    /// latched EOF position.
    fn readblocks(&mut self, mut inp_off: usize, mut pos: i64, end: i64) -> (usize, i64, i32) {
        let mut tdo: i64; // Number of bytes to read
        let mut done: i32 = 0; // Number of bytes read

        // Read loop
        while pos <= end {
            // Prepare
            tdo = self.blk_sze;
            if inp_off == self.buf.len() {
                inp_off = 0; // apInp = mpBuf
            } else if ((self.buf.len() - inp_off) as i64) < tdo {
                tdo = (self.buf.len() - inp_off) as i64; // mpMax - apInp
            }

            // Read
            done = jread(
                &mut self.file,
                &mut self.buf[inp_off..inp_off + tdo as usize],
            ) as i32;

            // Update buffer vars
            inp_off += done as usize;
            pos += i64::from(done);
            self.buf_usd += i64::from(done);

            // Handle EOF
            if done < tdo as i32 {
                self.pos_eof = pos;
                if self.buf_usd > self.buf_sze {
                    self.buf_usd = self.buf_sze;
                }
                if end >= self.pos_eof {
                    return (inp_off, pos, EOF);
                } else {
                    return (inp_off, pos, done);
                }
            }
        }

        // Update buffer vars
        if self.buf_usd > self.buf_sze {
            self.buf_usd = self.buf_sze;
        }

        (inp_off, pos, done)
    }
}

impl<R: Read + Seek> JFile for JFileAhead<R> {
    /// Gets one byte: the base-class sequential fast path over the read
    /// cursor, else `get_frombuffer` (C++ `JFile::get`, `JFile.h:73-81`;
    /// replicated here because the trait cannot hold the cursor state).
    fn get(&mut self, pos: i64, typ: ReadType) -> i32 {
        if pos == self.pos_red && self.red_sze > 0 {
            // mzPosRed++; miRedSze--; return *mpRed++;
            self.pos_red += 1;
            self.red_sze -= 1;
            let byte = self.buf[self.ptr_red] as i32;
            self.ptr_red += 1;
            if self.ptr_red == self.buf.len() {
                self.ptr_red = 0;
            }
            byte
        } else {
            self.get_frombuffer(pos, typ)
        }
    }

    /// Number of seeks performed (JFileAhead.cpp:93-95).
    fn seekcount(&self) -> i64 {
        self.seeks
    }

    /// Set lookahead base (JFileAhead.cpp:122-126).
    fn set_lookahead_base(&mut self, base: i64) {
        self.pos_bse = base;
    }

    /// Sequential flag (C++ `isSequential` reads `mbSeq`, `JFile.h:109`),
    /// set by the constructor's chkSeq probe.
    fn is_sequential(&self) -> bool {
        self.seq
    }

    /// Seek-EOF abstraction (`JFileAheadStdio.cpp:55-63`): seek to the end,
    /// tell, seek back to 0 — the restore seek's result is ignored, like the
    /// unchecked C++ `jfseek(mpFil, 0, SEEK_SET)`.
    fn jeofpos(&mut self) -> i64 {
        let eof = match self.file.seek(SeekFrom::End(0)) {
            Ok(p) => p as i64,
            Err(_) => return i64::from(EXI_SEK),
        };
        let _ = self.file.seek(SeekFrom::Start(0));
        eof
    }

    /// Position of the buffer (JFileAhead.cpp:102-104).
    fn get_buf_pos(&self) -> i64 {
        self.pos_inp - self.buf_usd
    }

    /// Size of the buffer (JFileAhead.cpp:111-113).
    fn get_buf_sze(&self) -> i64 {
        self.buf_sze
    }

    /// Get access to buffered read (JFileAhead.cpp:210-254); the run never
    /// wraps the ring, so a slice can represent it.
    fn getbuf(&mut self, pos: i64, len: &mut i64, typ: ReadType) -> Option<&[u8]> {
        let off = self.getbuf_off(pos, len, typ)?;
        Some(&self.buf[off..off + *len as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::defs::EXI_RED;
    use std::cell::Cell;
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

    /// Pipe-like sequential stream: every seek fails, so the constructor's
    /// EOF probe (chkSeq, `JFile.cpp:37-46`) detects a sequential file.
    fn pipe(n: usize) -> JFileAhead<FlakySeek> {
        JFileAhead::new(FlakySeek::failing(0, n), "Tst", 1024, 256)
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

    /// Brief step-1 test: hard-ahead far past the buffer resets it, seeks and
    /// serves the byte; the counter shows each reset seek. The reset lands on
    /// the block-aligned position (`JFileAhead.cpp:313`).
    #[test]
    fn hard_ahead_far_forward_resets() {
        let mut f = mk(data(6000));
        assert_eq!(f.get(5000, ReadType::HardAhead), pat(5000));
        assert_eq!(f.seekcount(), 1);
        assert_eq!(f.get(5001, ReadType::Read), pat(5001));
        assert_eq!(f.seekcount(), 1);
        // Back to the start: outside the window [4992, 5008), far before it
        // (0 + 1024 - 16 = 1008 <= 4992): reset, not scrollback.
        assert_eq!(f.get(0, ReadType::HardAhead), pat(0));
        assert_eq!(f.seekcount(), 2);
        assert_eq!(f.get(1, ReadType::Read), pat(1));
        assert_eq!(f.seekcount(), 2);
    }

    /// The EOF position is probed at construction (chkSeq → jeofpos,
    /// `JFileAheadStdio.cpp:30,55-63`): reads at/past the end return EOF
    /// without any file access, and the last byte stays reachable.
    #[test]
    fn eof_known_from_chkseq() {
        let mut f = mk(data(256));
        assert_eq!(f.get(256, ReadType::Read), EOF);
        assert_eq!(f.seekcount(), 0, "the probe's seeks are not counted");
        assert_eq!(f.get(300, ReadType::Read), EOF);
        assert_eq!(f.seekcount(), 0, "EOF needs no file access");
        assert_eq!(f.get(255, ReadType::Read), pat(255));
        assert_eq!(f.seekcount(), 0, "append read of the whole file");
        assert_eq!(f.get(256, ReadType::Read), EOF);
    }

    /// After an EOF read the read cursor sits at -1 (`get_frombuffer`'s
    /// reset, `JFileAhead.cpp:142-147`); re-reading at that negative
    /// position must return EOF again — never scroll back and serve a stale
    /// buffer byte. (Upstream 0.8.5 escapes its EOF gate with `-1` and spins
    /// forever on corrupt patches through this window in release builds;
    /// its debug build asserts. The port returns EOF, matching `JFileMem`
    /// and terminating the decode — module docs, deviation 4.)
    #[test]
    fn get_at_negative_position_after_eof_returns_eof() {
        let mut f = mk(data(3));
        for i in 0..3 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(3, ReadType::Read), EOF, "EOF at end");
        // The C++ zero-arg get() re-issues at the reset cursor (-1).
        assert_eq!(f.get(-1, ReadType::Read), EOF, "EOF is sticky at -1");
        assert_eq!(f.get(-1, ReadType::Read), EOF, "EOF stays sticky at -1");
        assert_eq!(f.get(-42, ReadType::Read), EOF, "any negative position");
    }

    /// Soft-ahead appends are bounded by the lookahead base:
    /// `pos > mzPosBse + mlBufSze - miBlkSze` → EOB without I/O
    /// (`JFileAhead.cpp:303-304`); the position == the bound is served. The
    /// base is set implicitly by a reset (`:321`).
    #[test]
    fn soft_append_bounded_by_implicit_base() {
        let mut f = mk(data(6000));
        for i in 0..16 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        // Base is 0: bound = 0 + 1024 - 16 = 1008.
        assert_eq!(f.get(1008, ReadType::SoftAhead), pat(1008));
        assert_eq!(f.get(1040, ReadType::SoftAhead), EOB);
        assert_eq!(f.get(5000, ReadType::SoftAhead), EOB);
        assert_eq!(f.seekcount(), 0, "the bound EOBs without file access");
        // A far-forward hard read resets and re-bases (mzPosBse = 2992, the
        // block-aligned position): the new bound 2992 + 1024 - 16 = 4000
        // holds without set_lookahead_base.
        assert_eq!(f.get(3000, ReadType::HardAhead), pat(3000));
        assert_eq!(f.seekcount(), 1);
        assert_eq!(f.get(4009, ReadType::SoftAhead), EOB);
        assert_eq!(f.get(4000, ReadType::SoftAhead), pat(4000));
        assert_eq!(f.seekcount(), 1);
    }

    /// `set_lookahead_base` (`JFileAhead.cpp:122-126`) moves the soft-append
    /// bound; hard appends are unbounded (within one buffer of the input
    /// position) and never touch the base.
    #[test]
    fn set_lookahead_base_bounds_soft_append() {
        let mut f = JFileAhead::new(Cursor::new(data(8192)), "Tst", 1024, 512);
        for i in 0..16 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        // Hard appends only: pos_inp reaches 4096 without a reset.
        for i in 1..8 {
            assert_eq!(f.get(512 * i, ReadType::HardAhead), pat(512 * i));
        }
        f.set_lookahead_base(4000); // bound = 4000 + 1024 - 512 = 4512
        assert_eq!(f.get(4513, ReadType::SoftAhead), EOB);
        assert_eq!(f.seekcount(), 0, "the bound EOBs without file access");
        assert_eq!(f.get(4512, ReadType::SoftAhead), pat(4512));
        assert_eq!(f.seekcount(), 0, "append reads do not seek");
        assert_eq!(f.get(4513, ReadType::SoftAhead), pat(4513));
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

    /// Brief step-1 test: Scrollback is reachable (`JFileAhead.cpp:341-382`).
    /// Read forward 64 KiB (buf 16 KiB, blk 4 KiB), then read the byte just
    /// before the window: the buffer scrolls back block-aligned and the data
    /// is exact — at the cost of 2 seeks (back, then forward again).
    #[test]
    fn scrollback_two_seeks_full_window() {
        let mut f = JFileAhead::new(Cursor::new(data(65536)), "Tst", 16384, 4096);
        for i in 0..65536 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0);
        // Window [49152, 65536); 49151 is just before it and within one
        // buffer of it (49151 + 16384 - 4096 = 61439 > 49152): scrollback.
        assert_eq!(f.get(49151, ReadType::Read), pat(49151));
        assert_eq!(f.seekcount(), 2, "back-seek + forward re-seek");
        assert_eq!(f.get(49150, ReadType::Read), pat(49150));
        assert_eq!(f.seekcount(), 2, "served from the scrolled-back buffer");
        assert_eq!(f.get(65536, ReadType::Read), EOF);
    }

    /// A pipe-like sequential stream (every seek fails) is auto-detected at
    /// construction: forward reads never seek, and a before-buffer read
    /// returns the raw `EXI_SEK` sentinel without touching the file
    /// (`JFileAhead.cpp:284-288`); hard/soft before-buffer reads return EOB
    /// (`:285-286`).
    #[test]
    fn sequential_stream_detect_and_sentinel() {
        let mut f = pipe(4096);
        assert!(f.is_sequential(), "the seek-EOF probe detects the pipe");
        for i in 0..1024 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0, "forward reads never seek");
        assert_eq!(f.get(0, ReadType::Read), pat(0), "history stays buffered");
        for i in 1024..4096 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0);
        // Window [3072, 4096): 2000 is before it — not allowed on a
        // sequential file.
        assert_eq!(f.get(2000, ReadType::Read), EXI_SEK);
        assert_eq!(f.get(2000, ReadType::HardAhead), EOB);
        assert_eq!(f.get(2000, ReadType::SoftAhead), EOB);
        assert_eq!(f.seekcount(), 0, "the sentinel paths do no I/O");
        // Beyond the buffer: reset — but the sequential reset's seek fails on
        // the pipe (`JFileAhead.cpp:316-326`).
        assert_eq!(f.get(10000, ReadType::Read), EXI_SEK);
        assert_eq!(f.seekcount(), 0, "the failed seek is not counted");
        // The failed reset left the buffer invalid (buf_usd 0): everything
        // now counts as before-buffer.
        assert_eq!(f.get(4096, ReadType::Read), EXI_SEK);
    }

    /// On a sequential stream EOF is only learned from a short read
    /// (`readblocks`, `JFileAhead.cpp:415-423`) and then reported without
    /// file access.
    #[test]
    fn sequential_eof_latched_without_seek() {
        let mut f = pipe(4096);
        for i in 0..4096 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0);
        assert_eq!(f.get(4096, ReadType::Read), EOF, "the append read hits EOF");
        assert_eq!(f.get(4097, ReadType::Read), EOF);
        assert_eq!(f.get(5000, ReadType::Read), EOF);
        assert_eq!(f.seekcount(), 0);
    }

    /// A reset block-aligns the buffer start (`JFileAhead.cpp:313`), visible
    /// through `getBufPos` (`:102-104`).
    #[test]
    fn reset_block_aligns_getbufpos() {
        let mut f = JFileAhead::new(Cursor::new(data(8192)), "Tst", 1024, 64);
        for i in 0..100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(5000, ReadType::HardAhead), pat(5000));
        assert_eq!(f.seekcount(), 1);
        assert_eq!(f.get_buf_pos() % 64, 0, "reset lands on a block boundary");
        assert_eq!(f.get_buf_pos(), 4992);
        assert_eq!(f.get_buf_sze(), 1024);
        // A second reset from a far-before position (3000 + 1024 - 64 = 3960
        // <= 4992: reset, not scrollback) aligns the same way.
        assert_eq!(f.get(3000, ReadType::Read), pat(3000));
        assert_eq!(f.seekcount(), 2);
        assert_eq!(f.get_buf_pos() % 64, 0);
        assert_eq!(f.get_buf_pos(), 2944);
    }

    /// Short reads latch the EOF position (`JFileAhead.cpp:415-423`) and the
    /// used-count clamp (`:417-418`, `:427-428`) keeps the window bounded; a
    /// scrollback on the short file refills the head exactly and EOF stays
    /// latched.
    #[test]
    fn short_read_latches_eof() {
        let mut f = JFileAhead::new(Cursor::new(data(300)), "Tst", 256, 128);
        for i in 0..300 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(300, ReadType::Read), EOF);
        assert_eq!(f.get(400, ReadType::Read), EOF);
        assert_eq!(f.seekcount(), 0);
        // Window [44, 300): 43 is just before it and within one buffer.
        assert_eq!(f.get(43, ReadType::Read), pat(43));
        assert_eq!(f.seekcount(), 2, "scrollback: back-seek + forward re-seek");
        assert_eq!(f.get(150, ReadType::Read), pat(150));
        assert_eq!(f.get(300, ReadType::Read), EOF, "EOF stays latched");
    }

    /// Replaces the removed `Buffer out of bounds` exit(6) quirk test: tiny
    /// wrapping buffers (64/16) clamp and wrap correctly through scrollback,
    /// append-around-the-ring and a final reset.
    #[test]
    fn tiny_buffer_wraparound_integrity() {
        let mut f = JFileAhead::new(Cursor::new(data(256)), "Tst", 64, 16);
        for i in 0..256 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0);
        // Window [192, 256): 190 is just before it, within one buffer —
        // scrollback (2 seeks), not the 0.8.1 bool-collapsed reset.
        assert_eq!(f.get(190, ReadType::Read), pat(190));
        assert_eq!(f.seekcount(), 2);
        assert_eq!(f.get(191, ReadType::Read), pat(191));
        for i in 192..256 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(256, ReadType::Read), EOF);
        assert_eq!(f.get(256, ReadType::Read), EOF);
        // Far before the window: reset (0 + 64 - 16 = 48 <= 192).
        for i in 0..64 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 3);
    }

    /// Buffer == block size: every append wraps the ring; scrollback is
    /// out of reach (`pos + buf - blk` never exceeds the window start) and
    /// resets serve exact data.
    #[test]
    fn tiny_buffer_equal_block_sze() {
        let mut f = JFileAhead::new(Cursor::new(data(100)), "Tst", 16, 16);
        for i in 0..100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.seekcount(), 0);
        // Window [84, 100): 83 + 16 - 16 = 83 <= 84 — reset.
        assert_eq!(f.get(83, ReadType::Read), pat(83));
        assert_eq!(f.seekcount(), 1);
        assert_eq!(f.get(0, ReadType::Read), pat(0));
        assert_eq!(f.seekcount(), 2);
        assert_eq!(f.get(99, ReadType::Read), pat(99));
        assert_eq!(f.get(100, ReadType::Read), EOF);
    }

    /// A failing `Seek` surfaces the raw `EXI_SEK` sentinel
    /// (`JFileAhead.h:115` `SeekError = EXI_SEK`, `JFileAhead.cpp:224`). The
    /// constructor probe consumes the first `SeekFrom::Start` budget slot
    /// (its restore seek), so `ok` counts down from there.
    #[test]
    fn seek_error_returns_exi_sek() {
        // Every seek fails: append reads still work (no seek), resets fail.
        let mut f = JFileAhead::new(FlakySeek::failing(0, 2048), "Tst", 1024, 16);
        assert_eq!(f.get(0, ReadType::Read), pat(0));
        assert_eq!(f.get(5000, ReadType::HardAhead), EXI_SEK);
        // One algorithm seek succeeds: the scrollback's back-seek — its
        // forward re-seek then fails (`JFileAhead.cpp:379-380`).
        let mut f = JFileAhead::new(FlakySeek::failing(2, 2048), "Tst", 1024, 16);
        for i in 0..1100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(70, ReadType::Read), EXI_SEK);
        // Two algorithm seeks: the scrollback completes; the next scrollback
        // fails on its own back-seek.
        let mut f = JFileAhead::new(FlakySeek::failing(3, 2048), "Tst", 1024, 16);
        for i in 0..1100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        assert_eq!(f.get(70, ReadType::Read), pat(70));
        assert_eq!(f.seekcount(), 2);
        assert_eq!(f.get(0, ReadType::Read), EXI_SEK);
    }

    /// A scrollback whose refill read hits EOF mid-way reports ReadError
    /// (`JFileAhead.cpp:367-376`) — the raw `EXI_RED` sentinel — leaving the
    /// buffer invalid (`miBufUsd = EOF`, `:374`).
    #[test]
    fn scrollback_read_error_returns_exi_red() {
        let mut f = JFileAhead::new(TruncatingReader::after(5, 4096), "Tst", 1024, 256);
        for i in 0..1280 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        // Window [256, 1280); 255 is just before it. The scrollback's refill
        // read finds EOF (the device stopped serving data): ReadError.
        assert_eq!(f.get(255, ReadType::Read), EXI_RED);
        assert_eq!(f.seekcount(), 1, "only the back-seek was counted");
        // The failed refill latched pos_eof = 0 (the 0-byte read), so later
        // gets take the EOF branch — and buf_usd = EOF = -1 stays set.
        assert_eq!(f.get(0, ReadType::Read), EOF);
    }

    /// The trait-level `getbuf` fast path (`JFileAhead.cpp:210-254`): a slice
    /// of the buffer run at the requested position with its length; EOF and
    /// EOB come back as `None` with the sentinel in `len`.
    #[test]
    fn getbuf_direct_fast_path() {
        let mut f = mk(data(2048));
        for i in 0..100 {
            assert_eq!(f.get(i, ReadType::Read), pat(i), "byte {i}");
        }
        let mut len: i64 = -999;
        let run = f.getbuf(50, &mut len, ReadType::Read).expect("buffered");
        assert_eq!(len, 62, "bytes available from pos to the input position");
        assert_eq!(run[0] as i32, pat(50));
        assert_eq!(run[61] as i32, pat(111));
        assert_eq!(f.seekcount(), 0, "in-buffer getbuf does no I/O");
        let mut len = 0;
        assert!(f.getbuf(2048, &mut len, ReadType::Read).is_none());
        assert_eq!(len, i64::from(EOF));
        let mut len = 0;
        assert!(f.getbuf(1500, &mut len, ReadType::SoftAhead).is_none());
        assert_eq!(len, i64::from(EOB), "beyond-buffer soft read");
        assert_eq!(f.seekcount(), 0);
    }

    /// Cursor whose `seek` fails after `ok` successful `SeekFrom::Start`
    /// seeks, like a `FILE*` whose `fseek` returns nonzero. End-seeks (the
    /// constructor's EOF probe) do not consume the budget — with `ok == 0`
    /// every seek fails, modelling a pipe.
    struct FlakySeek {
        inner: Cursor<Vec<u8>>,
        ok: usize,
    }

    impl FlakySeek {
        fn failing(ok: usize, n: usize) -> FlakySeek {
            FlakySeek {
                inner: Cursor::new(data(n)),
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
                if matches!(pos, SeekFrom::Start(_)) {
                    self.ok -= 1;
                }
                self.inner.seek(pos)
            }
        }
    }

    /// Reader that serves the first `after` `read` calls from a `data(n)`
    /// cursor and then reports EOF — a file truncated while it is being
    /// read (`JFileAhead.cpp:368-370`). Seeks delegate freely (the EOF probe
    /// sees the full length, like an fstat-able but shrinking file).
    struct TruncatingReader {
        inner: Cursor<Vec<u8>>,
        reads: Cell<usize>,
        after: usize,
    }

    impl TruncatingReader {
        fn after(after: usize, n: usize) -> TruncatingReader {
            TruncatingReader {
                inner: Cursor::new(data(n)),
                reads: Cell::new(0),
                after,
            }
        }
    }

    impl Read for TruncatingReader {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let reads = self.reads.get();
            self.reads.set(reads + 1);
            if reads >= self.after {
                return Ok(0);
            }
            self.inner.read(buf)
        }
    }

    impl Seek for TruncatingReader {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.inner.seek(pos)
        }
    }
}
