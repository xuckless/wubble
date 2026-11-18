//! Password generator on the OS CSPRNG.

use rand::rngs::OsRng;
use rand::seq::SliceRandom;
use zeroize::Zeroizing;

pub const MIN_LEN: usize = 4;
pub const MAX_LEN: usize = 128;
const LOWER: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
const UPPER: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &[u8] = b"0123456789";
const SYMBOLS: &[u8] = b"!@#$%^&*()-_=+[]{};:,.<>?/~";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GeneratorOptions {
    pub length: usize,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
}

impl Default for GeneratorOptions {
    fn default() -> Self {
        GeneratorOptions {
            length: 20,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GenError {
    #[error("enable at least one character class")]
    NoClass,
    #[error("length must be between {MIN_LEN} and {MAX_LEN}")]
    BadLength,
}

/// One character of every enabled class is guaranteed; the rest is drawn
/// uniformly from the union, then shuffled.
pub fn generate(opts: &GeneratorOptions) -> Result<Zeroizing<String>, GenError> {
    if !(MIN_LEN..=MAX_LEN).contains(&opts.length) {
        return Err(GenError::BadLength);
    }
    let classes: Vec<&[u8]> = [
        (opts.lower, LOWER),
        (opts.upper, UPPER),
        (opts.digits, DIGITS),
        (opts.symbols, SYMBOLS),
    ]
    .into_iter()
    .filter_map(|(on, set)| on.then_some(set))
    .collect();
    if classes.is_empty() {
        return Err(GenError::NoClass);
    }
    let union: Vec<u8> = classes.concat();
    let mut out = Zeroizing::new(Vec::with_capacity(opts.length));
    for set in &classes {
        out.push(*set.choose(&mut OsRng).expect("classes are non-empty"));
    }
    while out.len() < opts.length {
        out.push(*union.choose(&mut OsRng).expect("union is non-empty"));
    }
    out.shuffle(&mut OsRng);
    // All sets are ASCII.
    Ok(Zeroizing::new(out.iter().map(|&b| b as char).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(s: &str, set: &[u8]) -> bool {
        s.bytes().any(|b| set.contains(&b))
    }

    #[test]
    fn length_and_classes() {
        for len in [4, 5, 20, 128] {
            for _ in 0..50 {
                let pw = generate(&GeneratorOptions {
                    length: len,
                    ..Default::default()
                })
                .unwrap();
                assert_eq!(pw.len(), len);
                assert!(
                    has(&pw, LOWER) && has(&pw, UPPER) && has(&pw, DIGITS) && has(&pw, SYMBOLS)
                );
            }
        }
    }

    #[test]
    fn disabled_classes_absent() {
        let opts = GeneratorOptions {
            length: 40,
            lower: true,
            upper: false,
            digits: true,
            symbols: false,
        };
        for _ in 0..50 {
            let pw = generate(&opts).unwrap();
            assert!(has(&pw, LOWER) && has(&pw, DIGITS));
            assert!(!has(&pw, UPPER) && !has(&pw, SYMBOLS));
        }
    }

    #[test]
    fn bad_options_are_errors() {
        let none = GeneratorOptions {
            lower: false,
            upper: false,
            digits: false,
            symbols: false,
            ..Default::default()
        };
        assert_eq!(generate(&none).unwrap_err(), GenError::NoClass);
        for length in [0, 3, 129] {
            assert_eq!(
                generate(&GeneratorOptions {
                    length,
                    ..Default::default()
                })
                .unwrap_err(),
                GenError::BadLength
            );
        }
    }
}
