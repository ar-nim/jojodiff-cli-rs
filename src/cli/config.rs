//! Option parsing and buffer sizing out of the `jdiff` binary
//! (`main.cpp:276-476` and `main.cpp:617-645`): the argv[0] function
//! dispatch, the `getopt_long` option loop (GNU permutation; `?` sets
//! liHlp=1 and parsing continues) and the effective buffer geometry with
//! its byte-pinned misalignment warnings. The recorded-but-unread knobs
//! (`-s`, `-t <n>`) stay recorded for the §21.11/§21.3 parity
//! documentation; warnings here are `dbg_print` only — no process exits.

use std::ffi::{OsStr, OsString};

use crate::cli::opts::{Getopt, Opt, VAL_COMPAT_081};
use crate::defs::c_atoi;
use crate::jdebug::{DBG_TO_STDOUT, dbg_print};
#[cfg(feature = "debug")]
use crate::jdebug::{
    DBGAHD, DBGAHH, DBGBKT, DBGBUF, DBGCMP, DBGDST, DBGHSH, DBGHSK, DBGMCH, DBGPRG, DBGRED, dbg_set,
};

/// Function to execute (`enum {Diff, Patch, Dedup, Test} liFun`,
/// `main.cpp:293`). Dedup/Test are ported per rulings §21.4/§21.3.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Function {
    Diff,
    Patch,
    Dedup,
    Test,
}

/// Read the function from argv[0] (`main.cpp:303-315`): the basename
/// after the last '/' or '\\', case-insensitively. `jpatch*` → Patch;
/// `jptch*` → Patch is the port extension (spec §21.2). `jdedup`/`jtst`
/// routes are not ported and fall through to Diff.
pub fn function_from_argv0(cmd: &OsStr) -> Function {
    let cmd = cmd.to_string_lossy().into_owned();
    let base = cmd.rsplit(['/', '\\']).next().unwrap_or("").to_lowercase();
    if base.starts_with("jpatch") || base.starts_with("jptch") {
        Function::Patch
    } else {
        Function::Diff
    }
}

/// Parsed options (`main.cpp:276-476`): everything the phases need. The
/// recorded-but-unread knobs (`-s`, `-t <n>`) are kept for the
/// §21.11/§21.3 parity documentation as locals inside [`parse`].
#[derive(Debug)]
pub struct Options {
    pub fun: Function,
    pub out_typ: i32, // 0 = JOutBin, 1 = JOutAsc, 2 = JOutRgn (3 = dedup)
    pub verbose: i32,
    pub src_bkt: bool,
    pub cmp_all: bool,
    pub src_scn: i32,
    pub mch_max: i32,
    pub mch_min: i32,
    pub hsh_mbt: i32,
    pub buf_org: i64,
    pub buf_new: i64,
    pub blk_sze: i32,
    pub ahd_max: i32,
    pub li_hlp: i32, // 0=no, 1=-h, 2=-hh, 3=error
    pub compat_081: bool,
    pub seq_org: bool,
    pub seq_new: bool,
    /// Operands after GNU permutation, plus `optind`.
    pub operands: Vec<OsString>,
    pub opt_arg_cnt: usize,
}

