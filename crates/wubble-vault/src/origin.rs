//! Origins and the matching rule.
//!
//! Rule: a saved login matches a page only on exact scheme + host + port.
//! - Subdomains never match (`accounts.example.com` vs `example.com`, either way),
//!   no public-suffix matching: Firefox `LoginHelper.isOriginMatching` default
//!   (`acceptDifferentSubdomains = false`).
//! - An https login is never offered on http (Chromium and Firefox).
//! - An http login is not offered on https either in v1. Firefox allows that
//!   directional upgrade (`schemeUpgrades`); exact is the safe subset of both
//!   and the upgrade is a possible later opt-in.
//! - Only http and https origins exist; default ports are normalised away.

use std::fmt;
use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Scheme {
    Http,
    Https,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Origin {
    scheme: Scheme,
    host: String,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OriginError {
    #[error("not a valid URL")]
    Invalid,
    #[error("only http and https origins are supported")]
    Scheme,
}

impl Origin {
    pub fn parse(s: &str) -> Result<Origin, OriginError> {
        Origin::from_url(&Url::parse(s.trim()).map_err(|_| OriginError::Invalid)?)
    }

    pub fn from_url(url: &Url) -> Result<Origin, OriginError> {
        let scheme = match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err(OriginError::Scheme),
        };
        let host = url
            .host_str()
            .ok_or(OriginError::Invalid)?
            .to_ascii_lowercase();
        let port = url.port_or_known_default().ok_or(OriginError::Invalid)?;
        Ok(Origin { scheme, host, port })
    }

    pub fn scheme(&self) -> Scheme {
        self.scheme
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

impl fmt::Display for Origin {
    /// `scheme://host[:port]`, port only when not the scheme default. This is
    /// the canonical form stored in the `origin` column and used as AEAD data.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (name, default) = match self.scheme {
            Scheme::Http => ("http", 80),
            Scheme::Https => ("https", 443),
        };
        write!(f, "{name}://{}", self.host)?;
        if self.port != default {
            write!(f, ":{}", self.port)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_table() {
        let cases = [
            ("https://example.com", "https://example.com", true),
            (
                "https://example.com",
                "https://example.com:443/login?x=1",
                true,
            ),
            ("http://example.com", "http://example.com:80/", true),
            ("https://EXAMPLE.com", "https://example.com/a/b", true),
            ("https://example.com", "https://example.com:8443", false),
            ("https://example.com", "http://example.com", false),
            ("http://example.com", "https://example.com", false),
            ("https://example.com", "https://accounts.example.com", false),
            ("https://accounts.example.com", "https://example.com", false),
            ("https://example.com", "https://example.org", false),
        ];
        for (saved, page, want) in cases {
            let (s, p) = (Origin::parse(saved).unwrap(), Origin::parse(page).unwrap());
            assert_eq!(s == p, want, "{saved} vs {page}");
        }
    }

    #[test]
    fn rejects_other_schemes() {
        assert_eq!(
            Origin::parse("android://abc@com.x/"),
            Err(OriginError::Scheme)
        );
        assert_eq!(Origin::parse("ftp://example.com"), Err(OriginError::Scheme));
        assert_eq!(Origin::parse("nonsense"), Err(OriginError::Invalid));
    }

    #[test]
    fn display_is_canonical() {
        assert_eq!(
            Origin::parse("HTTPS://Example.com:443/x")
                .unwrap()
                .to_string(),
            "https://example.com"
        );
        assert_eq!(
            Origin::parse("http://example.com:8080")
                .unwrap()
                .to_string(),
            "http://example.com:8080"
        );
        assert_eq!(
            Origin::parse("https://example.com:8443")
                .unwrap()
                .to_string(),
            "https://example.com:8443"
        );
    }
}
