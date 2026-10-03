//! A std-only `getopt_long` equivalent for the 0.8.5 CLI surface
//! (spec §18.D): the C++ `main.cpp:236-261` option table is normative and is
//! copied verbatim; the scanner reproduces the glibc semantics the CLI
//! depends on —
//!
//! * the short-option string `"a:bcd:fhi:jk:lm:n:pqrst::uvx:y"`
//!   (`main.cpp:236`, including the `t::` optional argument; the `u::` in the
//!   C++ comment there is stale — `-t` is the optional-argument option),
//! * clustered short options (`-vv`, `-bb`, `-t3`, `-dhsh`),
//! * long options with abbreviation (unique prefix resolves, ambiguous
//!   prefix errors), `--name=value` for required/optional arguments and
//!   `--name value` for required ones,
//! * GNU permutation: options may appear before, between or after the
//!   operands; at end of scan the operands are the non-option arguments in
//!   their original order, exactly like glibc's in-place exchange,
//! * `--` ends option processing (itself consumed; everything after it is an
//!   operand, including further `--` tokens),
//! * a lone `-` is an operand (the stdin/stdout filename),
//! * glibc's four error messages on real stderr for the `?` return
//!   (invalid short option, missing short argument, unrecognized long
//!   option, long option that doesn't allow / requires an argument,
//!   ambiguous long prefix) — the messages go to plain stderr like libc,
//!   not to `JDebug::stddbg`,
//! * unknown options and argument errors answer `'?'` and scanning CONTINUES
//!   (no leading `+`/`:` in the optstring), which the CLI maps to
//!   `liHlp = 1` and continue (`main.cpp:473-475`).
//!
//! C `optind`/`liOptArgCnt` bookkeeping: the CLI only consumes the final
//! permutation state — `optind` (index of the first operand in the permuted
//! argv) = `1 + (number of argv slots held by options and their detached
//! arguments)`, and `liOptArgCnt = optind - 1`, so `nargs = argc -
//! liOptArgCnt` counts the operands plus argv[0]. [`Getopt::optind`] reports
//! exactly that; the operands themselves come from [`Getopt::operands`].

use std::ffi::OsString;
use std::io::Write;

/// Short-option string (`gcOptSht`, `main.cpp:236`). `t::` = optional
/// argument (the trailing `::` glibc syntax).
pub const OPT_SHT: &str = "a:bcd:fhi:jk:lm:n:pqrst::uvx:y";

/// The table code of the long-only `--compat-081` (port-only extension,
/// spec §21.16): it has no short-option letter — upstream's single-letter
/// option space is fully allocated — so its `val` is a non-ASCII sentinel
/// the short-option string can never collide with (the glibc analog of a
/// `val` beyond the `getopt` short-letter range).
pub const VAL_COMPAT_081: char = '\u{2603}';

/// Whether a long option takes an argument (`main.cpp:238-261`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HasArg {
    No,
    Required,
    Optional,
}

/// One `struct option` row (`main.cpp:238-261`; the C `flag` member is
/// always `NULL` and `val` carries the short code).
#[derive(Clone, Copy, Debug)]
pub struct LongOpt {
    pub name: &'static str,
    pub has_arg: HasArg,
    pub val: char,
}

