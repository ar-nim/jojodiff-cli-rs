//! The byte-pinned text blocks out of the `jdiff` binary (`main.cpp:480-593`):
//! greeting, usage/help and the `-hh` notes. Every `dbg_print` line is
//! verbatim from `main.cpp` — including the stale texts (spec §21.10): the
//! usage claims `-i` "(default 64)" (actual 32) and `-k` "(default 8192)"
//! (actual 32768), calls `-m` sizes "(in KB)" (actual MB) and offers
//! "0=no buffering" (no such mode). Do not fix stale texts or typos.

use crate::defs::{JDIFF_COPYRIGHT, JDIFF_VERSION, MAX_OFF_T, SMPSZE};
use crate::jdebug::dbg_print;

/// Greeting block (`main.cpp:480-509`), written line by line like the C++
/// `fprintf` calls — including the 0.8.5 GPL wording and the "adressing"
/// (sic) line computed from `MAX_OFF_T` (no 0.8.1 `+1`).
pub(crate) fn print_greeting() {
    dbg_print(format_args!(
        "\nJDIFF - binary diff version {JDIFF_VERSION}\n"
    ));
    dbg_print(format_args!("{JDIFF_COPYRIGHT}\n"));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "JojoDiff is free software: you can redistribute it and/or modify it\n"
    ));
    dbg_print(format_args!(
        "under the terms of the  GNU General Public License  as published by\n"
    ));
    dbg_print(format_args!(
        "the Free Software Foundation,  either version 3 of the License,  or\n"
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
        "MERCHANTABILITY  or  FITNESS FOR A PARTICULAR PURPOSE. See the\n"
    ));
    dbg_print(format_args!(
        "GNU General Public License for more details.\n"
    ));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "You should have received a copy of the GNU General Public License\n"
    ));
    dbg_print(format_args!(
        "along with this program. If not, see www.gnu.org/licenses/gpl-3.0\n\n"
    ));

    /* `main.cpp:497-508`: MAX_OFF_T >> 30 GB, shifted to TB when above 1024;
     * "%d bit" is sizeof(off_t) * 8 with off_t = i64. */
    let mut maxoff_t_gb = MAX_OFF_T >> 30;
    let mut maxoff_t_mul = "GB";
    if maxoff_t_gb > 1024 {
        maxoff_t_gb >>= 10;
        maxoff_t_mul = "TB";
    }
    dbg_print(format_args!(
        "File adressing is {} bit for files up to {maxoff_t_gb}{maxoff_t_mul}, samples are {SMPSZE} bytes.\n",
        std::mem::size_of::<i64>() as i32 * 8
    ));
}

/// Usage/help block (`main.cpp:511-557`); the `-n`/`-x` lines print the
/// current (post-parse) match limits, and the stale texts are replicated
/// verbatim (spec §21.10).
pub(crate) fn print_usage(mch_min: i32, mch_max: i32) {
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "JDiff differentiates two files so that the second file can be recreated from\n"
    ));
    dbg_print(format_args!(
        "the first by \"undiffing\". JDiff aims for the smallest possible diff file.\n\n"
    ));
    dbg_print(format_args!(
        "Usage: jdiff -j [options] <source file> <destination file> [<diff file>]\n"
    ));
    dbg_print(format_args!(
        "   or: jdiff -u [options] <source file> <diff file> [<destination file>]\n\n"
    ));
    dbg_print(format_args!(
        "  -j                       JDiff:  create a difference file.\n"
    ));
    dbg_print(format_args!(
        "  -u                       Undiff: undiff a difference file.\n\n"
    ));

    dbg_print(format_args!(
        "  -v --verbose             Verbose: greeting, results and tips.\n"
    ));
    dbg_print(format_args!(
        "  -vv                      Extra Verbose: progress info and statistics.\n"
    ));
    dbg_print(format_args!(
        "  -vvv                     Ultra Verbose: all info, including help and details.\n"
    ));
    dbg_print(format_args!(
        "  -h --help -hh            Help, additional help (-hh) and exit.\n"
    ));
    dbg_print(format_args!(
        "  -l --listing             Detailed human readable output.\n"
    ));
    dbg_print(format_args!(
        "  -r --regions             Grouped  human readable output.\n"
    ));
    dbg_print(format_args!(
        "  -c --console             Write verbose and debug info to stdout.\n\n"
    ));

    dbg_print(format_args!(
        "  -b --better -bb...       Better: use more memory, search more.\n"
    ));
    dbg_print(format_args!(
        "  -bb                      Best:   even more memory, search more.\n"
    ));
    dbg_print(format_args!(
        "  -f --lazy                Lazy:   no unbuffered searching (often slower).\n"
    ));
    dbg_print(format_args!(
        "  -ff                      Lazier: no full index table.\n"
    ));
    dbg_print(format_args!(
        "  -p --sequential-source   Sequential source (to avoid !) (with - for stdin).\n"
    ));
    dbg_print(format_args!(
        "  -q --sequential-dest     Sequential destination (with - for stdin).\n"
    ));
    dbg_print(format_args!(
        "  -s --stdio               Use stdio files (for testing).\n"
    ));
    dbg_print(format_args!("\n"));
    dbg_print(format_args!(
        "  -a --search-size <size>  Size (in KB) to search (default=buffer-size).\n"
    ));
    dbg_print(format_args!(
        "  -i --index-size  <size>  Size (in MB) for index table    (default 64).\n"
    ));
    dbg_print(format_args!(
        "  -k --block-size  <size>  Block size in bytes for reading (default 8192).\n"
    ));
    dbg_print(format_args!(
        "  -m --buffer-size <size>  Size (in KB) for search buffers (0=no buffering)\n"
    ));
    dbg_print(format_args!(
        "  -n --search-min <count>  Minimum number of matches to search (default {mch_min}).\n"
    ));
    dbg_print(format_args!(
        "  -x --search-max <count>  Maximum number of matches to search (default {mch_max}).\n\n"
    ));

    dbg_print(format_args!(
        "Make  diff-file: jdiff -j old-file new-file diff-file.jdf\n"
    ));
    dbg_print(format_args!(
        "Apply diff-file: jdiff -u old-file diff-file.jdf recreated-new-file\n\n"
    ));

    dbg_print(format_args!("Hint:\n"));
    dbg_print(format_args!(
        "  Do not use jdiff on compressed files. Rather use jdiff first and compress\n"
    ));
    dbg_print(format_args!(
        "  afterwards, e.g.: jdiff -j old new | gzip >dif.jdf.gz (or 7z with -si)\n"
    ));
}

