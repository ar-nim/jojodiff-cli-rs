//! `jptch` CLI and patch decoder — byte-compatible port of JojoDiff 0.8.1's
//! `src/jpatch.cpp` (spec §5): option parsing, greeting/help, file handling,
//! the `ufGetInt` length decoder and the `jpatch()` state machine.
//!
//! All printed strings are verbatim from `jpatch.cpp`. Greeting/usage/open
//! errors go to the [`jojodiff_cli_rs::jdebug`] stream (`stddbg`: stderr by
//! default, stdout with `-d`); the decoder's seek/read/write error messages
//! go to **real stderr** like the C++ `fprintf(stderr, …)` calls — even with
//! `-d` (`jpatch.cpp:161,178,216,185,196`).
//!
//! # Deviations / porting notes (spec §5, §12, §15)
//!
//! 1. **Open-error checks are live** (unlike the jdiff CLI): `jpatch.cpp`
//!    jfopens its three files directly — there is no pthread pre-read — so
//!    the stock binary really exits 3/4/5 with the documented messages.
//!    Verified against the oracle binary; the tests pin the real behavior.
//! 2. **stdin buffering (spec §15.4).** The C++ `getc`s/`fread`s/seeks the
//!    stdin `FILE*`, which fails on pipes. This port reads `-` inputs fully
//!    into memory (original into a `Cursor`, patch into a `Vec` reader) —
//!    strictly more permissive, identical output for seekable stdin. A read
//!    error on stdin is indistinguishable from EOF here, exactly like C's
//!    unchecked `ferror`: a truncated original then fails the EQL copy with
//!    exit 8, a truncated patch ends the stream early with exit 0.
//! 3. **Positions are mirrors.** The C++ prints `jftell()` values. This port
//!    tracks `org_pos` (advanced by real `SeekFrom::Current` seeks, whose
//!    returned absolute position it adopts, and by the EQL copy reads) and
//!    `out_pos` (advanced by every byte handed to the output stream,
//!    including buffered ones, like a C `FILE*` position). stdin inputs get
//!    real positions this way instead of the C pipe `ftell` failure value.
//! 4. **Write-error checking happens where the C++ checks it — and only
//!    there.** The `putc` calls of the MOD/INS data paths are never checked
//!    (`jpatch.cpp:250-258,267-282`); only the EQL copy's `fwrite`s are
//!    (`jpatch.cpp:188-191,199-202`), producing "Error writing output file."
//!    and exit 9. The output stream is therefore wrapped in a `BufWriter`
//!    (except `-d` + stdout output, which share Rust's global stdout buffer
//!    with the verbose stream like the C++ single `FILE*`): buffered small
//!    writes to a full device stay silently successful, matching the oracle
//!    (`jptch org small-eql-patch /dev/full` → exit 0, no message), while
//!    the checked multi-block copies surface failures with exit 9.
//! 5. **`-t` stores a flag nothing reads.** `gbTst` is set at
//!    `jpatch.cpp:330` and never consumed in 0.8.1; the port keeps it as an
//!    accepted no-op that raises verbose to 2.
//! 6. **`ufGetInt` EOF arithmetic.** Every `getc` may return EOF (-1) and the
//!    C code computes with it (`253 + (-1) = 252`, `(b << 8) + (-1)`, …)
//!    without any error path; the port replicates the i64 (C `off_t`)
//!    arithmetic with wrapping operations. The 8-byte form (`255` marker,
//!    largefile always on) is live code; the C++ non-largefile
//!    `exit(EXI_LRG)` branch is dead in the oracle build and is not ported.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io::{self, BufReader, BufWriter, Cursor, Read, Seek, SeekFrom, Write};
use std::process::exit;
use std::sync::atomic::Ordering;

use jojodiff_cli_rs::defs::{
    BKT, DEL, EOF, EQL, ESC, EXI_ARG, EXI_FRT, EXI_OUT, EXI_RED, EXI_SCD, EXI_SEK, EXI_WRI, INS,
    JDIFF_COPYRIGHT, JDIFF_VERSION, MOD, p8,
};
use jojodiff_cli_rs::jdebug::{DBG_TO_STDOUT, dbg_print};

/// Copy block size for EQL operations (`jpatch.cpp:43`).
const BLKSZE: usize = 4096;

fn main() {
    exit(real_main());
}