/// The long-option table (`gsOptLng`, `main.cpp:238-261`), in the C++ order
/// (order matters for the ambiguity message's possibility list), plus the
/// appended port-only `--compat-081` row (§21.16). Appending keeps every
/// upstream row's order (and thus the possibility-list order) unchanged;
/// the new name still joins the `--c` prefix's possibility list, so `--c`
/// becomes ambiguous (console | compat-081) — the same effect glibc shows
/// when a c-prefixed long option is added.
pub const OPT_LNG: &[LongOpt] = &[
    LongOpt {
        name: "better",
        has_arg: HasArg::No,
        val: 'b',
    },
    LongOpt {
        name: "lazy",
        has_arg: HasArg::No,
        val: 'f',
    },
    LongOpt {
        name: "console",
        has_arg: HasArg::No,
        val: 'c',
    },
    LongOpt {
        name: "debug",
        has_arg: HasArg::Required,
        val: 'd',
    },
    LongOpt {
        name: "help",
        has_arg: HasArg::No,
        val: 'h',
    },
    LongOpt {
        name: "listing",
        has_arg: HasArg::No,
        val: 'l',
    },
    LongOpt {
        name: "regions",
        has_arg: HasArg::No,
        val: 'r',
    },
    LongOpt {
        name: "sequential-source",
        has_arg: HasArg::No,
        val: 'p',
    },
    LongOpt {
        name: "sequential-dest",
        has_arg: HasArg::No,
        val: 'q',
    },
    LongOpt {
        name: "stdio",
        has_arg: HasArg::No,
        val: 's',
    },
    LongOpt {
        name: "test",
        has_arg: HasArg::Optional,
        val: 't',
    },
    LongOpt {
        name: "jdiff",
        has_arg: HasArg::No,
        val: 'j',
    },
    LongOpt {
        name: "undiff",
        has_arg: HasArg::No,
        val: 'u',
    },
    LongOpt {
        name: "index-size",
        has_arg: HasArg::Required,
        val: 'i',
    },
    LongOpt {
        name: "block-size",
        has_arg: HasArg::Required,
        val: 'k',
    },
    LongOpt {
        name: "buffer-size",
        has_arg: HasArg::Required,
        val: 'm',
    },
    LongOpt {
        name: "search-size",
        has_arg: HasArg::Required,
        val: 'a',
    },
    LongOpt {
        name: "search-min",
        has_arg: HasArg::Required,
        val: 'n',
    },
    LongOpt {
        name: "search-max",
        has_arg: HasArg::Required,
        val: 'x',
    },
    LongOpt {
        name: "reflink",
        has_arg: HasArg::No,
        val: 'y',
    },
    LongOpt {
        name: "verbose",
        has_arg: HasArg::No,
        val: 'v',
    },
    // Port-only extension (spec §21.16, appended after the upstream rows to
    // keep their order unchanged): long-only 0.8.1-format patch output;
    // meaningful on the diff side, accepted and ignored when patching.
    LongOpt {
        name: "compat-081",
        has_arg: HasArg::No,
        val: VAL_COMPAT_081,
    },
];

/// The getopt return: a short/long option's code (`val`), the `'?'` error
/// code, or end of scan (glibc `EOF`, i.e. -1).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Opt {
    Code(char),
    Unknown,
    End,
}

/// glibc-style scanner over the full argv (argv[0] included, like `main`).
pub struct Getopt {
    args: Vec<OsString>,
    /// `argv[0]` as passed, used verbatim in the error messages.
    prog: OsString,
    /// C `optind`: next argv index to examine.
    optind: usize,
    /// Byte offset of the next short option inside the current cluster
    /// (`nextchar`).
    nextchar: usize,
    /// The token the active cluster belongs to (empty when no cluster is in
    /// progress; the token itself was consumed from argv when it started).
    cluster: String,
    /// C `optarg`, valid for options taking an argument (required ones and
    /// attached optional ones).
    pub optarg: Option<OsString>,
    /// Non-option arguments in encounter order (the GNU permutation result).
    operands: Vec<OsString>,
    /// `--` seen: every further token is an operand.
    end_of_options: bool,
}

impl Getopt {
    /// Builds the scanner over `env::args_os()`.
    pub fn new(args: Vec<OsString>) -> Self {
        let prog = args.first().cloned().unwrap_or_default();
        Getopt {
            args,
            prog,
            optind: 1,
            nextchar: 0,
            cluster: String::new(),
            optarg: None,
            operands: Vec::new(),
            end_of_options: false,
        }
    }

    /// The operands seen so far (call again after the scan).
    pub fn operands(&self) -> &[OsString] {
        &self.operands
    }

