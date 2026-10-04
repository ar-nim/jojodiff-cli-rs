//! The single error→(code, stderr) boundary of the `jdiff` binary
//! (`main.cpp:897-932`): every `JDiffError` and every plain-`i32` success
//! code maps to the byte-pinned stderr text (the legacy `exit_switch`
//! texts — including their leading blank line and trailing " !"-forms) and
//! the positive process exit code. All output goes through `dbg_print`,
//! like every other legacy print, so `-c` keeps routing it to stdout.
//!
//! One text, printed once, per error class — reconciled against the
//! byte-pinned round-trip suite (`tests/roundtrip.rs`):
//!
//! - **Open failures** (`OpenFirst`/`OpenSecond`/`OpenOutput`) print the
//!   name-carrying `#[error]` text plus a newline, WITHOUT the engine
//!   arms' leading blank line (`unopenable_org_exit_3_message` et al.).
//! - **Engine errors** and `NotPorted` print `"\n{Display}\n"` — the
//!   `Display` texts are the `exit_switch` arm texts and the boundary
//!   supplies the surrounding blank line and newline (`u_read_write_error_exits`).
//! - **`Raw(c)`** dispatches BY CODE VALUE, never via `Display` (whose
//!   string is coincidentally wrong for `Raw(EXI_ERR)`): `c == EXI_ERR`
//!   prints `"\nError occurred !\n"`, anything else the fall-through
//!   `"\nUnknown exit code {c}\n"` — both exit `-EXI_ERR` (20), like the
//!   C switch's arm and fall-through (`u_trailing_byte_warning_exit_20`,
//!   `u_negative_length_unknown_exit_code`).
//! - **`Args`** prints NOTHING here: its two binary call sites carry their
//!   own distinct, `liHlp`-gated texts ("Error: Not enough arguments have
//!   been specified !" only when `liHlp == 0`; "Error: Original and
//!   destination files cannot both be from standard input !"), pinned by
//!   `missing_args_exit_2` / `help_exit_code_and_text` /
//!   `both_inputs_dash_exit_2`. The boundary only carries the code.
//! - The `JFileOut` library family ("Error reading source file." etc.)
//!   keeps printing at its own failure point in the library; this boundary
//!   does not duplicate it — the pinned stderr shows both lines in order.
//!
//! Success codes keep the 0.8.5 swap: EXI_EQL→0, EXI_DIF→1, with the
//! verbose verdict lines (`main.cpp:897-932`).

use crate::defs::{
    EXI_ARG, EXI_DIF, EXI_EQL, EXI_ERR, EXI_LRG, EXI_MEM, EXI_OK, EXI_RED, EXI_SEK, EXI_WRI,
};
use crate::error::JDiffError;
use crate::jdebug::dbg_print;

/// The single error→(code, stderr) boundary. Prints the pinned message for
/// the error half (see the module docs for the per-class reconciliation)
/// and returns the positive process exit code; the success half keeps the
/// legacy `exit_switch` behavior.
pub fn report(li_ret: Result<i32, JDiffError>, verbose: i32) -> i32 {
    match li_ret {
        Ok(li_ret) => exit_switch(li_ret, verbose),
        Err(e) => {
            if let Some(text) = error_text(&e) {
                dbg_print(format_args!("{text}"));
            }
            match e {
                // BOTH Raw arms exit -EXI_ERR (20): the arm for the EXI_ERR
                // value and the fall-through, like the C switch — the raw
                // value itself is NOT the process code.
                JDiffError::Raw(_) => -EXI_ERR,
                _ => -e.exit_code(),
            }
        }
    }
}

