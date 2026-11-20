//! The updater's two requests, over the HTTP client the browser already has for its own
//! requests (ureq, as `suggest_fx.rs` and `sponsor_net.rs`): the manifest with its signature,
//! and the archive into a file. Plain GETs: no cookies (ureq keeps none), no referrer, the
//! fixed `User-Agent: wubble`, HTTPS only, the proxy from the environment as ureq reads it
//! (`HTTPS_PROXY`, `ALL_PROXY`, `NO_PROXY`; wubble has no proxy setting of its own). GitHub
//! answers a release download with a redirect to its storage host, which is followed.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use wubble_core::update::manifest::MAX_MANIFEST;

/// How the requests are made. Production: [`Net::PRODUCTION`]; only `--smoke update` makes
/// another, for its loopback server.
#[derive(Clone, Copy, Debug)]
pub struct Net {
    pub https_only: bool,
    pub env_proxy: bool,
}

impl Net {
    pub const PRODUCTION: Net = Net {
        https_only: true,
        env_proxy: true,
    };

    fn agent(self) -> ureq::Agent {
        let mut config = ureq::Agent::config_builder()
            .https_only(self.https_only)
            .user_agent("wubble")
            .timeout_connect(Some(Duration::from_secs(15)))
            .timeout_recv_response(Some(Duration::from_secs(30)))
            // a stalled download fails instead of hanging
            .timeout_per_read(Some(Duration::from_secs(60)));
        if !self.env_proxy {
            config = config.proxy(None);
        }
        config.build().into()
    }

    /// The manifest and its `.minisig` text.
    pub fn manifest(self, url: &str) -> Result<(Vec<u8>, String), String> {
        let agent = self.agent();
        let get = |url: &str| -> Result<Vec<u8>, String> {
            let mut response = agent.get(url).call().map_err(|e| e.to_string())?;
            response
                .body_mut()
                .with_config()
                .limit(MAX_MANIFEST)
                .read_to_vec()
                .map_err(|e| e.to_string())
        };
        let body = get(url)?;
        let sig = get(&format!("{url}.minisig"))?;
        Ok((
            body,
            String::from_utf8(sig).map_err(|_| "the signature is not text".to_owned())?,
        ))
    }

    /// The archive into `to`, at most `size` bytes; `progress` hears the count now and then.
    pub fn download(
        self,
        url: &str,
        size: u64,
        to: &Path,
        progress: impl Fn(u64),
    ) -> Result<(), String> {
        let mut response = self.agent().get(url).call().map_err(|e| e.to_string())?;
        let mut body = response.body_mut().as_reader();
        let mut file = std::io::BufWriter::new(
            std::fs::File::create(to).map_err(|e| format!("{}: {e}", to.display()))?,
        );
        let (mut received, mut told) = (0u64, 0u64);
        let mut buf = vec![0; 1 << 16];
        loop {
            let n = body.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            received += n as u64;
            if received > size {
                return Err("the download is larger than the release's".into());
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            // about every 1 %
            if received - told >= (size / 100).max(1 << 20) {
                told = received;
                progress(received);
            }
        }
        file.flush().map_err(|e| e.to_string())?;
        progress(received);
        Ok(())
    }
}