    /// C `optind` after the scan: index of the first operand in the permuted
    /// argv = `argc - operand count` (options with detached arguments hold
    /// their argument slot, like glibc's exchange).
    pub fn optind(&self) -> usize {
        self.args.len() - self.operands.len()
    }

    /// Returns the next option (glibc `getopt_long`), or [`Opt::End`] after
    /// the last one. Errors answer [`Opt::Unknown`] and continue scanning.
    pub fn next_opt(&mut self) -> Opt {
        loop {
            // Everything after `--` is an operand.
            if self.end_of_options {
                if self.optind < self.args.len() {
                    self.operands.push(self.args[self.optind].clone());
                    self.optind += 1;
                    continue;
                }
                return Opt::End;
            }

            // Continue a clustered short group ("-vv", "-dhsh", "-t3").
            if self.nextchar != 0 {
                // None = cluster exhausted (scan_short reset nextchar; the
                // token itself was consumed when the cluster started).
                match self.scan_cluster() {
                    Some(opt) => return opt,
                    None => continue,
                }
            }

            if self.optind >= self.args.len() {
                return Opt::End;
            }
            let tok = self.args[self.optind].clone();
            let bytes = tok.to_string_lossy().into_owned();
            let b = bytes.as_bytes();

            if b.len() >= 2 && b[0] == b'-' && b[1] == b'-' {
                // Long option or `--`.
                self.optind += 1;
                if b.len() == 2 {
                    // `--` itself: ends option processing (consumed).
                    self.end_of_options = true;
                    continue;
                }
                return self.scan_long(&bytes);
            }

            if b.len() >= 2 && b[0] == b'-' {
                // Short cluster: `-vv`, `-i8`, `-t` …; a plain `-` is an
                // operand.
                self.optind += 1;
                self.nextchar = 1;
                self.cluster = bytes;
                if let Some(opt) = self.scan_cluster() {
                    return opt;
                }
                continue;
            }

            // Operand (GNU permutation: recorded, scanning continues).
            self.operands.push(tok);
            self.optind += 1;
        }
    }

    /// Scans the next option from `self.cluster` without cloning it per
    /// character: the token is moved out so `scan_short` can borrow `&mut
    /// self` and the cluster text simultaneously, then restored.
    fn scan_cluster(&mut self) -> Option<Opt> {
        let tok = std::mem::take(&mut self.cluster);
        let opt = self.scan_short(&tok);
        self.cluster = tok;
        opt
    }

    /// Scans the short cluster starting at `self.nextchar` in `tok`.
    /// Returns `Some(option)` when one was produced; clears `nextchar` when
    /// the cluster is exhausted (or the token demoted to an operand).
    fn scan_short(&mut self, tok: &str) -> Option<Opt> {
        let chars: Vec<char> = tok.chars().collect();
        // One option per call — every branch below returns; the caller
        // (`next_opt`) re-invokes while `nextchar` still points into the
        // cluster (clippy::never_loop).
        if self.nextchar < chars.len() {
            let c = chars[self.nextchar];
            self.nextchar += 1;
            let spec = short_spec(c);
            let Some(spec) = spec else {
                // glibc: `%s: invalid option -- '%c'`.
                self.error(&format!("invalid option -- '{c}'"));
                return Some(Opt::Unknown);
            };
            match spec {
                b':' => {
                    // Required argument: the cluster rest, else the next
                    // argv token, else the missing-argument error.
                    let rest: String = chars[self.nextchar..].iter().collect();
                    let arg = if !rest.is_empty() {
                        self.nextchar = 0;
                        OsString::from(rest)
                    } else if self.optind < self.args.len() {
                        let a = self.args[self.optind].clone();
                        self.optind += 1;
                        self.nextchar = 0;
                        a
                    } else {
                        self.nextchar = 0;
                        self.error(&format!("option requires an argument -- '{c}'"));
                        return Some(Opt::Unknown);
                    };
                    self.optarg = Some(arg);
                    return Some(Opt::Code(c));
                }
                b';' => {
                    // Optional argument (t::): only an ATTACHED argument
                    // counts, like glibc.
                    let rest: String = chars[self.nextchar..].iter().collect();
                    self.nextchar = 0;
                    self.optarg = if rest.is_empty() {
                        None
                    } else {
                        Some(OsString::from(rest))
                    };
                    return Some(Opt::Code(c));
                }
                _ => {
                    // No argument; the cluster continues after it.
                    self.optarg = None;
                    return Some(Opt::Code(c));
                }
            }
        }
        self.nextchar = 0;
        None
    }

