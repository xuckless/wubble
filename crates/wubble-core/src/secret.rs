//! A typed or stored password while it crosses the browser: zeroized when
//! dropped and never printed. Best effort, as the vault's threat model says:
//! copies handed to other layers (JSON, CEF strings, the page) are out of
//! reach.

use std::fmt;

use zeroize::Zeroizing;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: impl Into<String>) -> Self {
        Self(Zeroizing::new(value.into()))
    }

    /// The plain text, for the one call that needs it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<&str> for Secret {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for Secret {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_value() {
        let s = Secret::from("hunter2");
        assert_eq!(format!("{s:?}"), "Secret(..)");
        assert_eq!(s.expose(), "hunter2");
        assert!(Secret::default().is_empty());
    }
}
