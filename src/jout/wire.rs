//! Shared wire-length tier knowledge for the patch format: the length
//! classification (`JOutBin::ufPutLen` `JOutBin.cpp:65-104`), the real
//! encoded sizes 1/2/3/5/9 (`JOutAsc::ufPutSze` `JOutAsc.cpp:107-126`), and
//! the decoder's lead-byte classification (`JPatcht::ufGetInt`
//! `JPatcht.cpp:50-85`). One definition so encoder, size calculators and
//! decoder cannot drift apart.

/// The five wire-length tiers of the patch format (see module docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LenTier {
    /// 1..=252 — encoded as the length minus one in a single byte.
    L252,
    /// 253..=508 — marker 252 plus one byte.
    L508,
    /// 16-bit tier — marker 253 plus two bytes.
    L16,
    /// 32-bit tier — marker 254 plus four bytes.
    L32,
    /// 64-bit tier (`JDIFF_LARGEFILE`, live in the oracle and the port) —
    /// marker 255 plus eight bytes. The oracle build enables it unconditionally
    /// via `-D_FILE_OFFSET_BITS=64` (`JDefs.h:64-67`).
    L64,
}

/// Which tier a length encodes into (`JOutBin.cpp:65-104` boundaries).
pub(crate) fn len_tier(len: i64) -> LenTier {
    if len <= 252 {
        LenTier::L252
    } else if len <= 508 {
        LenTier::L508
    } else if len <= 0xffff {
        LenTier::L16
    } else if len <= 0xffff_ffff {
        LenTier::L32
    } else {
        LenTier::L64
    }
}

impl LenTier {
    /// Which tier a decoder lead byte introduces (`JPatcht.cpp:50-85`).
    /// Negative leads classify as L252 so the C arithmetic
    /// (`li_val = EOF` → `li_val + 1 = 0`) is preserved.
    pub(crate) fn from_lead(lead: i64) -> LenTier {
        match lead {
            i64::MIN..=251 => LenTier::L252,
            252 => LenTier::L508,
            253 => LenTier::L16,
            254 => LenTier::L32,
            _ => LenTier::L64,
        }
    }

    /// Real encoded size in bytes: 1/2/3/5/9.
    pub(crate) fn size(self) -> i64 {
        match self {
            LenTier::L252 => 1,
            LenTier::L508 => 2,
            LenTier::L16 => 3,
            LenTier::L32 => 5,
            LenTier::L64 => 9,
        }
    }

    /// Marker byte of the multi-byte tiers; `None` for L252 (its single
    /// byte is the length itself, minus one).
    pub(crate) fn marker(self) -> Option<u8> {
        match self {
            LenTier::L252 => None,
            LenTier::L508 => Some(252),
            LenTier::L16 => Some(253),
            LenTier::L32 => Some(254),
            LenTier::L64 => Some(255),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn len_tier_boundaries() {
        assert!(matches!(len_tier(1), LenTier::L252));
        assert!(matches!(len_tier(252), LenTier::L252));
        assert!(matches!(len_tier(253), LenTier::L508));
        assert!(matches!(len_tier(508), LenTier::L508));
        assert!(matches!(len_tier(509), LenTier::L16));
        assert!(matches!(len_tier(0xffff), LenTier::L16));
        assert!(matches!(len_tier(0x1_0000), LenTier::L32));
        assert!(matches!(len_tier(0xffff_ffff), LenTier::L32));
        assert!(matches!(len_tier(0x1_0000_0000), LenTier::L64));
    }

    #[test]
    fn real_sizes() {
        assert_eq!(LenTier::L252.size(), 1);
        assert_eq!(LenTier::L508.size(), 2);
        assert_eq!(LenTier::L16.size(), 3);
        assert_eq!(LenTier::L32.size(), 5);
        assert_eq!(LenTier::L64.size(), 9);
    }

    #[test]
    fn from_lead_classification() {
        assert!(matches!(LenTier::from_lead(0), LenTier::L252));
        assert!(matches!(LenTier::from_lead(251), LenTier::L252));
        assert!(matches!(LenTier::from_lead(-1), LenTier::L252)); // EOF arithmetic
        assert!(matches!(LenTier::from_lead(252), LenTier::L508));
        assert!(matches!(LenTier::from_lead(253), LenTier::L16));
        assert!(matches!(LenTier::from_lead(254), LenTier::L32));
        assert!(matches!(LenTier::from_lead(255), LenTier::L64));
    }
}