    /// Scans one `--name[=value]` token.
    fn scan_long(&mut self, tok: &str) -> Opt {
        let body = &tok[2..];
        let (name, value) = match body.find('=') {
            Some(eq) => (&body[..eq], Some(&body[eq + 1..])),
            None => (body, None),
        };

        // Exact match wins; otherwise collect prefix matches.
        let exact = OPT_LNG.iter().find(|o| o.name == name);
        let matches: Vec<&LongOpt> = match exact {
            Some(o) => vec![o],
            None => OPT_LNG
                .iter()
                .filter(|o| o.name.starts_with(name))
                .collect(),
        };

        let Some(opt) = (match matches.len() {
            1 => Some(matches[0]),
            0 => None,
            _ => {
                // glibc: `%s: option '%s' is ambiguous; possibilities: …`.
                let mut msg = format!("option '--{name}' is ambiguous; possibilities:");
                for m in &matches {
                    msg.push_str(&format!(" '--{}'", m.name));
                }
                self.error(&msg);
                return Opt::Unknown;
            }
        }) else {
            // glibc: `%s: unrecognized option '%s'` (the full token).
            self.error(&format!("unrecognized option '{tok}'"));
            return Opt::Unknown;
        };

        let arg: Option<OsString> = match (opt.has_arg, value) {
            (HasArg::No, Some(_)) => {
                // glibc: `%s: option '--%s' doesn't allow an argument`.
                self.error(&format!(
                    "option '--{}' doesn't allow an argument",
                    opt.name
                ));
                return Opt::Unknown;
            }
            (HasArg::No, None) => None,
            (HasArg::Optional, v) => v.map(OsString::from),
            (HasArg::Required, Some(v)) => Some(OsString::from(v)),
            (HasArg::Required, None) => {
                if self.optind < self.args.len() {
                    let a = self.args[self.optind].clone();
                    self.optind += 1;
                    Some(a)
                } else {
                    // glibc: `%s: option '--%s' requires an argument`.
                    self.error(&format!("option '--{}' requires an argument", opt.name));
                    return Opt::Unknown;
                }
            }
        };
        self.optarg = arg;
        Opt::Code(opt.val)
    }

    /// One glibc error line on real stderr (libc prints these directly, not
    /// through `JDebug::stddbg`).
    fn error(&self, msg: &str) {
        let mut err = std::io::stderr();
        let _ = writeln!(err, "{}: {}", self.prog.to_string_lossy(), msg);
        let _ = err.flush();
    }
}

/// The optstring entry for a short option: `b':'` required, `b';'` optional,
/// anything else none; `None` = unknown option.
fn short_spec(c: char) -> Option<u8> {
    let bytes = OPT_SHT.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] as char == c {
            return Some(match bytes.get(i + 1) {
                Some(b':') if bytes.get(i + 2) == Some(&b':') => b';', // optional
                Some(b':') => b':',                                    // required
                _ => b' ',                                             // none
            });
        }
        i += 1;
    }
    None
}

/// Unit tests for the scanner semantics pinned against the oracle in
/// tests/roundtrip.rs (the `?`-continues, permutation, `--`, error-message
/// and optional-argument behaviors).
#[cfg(test)]
mod tests {
    use super::*;