/// The `-hh` notes block (`main.cpp:559-593`, printed when liHlp > 1 or
/// verbose > 2), verbatim including the two-space "blank" lines.
pub(crate) fn print_notes() {
    dbg_print(format_args!("\nNotes:\n"));
    dbg_print(format_args!(
        " - Options -b, -bb, -f, -ff, ... should be used before other options.\n"
    ));
    dbg_print(format_args!(
        " - Accuracy may be improved by increasing the index table size (-i) or\n"
    ));
    dbg_print(format_args!("   the buffer size (-m), see below.\n"));
    dbg_print(format_args!(
        " - The index table size is always lowered to the nearest lower prime number.\n"
    ));
    dbg_print(format_args!(
        " - Output is sent to standard output if no output file is specified.\n"
    ));
    dbg_print(format_args!("\nAdditional explications:\n"));
    dbg_print(format_args!(
        "  JDiff starts by comparing source and destination files.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  When a difference is found, JDiff will first index the source file.\n"
    ));
    dbg_print(format_args!(
        "  Normally, the full source file is indexed, but this can be disabled by the\n"
    ));
    dbg_print(format_args!(
        "  -ff or -p options, in which case only the buffered part of the source file\n"
    ));
    dbg_print(format_args!(
        "  will be indexed. This may be faster, but at a loss of accuracy.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  Using the index, JDiff will search for equal regions between both files.\n"
    ));
    dbg_print(format_args!(
        "  The index table however has two problems:\n"
    ));
    dbg_print(format_args!(
        "  - too small, because a full index would require too much memory.\n"
    ));
    dbg_print(format_args!(
        "  - inaccurate, because the hash-keys are only 32 or 64 bit check-sums.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!("  The inaccuracy is reduced by either:\n"));
    dbg_print(format_args!(
        "  - comparing the found matches from the index, which is slower but certain\n"
    ));
    dbg_print(format_args!(
        "  - confirmation from subsequent matches, which is faster but uncertain\n"
    ));
    dbg_print(format_args!(
        "  Inaccuracy of course can also be reduced with a bigger index table (-i option)\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  Also, the first found solution is not always the best solution.\n"
    ));
    dbg_print(format_args!(
        "  Therefore, JDiff searches a minimum (-n) number of solutions, and\n"
    ));
    dbg_print(format_args!(
        "  will continue up to a maximum (-x) number of solutions if data is buffered.\n"
    ));
    dbg_print(format_args!(
        "  That's why, bigger buffers (-m) can improve accuracy.\n"
    ));
    dbg_print(format_args!("  \n"));
    dbg_print(format_args!(
        "  The -b/-bb options increase the index table, buffers and solutions to search.\n"
    ));
    dbg_print(format_args!(
        "  The -f/-ff options will only compare buffered data to gain some speed, but\n"
    ));
    dbg_print(format_args!(
        "  will often be slower due to the lower accuracy.\n"
    ));
}