/// The current operand `liOpr` (`jpatch.cpp:121`). `Esc` is the initial state
/// in which non-ESC bytes hit no case of the data switch and are silently
/// dropped (spec §15.7).
#[derive(Clone, Copy)]
enum Opr {
    Esc,
    Mod,
    Ins,
    Del,
    Eql,
    Bkt,
}

/// Original file: a real file or an in-memory copy of stdin (module note 2).
trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

fn real_main() -> i32 {
    /* Read options (`jpatch.cpp:316-335`): options must precede the
     * filenames; the first non-option token ends parsing and is re-queued as
     * the first filename. `li_opt_arg_cnt` mirrors the C++ `liOptArgCnt`
     * index into the argument vector (which includes argv[0]). */
    let args: Vec<OsString> = std::env::args_os().collect();
    let arg_cnt = args.len(); /* aiArgCnt */
    let mut li_opt_arg_cnt: usize = 0;
    let mut lb_opt_arg_dne = false;
    let mut verbose = 0i32; /* giVerbse */
    let mut help = false; /* lcHlp == 'h' */
    let mut tst = false; /* gbTst */

    while !lb_opt_arg_dne && arg_cnt > li_opt_arg_cnt + 1 {
        li_opt_arg_cnt += 1;
        let tok = args[li_opt_arg_cnt].as_os_str();
        if is(tok, "-v") {
            verbose = 1;
        } else if is(tok, "-vv") {
            verbose = 2;
        } else if is(tok, "-vvv") {
            verbose = 3;
        } else if is(tok, "-d") {
            DBG_TO_STDOUT.store(true, Ordering::Relaxed); /* stddbg = stdout */
        } else if is(tok, "-h") {
            help = true;
        } else if is(tok, "-t") {
            verbose = 2;
            tst = true;
        } else {
            lb_opt_arg_dne = true;
            li_opt_arg_cnt -= 1;
        }
    }
    /* gbTst is stored but never read by the 0.8.1 decoder (module note 5). */
    let _ = tst;

    /* Output greetings (`jpatch.cpp:338-357`) */
    let nargs = arg_cnt - li_opt_arg_cnt;
    if verbose > 0 || help || nargs < 3 {
        print_greeting();
    }

    /* Usage / exit on missing args or help (`jpatch.cpp:359-376`); the 0.8.5
     * `EXI_ARG` is negative, `exit(-EXI_ARG)` keeps the process code at 2. */
    if nargs < 3 || help {
        print_usage();
        exit(-EXI_ARG);
    }

    /* Read filenames (`jpatch.cpp:379-384`); the indexes are in range because
     * the exit above guarantees nargs >= 3. The output name defaults to "-". */
    let nam_org = args[li_opt_arg_cnt + 1].clone();
    let nam_pch = args[li_opt_arg_cnt + 2].clone();
    let nam_out: OsString = if arg_cnt > li_opt_arg_cnt + 3 {
        args[li_opt_arg_cnt + 3].clone()
    } else {
        OsString::from("-")
    };

    /* Open the original file (`jpatch.cpp:387-394`): "-" means stdin, read
     * fully into memory (module note 2). */
    let org: Box<dyn ReadSeek> = if nam_org == *"-" {
        Box::new(Cursor::new(read_stdin()))
    } else {
        match File::open(&nam_org) {
            Ok(file) => Box::new(file),
            Err(_) => {
                dbg_print(format_args!(
                    "Could not open data file {} for reading.\n",
                    nam_org.to_string_lossy()
                ));
                exit(-EXI_FRT);
            }
        }
    };

    /* Open the patch file (`jpatch.cpp:396-404`): "-" means stdin, read
     * fully; a file is buffered for the byte-wise decoder. */
    let mut pch: Box<dyn Read> = if nam_pch == *"-" {
        Box::new(Cursor::new(read_stdin()))
    } else {
        match File::open(&nam_pch) {
            Ok(file) => Box::new(BufReader::new(file)),
            Err(_) => {
                dbg_print(format_args!(
                    "Could not open patch file {} for reading.\n",
                    nam_pch.to_string_lossy()
                ));
                exit(-EXI_SCD);
            }
        }
    };

    /* Open the output (`jpatch.cpp:406-415`): "-" means stdout; the message
     * carries no filename (sic, `jpatch.cpp:411`). */
    let out_is_stdout = nam_out == *"-";
    let sink = if out_is_stdout {
        Sink::Stdout(io::stdout().lock())
    } else {
        match File::create(&nam_out) {
            Ok(file) => Sink::File(file),
            Err(_) => {
                dbg_print(format_args!("Could not open output file for writing.\n"));
                exit(-EXI_OUT);
            }
        }
    };

    /* Buffer the output like the C stdio `FILE*` (module note 4). With `-d`
     * and a stdout output the raw lock is used so patch bytes and verbose
     * lines share one ordered buffer, like the C++ single `FILE*`. */
    let mut out: Box<dyn Write> = if out_is_stdout && DBG_TO_STDOUT.load(Ordering::Relaxed) {
        Box::new(sink)
    } else {
        Box::new(BufWriter::new(sink))
    };

    /* Go … (`jpatch.cpp:418`) */
    jpatch(&mut *out, org, &mut *pch, verbose);

    /* Close files / exit (`jpatch.cpp:421-424`): exit 0 unconditionally; the
     * exit-time flush mirrors the C runtime's flush-on-exit (errors ignored,
     * like the unchecked `fclose`). */
    let _ = out.flush();
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();

    0
}