    fn scan(args: &[&str]) -> (Vec<Opt>, Vec<String>, Option<String>) {
        let mut g = Getopt::new(
            std::iter::once(OsString::from("jdiff"))
                .chain(args.iter().map(OsString::from))
                .collect(),
        );
        let mut opts = Vec::new();
        loop {
            match g.next_opt() {
                Opt::End => break,
                other => opts.push(other),
            }
        }
        let operands = g
            .operands()
            .iter()
            .map(|o| o.to_string_lossy().into_owned())
            .collect();
        let optarg = g.optarg.as_ref().map(|o| o.to_string_lossy().into_owned());
        (opts, operands, optarg)
    }

    #[test]
    fn short_options_and_clusters() {
        let (opts, operands, _) = scan(&["-vv", "a", "b"]);
        assert_eq!(opts, vec![Opt::Code('v'), Opt::Code('v')], "clustered -vv");
        assert_eq!(operands, ["a", "b"]);

        // Required argument: attached, detached and clustered.
        let (opts, _, optarg) = scan(&["-i8"]);
        assert_eq!(opts, vec![Opt::Code('i')]);
        assert_eq!(optarg.as_deref(), Some("8"));

        let (opts, operands, _) = scan(&["-i", "8", "a", "b"]);
        assert_eq!(opts, vec![Opt::Code('i')]);
        assert_eq!(operands, ["a", "b"]);

        // Clustered with argument: `-vi8` yields v then i/8.
        let (opts, _, optarg) = scan(&["-vi8"]);
        assert_eq!(opts, vec![Opt::Code('v'), Opt::Code('i')]);
        assert_eq!(optarg.as_deref(), Some("8"));
    }

    #[test]
    fn optional_argument_only_when_attached() {
        // Bare -t does not consume the next token (glibc `t::`).
        let (opts, operands, optarg) = scan(&["-t", "a"]);
        assert_eq!(opts, vec![Opt::Code('t')]);
        assert_eq!(operands, ["a"]);
        assert_eq!(optarg, None);

        // Attached: -t3 → optarg "3".
        let (_, operands, optarg) = scan(&["-t3", "a"]);
        assert_eq!(operands, ["a"]);
        assert_eq!(optarg.as_deref(), Some("3"));
    }

    #[test]
    fn long_options_and_equals() {
        let (opts, _, optarg) = scan(&["--undiff", "--index-size=1"]);
        assert_eq!(opts, vec![Opt::Code('u'), Opt::Code('i')]);
        assert_eq!(optarg.as_deref(), Some("1"));

        // Required argument may be detached.
        let (opts, _, optarg) = scan(&["--index-size", "1"]);
        assert_eq!(opts, vec![Opt::Code('i')]);
        assert_eq!(optarg.as_deref(), Some("1"));

        // Optional long argument: `--test` bare vs `--test=2`.
        let (_, _, optarg) = scan(&["--test"]);
        assert_eq!(optarg, None);
        let (_, _, optarg) = scan(&["--test=2"]);
        assert_eq!(optarg.as_deref(), Some("2"));
    }

    #[test]
    fn abbreviation_and_ambiguity() {
        // Unique prefix resolves.
        let (opts, _, _) = scan(&["--und"]);
        assert_eq!(opts, vec![Opt::Code('u')]);

        // `--s` is ambiguous (sequential-source/-dest/stdio/search-*); the
        // exact-name match still wins over prefixes.
        let (opts, _, _) = scan(&["--s"]);
        assert_eq!(opts, vec![Opt::Unknown]);
        let (opts, _, _) = scan(&["--stdio"]);
        assert_eq!(opts, vec![Opt::Code('s')]);
    }

    #[test]
    fn unknown_short_continues() {
        let (opts, operands, _) = scan(&["-Z", "a", "b", "c"]);
        assert_eq!(opts, vec![Opt::Unknown]);
        assert_eq!(operands, ["a", "b", "c"]);
    }