/// The pinned stderr chunk for one error, printed exactly once at boundary
/// call time; `None` for [`JDiffError::Args`] (its texts print at the two
/// gated call sites in the binary). Unit-pinned below, byte-pinned by the
/// round-trip suite.
fn error_text(e: &JDiffError) -> Option<String> {
    match e {
        JDiffError::Args => None,
        // Raw dispatches BY CODE VALUE (controller ruling): the C switch's
        // EXI_ERR arm for EXI_ERR itself, the fall-through for everything
        // else.
        JDiffError::Raw(code) if *code == EXI_ERR => Some("\nError occurred !\n".to_string()),
        JDiffError::Raw(code) => Some(format!("\nUnknown exit code {code}\n")),
        // Open failures: no leading blank line (pinned bytes).
        JDiffError::OpenFirst { .. }
        | JDiffError::OpenSecond { .. }
        | JDiffError::OpenOutput { .. } => Some(format!("{e}\n")),
        // Engine family + NotPorted: the exit-switch text, wrapped in the
        // arm's blank line and newline.
        _ => Some(format!("\n{e}\n")),
    }
}

/// The exit-code switch for the plain-`i32` vocabulary (`main.cpp:897-932`),
/// the `Ok` half of [`report`]: engine errors print their message and return
/// `-EXI_*` (the positive process code); patch success returns 0;
/// EXI_EQL/EXI_DIF map to the swapped 0/1 with the verbose verdict lines.
/// Every arm returns, so the tail is unreachable.
fn exit_switch(li_ret: i32, verbose: i32) -> i32 {
    match li_ret {
        r if r == EXI_SEK => {
            dbg_print(format_args!("\nSeek error !\n"));
            -EXI_SEK
        }
        r if r == EXI_LRG => {
            dbg_print(format_args!("\nError: 64-bit offsets not supported !\n"));
            -EXI_LRG
        }
        r if r == EXI_RED => {
            dbg_print(format_args!("\nError reading file !\n"));
            -EXI_RED
        }
        r if r == EXI_WRI => {
            dbg_print(format_args!("\nError writing file !\n"));
            -EXI_WRI
        }
        r if r == EXI_MEM => {
            dbg_print(format_args!("\nError allocating memory !\n"));
            -EXI_MEM
        }
        r if r == EXI_ARG => {
            dbg_print(format_args!("\nError in arguments !\n"));
            -EXI_ARG
        }
        r if r == EXI_ERR => {
            dbg_print(format_args!("\nError occurred !\n"));
            -EXI_ERR
        }
        EXI_OK => EXI_OK,
        EXI_EQL => {
            if verbose > 1 {
                dbg_print(format_args!("\nFound all data within source file.\n"));
            }
            EXI_OK
        }
        EXI_DIF => {
            if verbose > 1 {
                dbg_print(format_args!(
                    "\nNot all data has been found in source file.\n"
                ));
            }
            EXI_DIF
        }
        _ => {
            dbg_print(format_args!("\nUnknown exit code {}\n", li_ret));
            -EXI_ERR
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn src() -> std::io::Error {
        std::io::Error::other("x")
    }

    /// Every variant's process code is the negated `EXI_*` (the brief's
    /// `exit_codes_are_the_exi_negation`, concretized) — success codes keep
    /// the 0.8.5 swap.
    #[test]
    fn exit_codes_are_the_exi_negation() {
        assert_eq!(report(Ok(EXI_EQL), 0), 0);
        assert_eq!(report(Ok(EXI_DIF), 0), 1);
        assert_eq!(report(Ok(EXI_OK), 0), 0);
        assert_eq!(report(Err(JDiffError::Args), 0), 2);
        assert_eq!(
            report(
                Err(JDiffError::OpenFirst {
                    name: std::ffi::OsString::new(),
                    source: src(),
                }),
                0
            ),
            3
        );
        assert_eq!(
            report(
                Err(JDiffError::OpenSecond {
                    name: std::ffi::OsString::new(),
                    source: src(),
                }),
                0
            ),
            4
        );
        assert_eq!(
            report(
                Err(JDiffError::OpenOutput {
                    name: std::ffi::OsString::new(),
                    append: true,
                    source: src(),
                }),
                0
            ),
            5
        );
        assert_eq!(report(Err(JDiffError::Seek), 0), 6);
        assert_eq!(report(Err(JDiffError::Large), 0), 7);
        assert_eq!(report(Err(JDiffError::Read), 0), 8);
        assert_eq!(report(Err(JDiffError::Write(src())), 0), 9);
        assert_eq!(report(Err(JDiffError::Memory), 0), 10);
        assert_eq!(report(Err(JDiffError::NotPorted), 0), 20);
    }

    /// CONTROLLER RULING, binding: `Raw` dispatches BY CODE VALUE —
    /// `Raw(EXI_ERR)` prints the arm text "\nError occurred !\n" and any
    /// other `Raw(c)` the fall-through "\nUnknown exit code {c}\n"; BOTH
    /// exit 20. Never via `Display`: `Raw(EXI_ERR)`'s Display string is
    /// coincidentally "Unknown exit code -20".
    #[test]
    fn raw_dispatches_by_code_value() {
        assert_eq!(
            error_text(&JDiffError::Raw(EXI_ERR)).as_deref(),
            Some("\nError occurred !\n")
        );
        assert_eq!(
            error_text(&JDiffError::Raw(-257)).as_deref(),
            Some("\nUnknown exit code -257\n")
        );
        assert_eq!(report(Err(JDiffError::Raw(EXI_ERR)), 0), 20);
        assert_eq!(report(Err(JDiffError::Raw(-257)), 0), 20);
        assert_ne!(
            JDiffError::Raw(EXI_ERR).to_string(),
            "Error occurred !",
            "Display must not feed the Raw dispatch"
        );
    }

    /// Open failures print WITHOUT the engine arms' leading blank line
    /// (`tests/roundtrip.rs` `unopenable_org_exit_3_message` et al.), via
    /// the variant's Display text + newline; `append` does not alter the
    /// text.
    #[test]
    fn open_failure_texts_have_no_leading_blank_line() {
        let name = std::ffi::OsString::from("no/such");
        assert_eq!(
            error_text(&JDiffError::OpenFirst {
                name: name.clone(),
                source: src(),
            })
            .as_deref(),
            Some("Could not open first file no/such for reading.\n")
        );
        assert_eq!(
            error_text(&JDiffError::OpenSecond {
                name: name.clone(),
                source: src(),
            })
            .as_deref(),
            Some("Could not open second file no/such for reading.\n")
        );
        assert_eq!(
            error_text(&JDiffError::OpenOutput {
                name: name.clone(),
                append: false,
                source: src(),
            })
            .as_deref(),
            Some("Could not open output file no/such for writing.\n")
        );
        assert_eq!(
            error_text(&JDiffError::OpenOutput {
                name,
                append: true,
                source: src(),
            })
            .as_deref(),
            Some("Could not open output file no/such for writing.\n")
        );
    }

    /// `Args` is silent at the boundary: its two call sites print their own
    /// gated, distinct texts (missing_args_exit_2 / help_exit_code_and_text
    /// / both_inputs_dash_exit_2 pins).
    #[test]
    fn args_is_silent_at_the_boundary() {
        assert_eq!(error_text(&JDiffError::Args), None);
    }

    /// The engine family and NotPorted print the exit-switch text with the
    /// arm's blank-line + newline wrap.
    #[test]
    fn engine_texts_get_the_blank_line_wrap() {
        assert_eq!(
            error_text(&JDiffError::Seek).as_deref(),
            Some("\nSeek error !\n")
        );
        assert_eq!(
            error_text(&JDiffError::Read).as_deref(),
            Some("\nError reading file !\n")
        );
        assert_eq!(
            error_text(&JDiffError::Write(src())).as_deref(),
            Some("\nError writing file !\n")
        );
        assert_eq!(
            error_text(&JDiffError::Memory).as_deref(),
            Some("\nError allocating memory !\n")
        );
        assert_eq!(
            error_text(&JDiffError::Large).as_deref(),
            Some("\nError: 64-bit offsets not supported !\n")
        );
        assert_eq!(
            error_text(&JDiffError::NotPorted).as_deref(),
            Some("\nError occurred !\n")
        );
    }
}