/// Byte-equality of an option token with a C string (`strcmp == 0`).
fn is(tok: &OsStr, opt: &str) -> bool {
    tok == OsStr::new(opt)
}

/// Reads a `-` input fully into memory (module note 2): read errors are
/// indistinguishable from EOF, like the C `getc`/`fread` calls that never
/// check `ferror` — a truncated original then fails the EQL copy (exit 8),
/// a truncated patch simply ends the stream (exit 0).
fn read_stdin() -> Vec<u8> {
    let mut data = Vec::new();
    let _ = io::stdin().lock().read_to_end(&mut data);
    data
}

/// Greeting block (`jpatch.cpp:339-356`), written line by line like the C++
/// `fprintf` calls; the file-adressing line has no sizes (sic spelling kept).
fn print_greeting() {
    dbg_print(format_args!(
        "JPATCH - Jojo's binary patch version {JDIFF_VERSION}\n"
    ));
    dbg_print(format_args!("{JDIFF_COPYRIGHT}\n"));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "JojoDiff is free software: you can redistribute it and/or modify\n"
    ));
    dbg_print(format_args!(
        "it under the terms of the GNU General Public License as published by\n"
    ));
    dbg_print(format_args!(
        "the Free Software Foundation, either version 3 of the License, or\n"
    ));
    dbg_print(format_args!("(at your option) any later version.\n"));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "This program is distributed in the hope that it will be useful,\n"
    ));
    dbg_print(format_args!(
        "but WITHOUT ANY WARRANTY; without even the implied warranty of\n"
    ));
    dbg_print(format_args!(
        "MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the\n"
    ));
    dbg_print(format_args!(
        "GNU General Public License for more details.\n"
    ));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "You should have received a copy of the GNU General Public License\n"
    ));
    dbg_print(format_args!(
        "along with this program.  If not, see <http://www.gnu.org/licenses/>.\n"
    ));
    dbg_print(format_args!("\n"));
    /* `jpatch.cpp:355`: "%d bit" is sizeof(off_t) * 8 with off_t = i64. */
    dbg_print(format_args!(
        "File adressing is {} bit.\n",
        std::mem::size_of::<i64>() as i32 * 8
    ));
    dbg_print(format_args!("\n"));
}

/// Usage/help block (`jpatch.cpp:360-373`); the `-l` line is commented out in
/// the C++ and therefore absent.
fn print_usage() {
    dbg_print(format_args!(
        "Usage: jpatch [options] <original file> <patch file> [<output file>]\n"
    ));
    dbg_print(format_args!(
        "  -v               Verbose: version and licence.\n"
    ));
    dbg_print(format_args!("  -vv              Verbose: debug info.\n"));
    dbg_print(format_args!(
        "  -vvv             Verbose: more debug info.\n"
    ));
    dbg_print(format_args!("  -h               Help (this text).\n"));
    dbg_print(format_args!("  -t               Test: no output file.\n"));
    dbg_print(format_args!("Principles:\n"));
    dbg_print(format_args!(
        "  JPATCH reapplies a diff file, generated by jdiff, to the <original file>,\n"
    ));
    dbg_print(format_args!(
        "  restoring the <new file>. For example, if jdiff has been called like this:\n"
    ));
    dbg_print(format_args!("    jdiff data01.tar data02.tar data02.dif\n"));
    dbg_print(format_args!(
        "  then data02.tar can be restored as follows:\n"
    ));
    dbg_print(format_args!(
        "    jpatch data01.tar data02.dif data02.tar\n"
    ));
    dbg_print(format_args!("\n"));
}