    #[test]
    fn missing_required_argument_errors() {
        // At end of argv: '?' with the operand set untouched.
        let (opts, operands, _) = scan(&["a", "b", "-d"]);
        assert_eq!(opts, vec![Opt::Unknown]);
        assert_eq!(operands, ["a", "b"]);

        // Mid-argv the next token becomes the argument.
        let (opts, operands, optarg) = scan(&["-d", "hsh", "a", "b"]);
        assert_eq!(opts, vec![Opt::Code('d')]);
        assert_eq!(operands, ["a", "b"]);
        assert_eq!(optarg.as_deref(), Some("hsh"));

        // Bare long option at end of argv.
        let (opts, _, _) = scan(&["--index-size"]);
        assert_eq!(opts, vec![Opt::Unknown]);
    }

    #[test]
    fn gnu_permutation_and_lone_dash() {
        // Options after, between and before operands.
        let (_, operands, _) = scan(&["a", "-l", "b", "-v"]);
        assert_eq!(operands, ["a", "b"]);

        // A lone `-` is an operand; everything after `--` too (including
        // another `--`).
        let (opts, operands, _) = scan(&["--", "a", "--", "b"]);
        assert_eq!(opts, vec![]);
        assert_eq!(operands, ["a", "--", "b"]);

        let (_, operands, _) = scan(&["-"]);
        assert_eq!(operands, ["-"]);
    }

    #[test]
    fn optind_counts_option_slots() {
        let mut g = Getopt::new(
            ["jdiff", "-i", "8", "a", "b", "-v"]
                .iter()
                .map(OsString::from)
                .collect(),
        );
        while g.next_opt() != Opt::End {}
        // Options + detached arguments hold slots 1..3; glibc permutes argv
        // to [jdiff, -i, 8, -v, a, b], so the first operand sits at index 4.
        assert_eq!(g.optind(), 4);
        assert_eq!(g.operands().len(), 2);

        let mut g = Getopt::new(["jdiff", "a", "b"].iter().map(OsString::from).collect());
        while g.next_opt() != Opt::End {}
        assert_eq!(g.optind(), 1);
    }

    /// `short_spec` reflects the `main.cpp:236` optstring.
    #[test]
    fn short_specs_match_optstring() {
        assert_eq!(short_spec('a'), Some(b':')); // required
        assert_eq!(short_spec('b'), Some(b' ')); // none
        assert_eq!(short_spec('t'), Some(b';')); // optional
        assert_eq!(short_spec('y'), Some(b' '));
        assert_eq!(short_spec('Z'), None);
        // Every code in the long table exists in the short string — except
        // the long-only `--compat-081` sentinel (port-only, §21.16), which
        // the short string must NOT contain.
        for o in OPT_LNG {
            if o.val == VAL_COMPAT_081 {
                assert_eq!(short_spec(o.val), None, "--compat-081 is long-only");
            } else {
                assert!(short_spec(o.val).is_some(), "-{}", o.val);
            }
        }
    }

    /// The long-only `--compat-081` (§21.16): resolves exactly (and via its
    /// unique `--compat` prefix), takes no argument, and leaves `--c`
    /// ambiguous (console | compat-081).
    #[test]
    fn compat_081_long_only_option() {
        let (opts, operands, _) = scan(&["--compat-081", "a", "b"]);
        assert_eq!(opts, vec![Opt::Code(VAL_COMPAT_081)]);
        assert_eq!(operands, ["a", "b"]);

        let (opts, _, _) = scan(&["--compat"]);
        assert_eq!(opts, vec![Opt::Code(VAL_COMPAT_081)], "unique prefix");

        let (opts, _, _) = scan(&["--c"]);
        assert_eq!(opts, vec![Opt::Unknown], "--c stays ambiguous");

        // A value argument is rejected ("doesn't allow an argument" → '?').
        let (opts, _, _) = scan(&["--compat-081=x"]);
        assert_eq!(opts, vec![Opt::Unknown]);
    }
}
