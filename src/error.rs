//! Library-wide error type: the negative `EXI_*` C vocabulary as a Rust
//! type. Engine APIs return `Result<_, JDiffError>`; the CLI boundary maps
//! [`JDiffError::exit_code`] to the process exit code (negating, like
//! `exit(-EXI_*)`) and prints the pinned texts.

/// Engine error (`EXI_*`, `JDefs.h:146-158`).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive] // API-07: CLI variants arrive in Phase 4; downstream must not match exhaustively
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
}