/// Output sink: a real file or locked stdout (`jpatch.cpp:406-409`).
enum Sink {
    File(File),
    Stdout(std::io::StdoutLock<'static>),
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Sink::File(file) => file.write(buf),
            Sink::Stdout(lock) => lock.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Sink::File(file) => file.flush(),
            Sink::Stdout(lock) => lock.flush(),
        }
    }
}

/// C `getc`: one byte, or `EOF` (-1) on end-of-stream or error — the C code
/// never distinguishes the two (no `ferror` checks).
fn getc(stream: &mut dyn Read) -> i32 {
    let mut buf = [0u8; 1];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => return EOF,
            Ok(_) => return i32::from(buf[0]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return EOF,
        }
    }
}

/// Port of `ufGetInt` (`jpatch.cpp:70-105`): variable-length big-endian
/// number, computed on `off_t` (i64) values where every `getc` may also be
/// EOF (-1). Overflow wraps like the two's-complement C arithmetic.
fn uf_get_int(stream: &mut dyn Read) -> i64 {
    let mut val = i64::from(getc(stream)); /* -1 … 255 */
    if val < 252 {
        val + 1 /* EOF (-1) yields 0, like the C */
    } else if val == 252 {
        i64::from(253 + getc(stream)) /* EOF yields 252 */
    } else if val == 253 {
        val = i64::from(getc(stream));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val
    } else if val == 254 {
        val = i64::from(getc(stream));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val
    } else {
        /* JDIFF_LARGEFILE 8-byte form; the C++ non-largefile
         * "64-bit length numbers not supported!" exit is dead in the oracle
         * build (JDIFF_LARGEFILE always defined) and is not ported. */
        val = i64::from(getc(stream));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val = (val << 8).wrapping_add(i64::from(getc(stream)));
        val
    }
}

/// Checked-output failure (`jpatch.cpp:162-163` et al): flush the output like
/// the C runtime's flush-on-exit, print to real stderr, exit with the
/// positive process code `code` (the `-EXI_*` of a 0.8.5 constant).
fn fail(out: &mut dyn Write, code: i32, message: &str) -> ! {
    let _ = out.flush();
    eprint!("{message}");
    exit(code);
}

/// `jfseek(asFilOrg, delta, SEEK_CUR)` (`jpatch.cpp:160,177,215`); the
/// `org_pos` mirror adopts the returned absolute position (the C `ftello`
/// value after the seek).
fn org_seek(org: &mut dyn ReadSeek, delta: i64, org_pos: &mut i64) -> bool {
    match org.seek(SeekFrom::Current(delta)) {
        /* Rust's Seek is u64-based; the bit pattern reinterpreted as i64 is
         * the C `off_t`/`ftello` value. */
        Ok(pos) => {
            *org_pos = pos as i64;
            true
        }
        Err(_) => false,
    }
}

