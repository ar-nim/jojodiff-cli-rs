//! Library-wide error type: the negative `EXI_*` C vocabulary as a Rust
//! type. Engine APIs return `Result<_, JDiffError>`; the CLI boundary maps
//! [`JDiffError::exit_code`] to the process exit code (negating, like
//! `exit(-EXI_*)`) and prints the pinned texts.

use std::ffi::OsString;

/// Engine and CLI error (`EXI_*`, `JDefs.h:146-158`).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive] // API-07: open vocabulary — downstream must not match exhaustively
pub enum JDiffError {
    /// `EXI_SEK` — seek error (`JFileAhead.h:115`).
    #[error("Seek error !")]
    Seek,
    /// `EXI_RED` — read error.
    #[error("Error reading file !")]
    Read,
    /// `EXI_WRI` — write error, with the underlying I/O failure.
    #[error("Error writing file !")]
    Write(#[from] std::io::Error),
    /// `EXI_MEM` — allocation failure.
    #[error("Error allocating memory !")]
    Memory,
    /// `EXI_LRG` — 64-bit number rejected.
    #[error("Error: 64-bit offsets not supported !")]
    Large,
    /// A raw non-`EXI_*` sentinel that the exit switch dispatches by its
    /// value: `JPatcht::jpatch`'s truncated 64-bit-length returns (arbitrary
    /// negative numbers — the exit switch's fall-through arm prints
    /// `"Unknown exit code <n>"` and exits `-EXI_ERR`) and `EXI_ERR` itself
    /// (its own arm prints `"Error occurred !"`). No library `Display` text
    /// is pinned to this variant; the boundary prints the message.
    #[error("Unknown exit code {0}")]
    Raw(i32),
    /// `EXI_ARG` — not enough arguments, or both inputs from stdin. The two
    /// binary call sites print their own distinct, gated texts (the
    /// "Error: Not enough arguments…" line only when `liHlp == 0`), so the
    /// boundary itself prints nothing for this variant; its `Display` text
    /// is the legacy `exit_switch` `EXI_ARG` arm.
    #[error("Error in arguments !")]
    Args,
    /// `EXI_FRT` — could not open the first (source) file. The name prints
    /// lossily, like the old `to_string_lossy` sites.
    #[error("Could not open first file {} for reading.", name.to_string_lossy())]
    OpenFirst {
        name: OsString,
        #[source]
        source: std::io::Error,
    },
    /// `EXI_SCD` — could not open the second (destination/patch) file.
    #[error("Could not open second file {} for reading.", name.to_string_lossy())]
    OpenSecond {
        name: OsString,
        #[source]
        source: std::io::Error,
    },
    /// `EXI_OUT` — could not open the output file (create or, for `-t`'s
    /// patch phase, append); `append` records which open failed.
    #[error("Could not open output file {} for writing.", name.to_string_lossy())]
    OpenOutput {
        name: OsString,
        append: bool,
        #[source]
        source: std::io::Error,
    },
    /// `EXI_ERR` — the un-ported dedup route (§21.4): the files open, then
    /// the C++ crash path becomes this error ("Error occurred !").
    #[error("Error occurred !")]
    NotPorted,
}

impl JDiffError {
    /// The raw negative engine code (`EXI_SEK` etc.); the process exit code
    /// is its negation, as in `exit(-EXI_*)`.
    pub fn exit_code(&self) -> i32 {
        match self {
            JDiffError::Seek => crate::defs::EXI_SEK,
            JDiffError::Read => crate::defs::EXI_RED,
            JDiffError::Write(_) => crate::defs::EXI_WRI,
            JDiffError::Memory => crate::defs::EXI_MEM,
            JDiffError::Large => crate::defs::EXI_LRG,
            JDiffError::Raw(code) => *code,
            JDiffError::Args => crate::defs::EXI_ARG,
            JDiffError::OpenFirst { .. } => crate::defs::EXI_FRT,
            JDiffError::OpenSecond { .. } => crate::defs::EXI_SCD,
            JDiffError::OpenOutput { .. } => crate::defs::EXI_OUT,
            JDiffError::NotPorted => crate::defs::EXI_ERR,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ERRORS-12 / API-15: the error type crosses `anyhow` and thread
    /// boundaries in the binary — lock `Send + Sync + 'static` at compile time.
    #[test]
    fn jdiff_error_is_send_sync_static() {
        fn assert_bounds<T: Send + Sync + 'static>() {}
        assert_bounds::<JDiffError>();
    }

    #[test]
    fn exit_codes_match_exi() {
        assert_eq!(JDiffError::Seek.exit_code(), crate::defs::EXI_SEK);
        assert_eq!(JDiffError::Read.exit_code(), crate::defs::EXI_RED);
        assert_eq!(JDiffError::Memory.exit_code(), crate::defs::EXI_MEM);
        assert_eq!(JDiffError::Large.exit_code(), crate::defs::EXI_LRG);
    }

    /// The `#[error]` texts are byte-pinned: the CLI boundary prints them
    /// (surrounded by the exit switch's `\n` … `\n`) for the engine-error
    /// exits (`main.cpp:897-932`, `src/bin/jdiff.rs` `exit_switch`). Display
    /// itself stays newline-free; the newlines belong to the boundary print.
    #[test]
    fn display_texts_are_byte_pinned() {
        assert_eq!(JDiffError::Seek.to_string(), "Seek error !");
        assert_eq!(JDiffError::Read.to_string(), "Error reading file !");
        assert_eq!(
            JDiffError::Write(std::io::Error::other("x")).to_string(),
            "Error writing file !"
        );
        assert_eq!(JDiffError::Memory.to_string(), "Error allocating memory !");
        assert_eq!(
            JDiffError::Large.to_string(),
            "Error: 64-bit offsets not supported !"
        );
    }

    /// `Write` converts from `std::io::Error` via `#[from]`.
    #[test]
    fn write_from_io_error() {
        let err: JDiffError = std::io::Error::other("boom").into();
        assert!(matches!(err, JDiffError::Write(_)));
        assert_eq!(err.exit_code(), crate::defs::EXI_WRI);
    }

    /// `Raw` carries the raw sentinel value through `exit_code` untouched:
    /// the truncated-length returns (`tests/roundtrip.rs`: "Unknown exit
    /// code -257") and `EXI_ERR` ("Error occurred !") are dispatched by the
    /// boundary's exit switch on the code value itself.
    #[test]
    fn raw_carries_the_sentinel_value() {
        assert_eq!(JDiffError::Raw(-257).exit_code(), -257);
        assert_eq!(
            JDiffError::Raw(crate::defs::EXI_ERR).exit_code(),
            crate::defs::EXI_ERR
        );
    }

    /// The CLI variants map to their `EXI_*` codes (`EXI_ARG`/`EXI_FRT`/
    /// `EXI_SCD`/`EXI_OUT`/`EXI_ERR`); the process exit code is the negation.
    #[test]
    fn cli_exit_codes_match_exi() {
        fn src() -> std::io::Error {
            std::io::Error::other("x")
        }
        assert_eq!(JDiffError::Args.exit_code(), crate::defs::EXI_ARG);
        assert_eq!(
            JDiffError::OpenFirst {
                name: OsString::new(),
                source: src(),
            }
            .exit_code(),
            crate::defs::EXI_FRT
        );
        assert_eq!(
            JDiffError::OpenSecond {
                name: OsString::new(),
                source: src(),
            }
            .exit_code(),
            crate::defs::EXI_SCD
        );
        assert_eq!(
            JDiffError::OpenOutput {
                name: OsString::new(),
                append: true,
                source: src(),
            }
            .exit_code(),
            crate::defs::EXI_OUT
        );
        assert_eq!(JDiffError::NotPorted.exit_code(), crate::defs::EXI_ERR);
    }

    /// The CLI `#[error]` texts are byte-pinned from the `main.cpp` print
    /// sites (verbatim including the " !" spacing); file names render
    /// lossily, like the old `to_string_lossy` print sites. The boundary
    /// adds the surrounding newlines (`cli::error`).
    #[test]
    fn cli_display_texts_are_byte_pinned() {
        fn src() -> std::io::Error {
            std::io::Error::other("x")
        }
        assert_eq!(JDiffError::Args.to_string(), "Error in arguments !");
        assert_eq!(
            JDiffError::OpenFirst {
                name: OsString::from("no/such"),
                source: src(),
            }
            .to_string(),
            "Could not open first file no/such for reading."
        );
        assert_eq!(
            JDiffError::OpenSecond {
                name: OsString::from("no/such"),
                source: src(),
            }
            .to_string(),
            "Could not open second file no/such for reading."
        );
        assert_eq!(
            JDiffError::OpenOutput {
                name: OsString::from("no/such"),
                append: true,
                source: src(),
            }
            .to_string(),
            "Could not open output file no/such for writing."
        );
        assert_eq!(JDiffError::NotPorted.to_string(), "Error occurred !");
    }

    /// Non-UTF-8 names print lossily (U+FFFD), exactly like the superseded
    /// `to_string_lossy` sites in the binary.
    #[cfg(unix)]
    #[test]
    fn open_failure_names_print_lossily() {
        use std::os::unix::ffi::OsStrExt;
        let name = OsString::from(std::ffi::OsStr::from_bytes(b"bad\xffname"));
        assert_eq!(
            JDiffError::OpenFirst {
                name,
                source: std::io::Error::other("x"),
            }
            .to_string(),
            "Could not open first file bad\u{FFFD}name for reading."
        );
    }
}