/// Parse the command line (`main.cpp:318-476`): getopt_long with GNU
/// permutation; `?` (unknown option or argument error) sets liHlp=1 and
/// parsing CONTINUES.
pub fn parse(args: &[OsString]) -> Options {
    /* Read the function from argv[0] (`main.cpp:303-315`): the basename
     * after the last '/' or '\', case-insensitively. `jpatch*` → Patch;
     * `jptch*` → Patch is the port extension (spec §21.2). The `jdedup` and
     * `jtst` routes are not ported (§21.3/§21.4) and fall through to Diff. */
    let mut fun = args
        .first()
        .map_or(Function::Diff, |a| function_from_argv0(a));

    /* Default settings (`main.cpp:276-293`). */
    let mut out_typ: i32 = 0; /* 0 = JOutBin, 1 = JOutAsc, 2 = JOutRgn */
    let mut verbose: i32 = 0; /* 0=no, 1=normal, 2=high */
    let mut src_bkt: bool = true; /* Backtrace on sourcefile allowed? */
    let mut cmp_all: bool = true; /* Compare even if data not in buffer? */
    let mut src_scn: i32 = 1; /* Prescan source file: 0=no, 1=do, 2=done */
    let mut mch_max: i32 = 128; /* Maximum entries in matching table. */
    let mut mch_min: i32 = 2; /* Minimum entries in matching table. */
    let mut hsh_mbt: i32 = 32; /* Hashtable size in MB (* 1024 * 1024) */
    let mut buf_org: i64 = 0; /* Default source-file buffer in MB */
    let mut buf_new: i64 = 0; /* Default destin-file buffer in MB */
    let mut blk_sze: i32 = 32 * 1024; /* Default block size (in bytes) */
    let mut ahd_max: i32 = 0; /* Lookahead range (0=same as llBufSze) */
    let mut li_hlp: i32 = 0; /* -h/--help flag: 0=no, 1=-h, 2=-hh, 3=error */
    let mut lb_stdio: bool = false; /* use stdio */
    let mut li_tst: i32 = 0; /* test to execute : 0 = normal, 1 etc... see JTest */
    let mut seq_org: bool = false; /* Sequential source file? */
    let mut seq_new: bool = false; /* Sequential destination file? */
    let mut compat_081 = false; /* --compat-081: 0.8.1-format patch output (§21.16) */

    let mut opt = Getopt::new(args.to_vec());
    loop {
        let code = match opt.next_opt() {
            Opt::End => break,
            Opt::Unknown => {
                li_hlp = 1;
                continue;
            }
            Opt::Code(c) => c,
        };
        let optarg = opt.optarg.take();
        match code {
            'b' => {
                // try-harder: increase hashtable size and more searching
                cmp_all = true; // verify all hashtable matches
                src_bkt = true; // allow going back on source file
                src_scn = 1; // create full index on source file
                mch_min = mch_min.wrapping_mul(2); // increase minimum number of matches to search
                mch_max = mch_max.wrapping_mul(4); // increase maximum number of matches to search
                hsh_mbt = hsh_mbt.wrapping_mul(4); // Increase index table size

                // larger buffers (more soft-ahead searching)
                buf_org = (if buf_org <= 0 { 1 } else { buf_org }) * 4;
            }
            'f' => {
                // faster (or rather: lazier)
                if cmp_all {
                    cmp_all = false; // No compares out-of-buffer
                    src_bkt = true;
                    src_scn = 1;
                    mch_min = mch_min.wrapping_mul(2);
                    mch_max /= 2;

                    // increase buffer size to have more lookahead indexing
                    buf_org = (if buf_org <= 0 { 1 } else { buf_org }) * 16;
                } else {
                    // even faster (lazier)
                    src_scn = 0; // No indexing scan
                    mch_min /= 2; // Reduce lookahead
                    mch_max /= 2;
                }
                hsh_mbt /= 2; // Reduce index table by 2
            }
            'p' => {
                // sequential source file
                seq_org = true;
                cmp_all = false; // only compare data within the buffer
                src_bkt = false; // only backtrack on source file in buffer
                src_scn = 0; // no pre-scan indexing
            }
            'q' => {
                // sequential destination file
                seq_new = true;
                mch_min = 0; // only search within the buffer
            }

            'c' => {
                // verbose-stdout
                DBG_TO_STDOUT.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            'h' => {
                // help
                li_hlp += 1;
            }
            'j' => {
                // jdiff
                fun = Function::Diff;
            }
            'l' => {
                // "list-details"
                out_typ = 1;
            }
            'r' => {
                // "list-groups"
                out_typ = 2;
            }
            's' => {
                // use stdio: accepted and recorded; nothing observable in the
                // port (one buffered engine over both backends, §21.11)
                lb_stdio = true;
            }
            't' => {
                // test: patch and unpatch in one go (broken upstream, §21.3)
                fun = Function::Test;
                li_tst = optarg.as_ref().map_or(0, |a| c_atoi(a)); // test number
            }
            'u' => {
                // unpatch
                fun = Function::Patch;
            }
            'v' => {
                // "verbose"
                verbose += 1;
            }
            'y' => {
                // deduplicate (compiled out upstream; §21.4)
                fun = Function::Dedup;
                out_typ = 3;
                lb_stdio = true;
                li_tst = optarg.as_ref().map_or(0, |a| c_atoi(a)); // never used
            }

            VAL_COMPAT_081 => {
                // --compat-081 (port-only, §21.16): 0.8.1-format patch
                // output on the diff side; accepted and ignored when
                // patching (and for the `-l`/`-r` listings, which are
                // diagnostic formats no patcher applies).
                compat_081 = true;
            }

            'a' => {
                // search-ahead-size
                ahd_max = match optarg {
                    Some(arg) => c_atoi(&arg).wrapping_mul(1024),
                    None => 0,
                };
            }

            'i' => {
                // index-size
                hsh_mbt = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if hsh_mbt <= 0 {
                    hsh_mbt = 1;
                    dbg_print(format_args!(
                        "Warning: invalid --index-size/-i specified, set to 1.\n"
                    ));
                }
            }
            'k' => {
                // "block-size"
                blk_sze = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if blk_sze <= 0 {
                    blk_sze = 1;
                    dbg_print(format_args!(
                        "Warning: invalid --block-size/-k specified, set to 1.\n"
                    ));
                }
            }
            'm' => {
                // "buffer-size": MB total, split evenly (`main.cpp:422-434`).
                let val = i64::from(optarg.as_ref().map_or(0, |a| c_atoi(a)));
                if buf_new == 0 {
                    // first -m
                    buf_new = val / 2;
                    buf_org = buf_new; // first -m specifies source and destination buffer
                } else if buf_org == buf_new {
                    // second -m
                    buf_org *= 2;
                    buf_new = val;
                } else {
                    // third and subsequent -m: do nothing
                }
            }
            'n' => {
                // "search-min"
                mch_min = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if mch_min < 0 {
                    mch_min = 0;
                }
            }
            'x' => {
                // "search-max": floored 1024; the unclamped value reaches
                // JMatchTable's bucket prime via JDiff's ctor (§21.15)
                mch_max = optarg.as_ref().map_or(0, |a| c_atoi(a));
                if mch_max <= 0 {
                    mch_max = 1024;
                }
            }

            'd' => {
                // debug flag by name (`main.cpp:446-472`); unknown names are
                // silently ignored, like the release C++ build whose strcmp
                // arms are compiled out.
                #[cfg(feature = "debug")]
                if let Some(name) = optarg.as_ref().map(|a| a.to_string_lossy().into_owned()) {
                    let flag = match name.as_str() {
                        "hsh" => Some(DBGHSH),
                        "ahd" => Some(DBGAHD),
                        "cmp" => Some(DBGCMP),
                        "prg" => Some(DBGPRG),
                        "buf" => Some(DBGBUF),
                        "hsk" => Some(DBGHSK),
                        "ahh" => Some(DBGAHH),
                        "bkt" => Some(DBGBKT),
                        "red" => Some(DBGRED),
                        "mch" => Some(DBGMCH),
                        "dst" => Some(DBGDST),
                        _ => None,
                    };
                    if let Some(idx) = flag {
                        dbg_set(idx, true);
                    }
                }
            }
            _ => unreachable!("Getopt only returns codes from the option table"),
        }
    }
    let opt_arg_cnt = opt.optind() - 1;
    // The recorded-but-unread knobs (§21.11/§21.3): `-s` picks the file
    // backend in C++ (one engine here); `liTst` is parsed and never used.
    let _ = lb_stdio;
    let _ = li_tst;

    Options {
        fun,
        out_typ,
        verbose,
        src_bkt,
        cmp_all,
        src_scn,
        mch_max,
        mch_min,
        hsh_mbt,
        buf_org,
        buf_new,
        blk_sze,
        ahd_max,
        li_hlp,
        compat_081,
        seq_org,
        seq_new,
        operands: opt.operands().to_vec(),
        opt_arg_cnt,
    }
}

/// Effective buffer geometry (`main.cpp:617-645`).
#[derive(Debug, Clone, Copy)]
pub struct Buffers {
    pub ll_buf_org: i64,
    pub ll_buf_new: i64,
    pub blk_sze: i32,
    pub ahd_max: i32,
}

/// Effective buffer geometry (`main.cpp:617-645`): defaults per
/// sequentiality, MB→bytes, block alignment warnings (byte-pinned
/// texts), the ahd_max default. Pure computation + pinned `dbg_print`
/// warnings; no process exits.
pub fn size_buffers(opts: &Options) -> Buffers {
    let buf_org = opts.buf_org;
    let buf_new = opts.buf_new;
    let mut blk_sze = opts.blk_sze;
    let mut ahd_max = opts.ahd_max;
    let seq_org = opts.seq_org;
    let seq_new = opts.seq_new;

    // Set default values for llBlk and liBlk (`main.cpp:617-621`).
    let mut ll_buf_org: i64 = if buf_org > 0 {
        buf_org
    } else if seq_org {
        32
    } else {
        1
    };
    let mut ll_buf_new: i64 = (if buf_new > 0 {
        buf_new
    } else if seq_new {
        16
    } else {
        ll_buf_org
    }) * 1024
        * 1024;
    ll_buf_org *= 1024 * 1024;
    blk_sze = if blk_sze < 4096 { 4096 } else { blk_sze };

    // Buffer size cannot be zero and must be aligned on block size
    // Block size  cannot be larger than buffer size (`main.cpp:623-636`)
    if ll_buf_org % i64::from(blk_sze) != 0 {
        ll_buf_org -= ll_buf_org % i64::from(blk_sze);
        if ll_buf_org <= 0 {
            ll_buf_org = i64::from(blk_sze);
        }
        dbg_print(format_args!(
            "Warning: Source buffer size misaligned with block size: set to {}.\n",
            ll_buf_org
        ));
    }
    if ll_buf_new % i64::from(blk_sze) != 0 {
        ll_buf_new -= ll_buf_new % i64::from(blk_sze);
        if ll_buf_new <= 0 {
            ll_buf_new = i64::from(blk_sze);
        }
        dbg_print(format_args!(
            "Warning: Destination buffer size misaligned with block size: set to {}.\n",
            ll_buf_new
        ));
    }

    // Default search ahead window (`main.cpp:638-645`): the C++ narrows the
    // long difference to int (wrapping for absurd -m values, like the C++
    // int conversion).
    if ahd_max == 0 {
        let diff = ll_buf_new - i64::from(blk_sze);
        ahd_max = diff as i32;
        if ahd_max < 4096 {
            ahd_max = 4096;
        }
    }

    Buffers {
        ll_buf_org,
        ll_buf_new,
        blk_sze,
        ahd_max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn argv0(name: &str) -> OsString {
        OsString::from(name)
    }

    #[test]
    fn argv0_dispatch_rules() {
        assert_eq!(function_from_argv0(&argv0("jdiff")), Function::Diff);
        assert_eq!(
            function_from_argv0(&argv0("/usr/bin/jpatch")),
            Function::Patch
        );
        assert_eq!(function_from_argv0(&argv0("JPTCH-OLD")), Function::Patch); // port ext, case-insensitive
        assert_eq!(function_from_argv0(&argv0("jtst")), Function::Diff); // not ported (§21.3)
    }

    #[test]
    fn parse_defaults_and_clusters() {
        let o = parse(&[argv0("jdiff"), OsString::from("a"), OsString::from("b")]);
        assert_eq!((o.mch_max, o.mch_min, o.hsh_mbt), (128, 2, 32));
        let o = parse(&[argv0("jdiff"), OsString::from("-vvb")]);
        assert_eq!(o.verbose, 2);
        assert!(o.cmp_all && o.src_bkt && o.src_scn == 1);
        // -b: mch_min *2 (2 → 4), mch_max *4 (128 → 512), hsh_mbt *4
        // (32 → 128), buf_org (0 → 1) *4 = 4.
        assert_eq!(o.mch_min, 4);
        assert_eq!(o.mch_max, 512);
        assert_eq!(o.hsh_mbt, 128);
        assert_eq!(o.buf_org, 4);
    }

    #[test]
    fn parse_unknown_option_sets_help_and_continues() {
        let o = parse(&[argv0("jdiff"), OsString::from("-Z")]);
        assert_eq!(o.li_hlp, 1);
    }

    #[test]
    fn size_buffers_aligns_on_blocks() {
        // -k 5000 is above the 4096 floor but misaligns the default source
        // buffer: 1 MB (buf_org 0 → 1, non-sequential) scaled to bytes is
        // 1048576, and 1048576 % 5000 = 3576, so the warning path runs and
        // floors ll_buf_org to a multiple of the block size. (The brief's
        // -k 8192 variant could not trigger it: any MB-multiple buffer is
        // already aligned on a power-of-two block size.)
        let o = parse(&[argv0("jdiff"), OsString::from("-k"), OsString::from("5000")]);
        let b = size_buffers(&o);
        assert_eq!(b.blk_sze, 5000);
        assert_eq!(b.ll_buf_org % i64::from(b.blk_sze), 0);
        assert_eq!(b.ll_buf_org, 1_045_000);
    }

    #[test]
    fn size_buffers_defaults_and_floors() {
        // Defaults: 1 MB source (buf_org 0 → 1) and — before scaling — the
        // same for the destination (buf_new 0 falls back to ll_buf_org);
        // the 32 KB default block size stays (already above the 4096
        // floor); ahd_max = ll_buf_new - blk_sze.
        let o = parse(&[argv0("jdiff"), OsString::from("a"), OsString::from("b")]);
        let b = size_buffers(&o);
        assert_eq!(b.blk_sze, 32 * 1024);
        assert_eq!(b.ll_buf_org, 1024 * 1024);
        assert_eq!(b.ll_buf_new, 1024 * 1024);
        assert_eq!(b.ahd_max, 1024 * 1024 - 32 * 1024);
    }

    #[test]
    fn size_buffers_floors_small_block_size() {
        // The 4096 floor: -k 1024 parses as 1024 but size_buffers raises it.
        let o = parse(&[argv0("jdiff"), OsString::from("-k"), OsString::from("1024")]);
        let b = size_buffers(&o);
        assert_eq!(b.blk_sze, 4096);
        assert_eq!(b.ll_buf_org % i64::from(b.blk_sze), 0);
    }

    #[test]
    fn size_buffers_sequential_defaults() {
        // -p/-q defaults (`main.cpp:617-621`): 32 MB source, 16 MB
        // destination.
        let o = parse(&[argv0("jdiff"), OsString::from("-p"), OsString::from("-q")]);
        let b = size_buffers(&o);
        assert_eq!(b.ll_buf_org, 32 * 1024 * 1024);
        assert_eq!(b.ll_buf_new, 16 * 1024 * 1024);
    }
}
