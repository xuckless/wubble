//! mozLz4 framing used by Firefox/Zen `*.jsonlz4`, `*.baklz4`.
//!
//! Source: mozilla-firefox/firefox `xpcom/ioutils/IOUtils.cpp` `IOUtils::MozLZ4::Decompress`
//! and `IOUtils.h` (`MAGIC_NUMBER`, `HEADER_SIZE`), MPL-2.0. Layout: 8-byte magic
//! `mozLz40\0`, u32 little-endian decoded size, one raw LZ4 block (not the LZ4 frame format).

use crate::ImportError;

const MAGIC: &[u8; 8] = b"mozLz40\0";
/// Encrypted session files (IOUtils.h); the key lives in the OS keystore.
const MAGIC_ENCRYPTED: &[u8; 8] = b"mozEnc0\0";
const HEADER: usize = 12;
/// A session file is a few MiB at most; the size field is untrusted.
pub const MAX_DECODED: usize = 64 << 20;

pub fn decode(bytes: &[u8]) -> Result<Vec<u8>, ImportError> {
    if bytes.starts_with(MAGIC_ENCRYPTED) {
        return Err(ImportError::Unsupported("encrypted session file"));
    }
    if bytes.len() < HEADER || !bytes.starts_with(MAGIC) {
        return Err(ImportError::Unreadable("not a mozLz4 file"));
    }
    let size = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    if size == 0 {
        return Ok(Vec::new());
    }
    if size > MAX_DECODED {
        return Err(ImportError::Unreadable("mozLz4 size above limit"));
    }
    let out = lz4_flex::block::decompress(&bytes[HEADER..], size)
        .map_err(|_| ImportError::Unreadable("bad LZ4 block"))?;
    if out.len() != size {
        return Err(ImportError::Unreadable("mozLz4 size mismatch"));
    }
    Ok(out)
}

/// Inverse of [`decode`], for building fixtures.
#[cfg(test)]
pub(crate) fn encode(data: &[u8]) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&lz4_flex::block::compress(data));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let data = br#"{"windows":[{"tabs":[]}]}"#.repeat(20);
        assert_eq!(decode(&encode(&data)).unwrap(), data);
    }

    #[test]
    fn bad_magic_and_truncated_header() {
        assert!(matches!(
            decode(b"notmozlz4data"),
            Err(ImportError::Unreadable(_))
        ));
        assert!(matches!(
            decode(b"mozLz40\0\x01"),
            Err(ImportError::Unreadable(_))
        ));
        assert!(matches!(decode(b""), Err(ImportError::Unreadable(_))));
    }

    #[test]
    fn size_field_wrong_in_both_directions() {
        let data = b"hello hello hello hello hello hello".to_vec();
        let mut small = encode(&data);
        small[8..12].copy_from_slice(&((data.len() - 5) as u32).to_le_bytes());
        assert!(decode(&small).is_err());
        let mut big = encode(&data);
        big[8..12].copy_from_slice(&((data.len() + 5) as u32).to_le_bytes());
        assert!(decode(&big).is_err());
    }

    #[test]
    fn size_zero_is_empty() {
        let mut f = MAGIC.to_vec();
        f.extend_from_slice(&0u32.to_le_bytes());
        assert_eq!(decode(&f).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn size_above_cap_is_rejected_without_allocating() {
        let mut f = MAGIC.to_vec();
        f.extend_from_slice(&u32::MAX.to_le_bytes());
        f.push(0);
        assert!(matches!(decode(&f), Err(ImportError::Unreadable(_))));
    }

    #[test]
    fn encrypted_is_unsupported() {
        assert!(matches!(
            decode(b"mozEnc0\0\x01\0\0\0abc"),
            Err(ImportError::Unsupported(_))
        ));
    }
}