/// EQL copy read (`jpatch.cpp:184,195`): a short read (EOF or error) is
/// fatal — the C `fread` return value is checked, nothing is written.
fn org_read(org: &mut dyn ReadSeek, buf: &mut [u8], org_pos: &mut i64) -> bool {
    let mut filled = 0;
    while filled < buf.len() {
        match org.read(&mut buf[filled..]) {
            Ok(0) => return false,
            Ok(n) => {
                filled += n;
                *org_pos += n as i64;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        }
    }
    true
}

/// EQL copy write (`jpatch.cpp:188,199`): the `fwrite` return value is
/// checked — any failure is fatal (exit 9). On success the `out_pos` mirror
/// advances by the full count (the C `ftello` includes buffered bytes).
fn out_write(out: &mut dyn Write, buf: &[u8], out_pos: &mut i64) {
    if out.write_all(buf).is_err() {
        fail(out, -EXI_WRI, "Error writing output file.\n");
    }
    *out_pos += buf.len() as i64;
}

/// `putc(...)`: the result is never checked in the C++ (module note 4); the
/// `out_pos` mirror advances regardless, like the C `ftello` of a buffered
/// stream.
fn putc_ign(out: &mut dyn Write, b: u8, out_pos: &mut i64) {
    let _ = out.write_all(&[b]);
    *out_pos += 1;
}

/// The `((liInp >= 32 && liInp <= 127) ? (char) liInp : ' ')` argument of the
/// `%c` conversions (`jpatch.cpp:263,279`).
fn chr(inp: i32) -> char {
    if (32..=127).contains(&inp) {
        char::from_u32(inp as u32).unwrap_or(' ')
    } else {
        ' '
    }
}

/// The C `jftell(asFilOrg)+lzMod` position argument of the verbose prints
/// (`jpatch.cpp:138-291`); wrapping because the C `off_t` arithmetic wraps
/// where a debug build would panic (only reachable behind an extreme seek).
fn org_print_pos(org_pos: i64, lz_mod: i64) -> i64 {
    org_pos.wrapping_add(lz_mod)
}

/// Port of `jpatch()` (`jpatch.cpp:118-294`): applies the patch stream to the
/// original file, writing the restored file. Positions are printed from the
/// `org_pos`/`out_pos` mirrors (module note 3).
fn jpatch(out: &mut dyn Write, mut org: Box<dyn ReadSeek>, pch: &mut dyn Read, verbose: i32) {
    let mut li_opr = Opr::Esc; /* Current operand */
    let mut lz_mod: i64 = 0; /* Bytes written by MOD, not read from org */
    let mut lb_chg = false; /* An opcode was just consumed */
    let mut lb_esc = false; /* Non-operand escape char found */

    let mut org_pos: i64 = 0; /* Mirror of jftell(asFilOrg) */
    let mut out_pos: i64 = 0; /* Mirror of jftell(asFilOut) */

    let mut lc_dta = [0u8; BLKSZE];

    loop {
        let mut li_inp = getc(pch); /* Current input from patch file */
        if li_inp == EOF {
            break;
        }

        /* Parse an operator: ESC liOpr [lzOff] (`jpatch.cpp:132-239`) */
        if li_inp == ESC {
            /* The operand byte replaces liInp; EOF (-1) falls into the
             * default arm and its data `putc` emits byte 0xFF, exactly like
             * the C int arithmetic (`jpatch.cpp:133`). */
            li_inp = getc(pch);
            match li_inp {
                MOD => {
                    li_opr = Opr::Mod;
                    if verbose == 1 {
                        dbg_print(format_args!(
                            "{} {} MOD ...    \n",
                            p8(org_print_pos(org_pos, lz_mod - 1)),
                            p8(out_pos)
                        ));
                    }
                    lb_chg = true;
                }

                INS => {
                    li_opr = Opr::Ins;
                    if verbose == 1 {
                        dbg_print(format_args!(
                            "{} {} INS ...    \n",
                            p8(org_print_pos(org_pos, lz_mod - 1)),
                            p8(out_pos)
                        ));
                    }
                    lb_chg = true;
                }

                DEL => {
                    li_opr = Opr::Del;
                    let lz_off = uf_get_int(pch);
                    if verbose >= 1 {
                        dbg_print(format_args!(
                            "{} {} DEL {}\n",
                            p8(org_print_pos(org_pos, lz_mod)),
                            p8(out_pos),
                            lz_off
                        ));
                    }

                    if !org_seek(&mut org, lz_off.wrapping_add(lz_mod), &mut org_pos) {
                        fail(
                            out,
                            -EXI_SEK,
                            &format!(
                                "Could not position on original file (seek {} + {}).\n",
                                lz_off, lz_mod
                            ),
                        );
                    }
                    lz_mod = 0;
                    lb_chg = true;
                }

                EQL => {
                    li_opr = Opr::Eql;
                    let lz_off = uf_get_int(pch);
                    if verbose >= 1 {
                        dbg_print(format_args!(
                            "{} {} EQL {}\n",
                            p8(org_print_pos(org_pos, lz_mod)),
                            p8(out_pos),
                            lz_off
                        ));
                    }

                    if lz_mod > 0 {
                        if !org_seek(&mut org, lz_mod, &mut org_pos) {
                            fail(
                                out,
                                -EXI_SEK,
                                &format!(
                                    "Could not position on original file (skip {}).\n",
                                    lz_mod
                                ),
                            );
                        }
                        lz_mod = 0;
                    }
                    let mut lz_cnt = lz_off;
                    while lz_cnt > BLKSZE as i64 {
                        if !org_read(&mut org, &mut lc_dta, &mut org_pos) {
                            fail(out, -EXI_RED, "Error reading original file.\n");
                        }
                        out_write(out, &lc_dta, &mut out_pos);
                        lz_cnt -= BLKSZE as i64;
                    }
                    if lz_cnt > 0 {
                        if !org_read(&mut org, &mut lc_dta[..lz_cnt as usize], &mut org_pos) {
                            fail(out, -EXI_RED, "Error reading original file.\n");
                        }
                        out_write(out, &lc_dta[..lz_cnt as usize], &mut out_pos);
                    }
                    lb_chg = true;
                }

                BKT => {
                    li_opr = Opr::Bkt;
                    let lz_off = uf_get_int(pch);
                    if verbose >= 1 {
                        dbg_print(format_args!(
                            "{} {} BKT {}\n",
                            p8(org_print_pos(org_pos, lz_mod)),
                            p8(out_pos),
                            lz_off
                        ));
                    }

                    if !org_seek(&mut org, lz_mod.wrapping_sub(lz_off), &mut org_pos) {
                        fail(
                            out,
                            -EXI_SEK,
                            &format!(
                                "Could not position on original file (seek back {} - {}).\n",
                                lz_mod, lz_off
                            ),
                        );
                    }
                    lz_mod = 0;
                    lb_chg = true;
                }

                ESC => {
                    if verbose > 2 {
                        dbg_print(format_args!(
                            "{} {} ESC ESC\n",
                            p8(org_print_pos(org_pos, lz_mod)),
                            p8(out_pos)
                        ));
                    }
                }

                _ => {
                    if verbose > 2 {
                        dbg_print(format_args!(
                            "{} {} ESC XXX\n",
                            p8(org_print_pos(org_pos, lz_mod)),
                            p8(out_pos)
                        ));
                    }
                    lb_esc = true;
                }
            }
        }

        /* Handle a data byte unless an opcode was just consumed
         * (`jpatch.cpp:241-285`). */
        if lb_chg {
            lb_chg = false;
        } else {
            match li_opr {
                Opr::Del | Opr::Eql | Opr::Bkt | Opr::Esc => {}

                Opr::Mod => {
                    if lb_esc {
                        putc_ign(out, ESC as u8, &mut out_pos);
                        lz_mod += 1;
                        if verbose > 2 {
                            dbg_print(format_args!(
                                "{} {} MOD {:>3o} ESC\n",
                                p8(org_print_pos(org_pos, lz_mod - 1)),
                                p8(out_pos - 1),
                                ESC as u32
                            ));
                        }
                    }

                    putc_ign(out, li_inp as u8, &mut out_pos);
                    lz_mod += 1;
                    if verbose > 2 {
                        dbg_print(format_args!(
                            "{} {} MOD {:>3o} {}\n",
                            p8(org_print_pos(org_pos, lz_mod - 1)),
                            p8(out_pos - 1),
                            li_inp as u32,
                            chr(li_inp)
                        ));
                    }
                }

                Opr::Ins => {
                    if lb_esc {
                        if verbose > 2 {
                            dbg_print(format_args!(
                                "{} {} INS {:>3o} ESC\n",
                                p8(org_print_pos(org_pos, lz_mod - 1)),
                                p8(out_pos),
                                ESC as u32
                            ));
                        }
                        putc_ign(out, ESC as u8, &mut out_pos);
                    }

                    if verbose > 2 {
                        dbg_print(format_args!(
                            "{} {} INS {:>3o} {}\n",
                            p8(org_print_pos(org_pos, lz_mod - 1)),
                            p8(out_pos),
                            li_inp as u32,
                            chr(li_inp)
                        ));
                    }

                    putc_ign(out, li_inp as u8, &mut out_pos);
                }
            }
        } /* if lb_chg */

        lb_esc = false;
    } /* while */

    if verbose > 1 {
        dbg_print(format_args!(
            "{} {} EOF",
            p8(org_print_pos(org_pos, lz_mod)),
            p8(out_pos)
        ));
    }
}
