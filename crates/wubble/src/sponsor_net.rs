//! SponsorBlock's lookup, on a thread of its own: one GET per video, carrying only the first four
//! hex digits of the video id's SHA-256 (`wubble_core::sponsor::hash_prefix`); the answer holds
//! every video under that prefix and the caller picks its own. No cookies (ureq keeps none), no
//! referrer, a fixed `User-Agent: wubble`, https only unless the endpoint is the loopback
//! address (`--smoke video`'s fake server). Nothing is fetched unless `sponsor.enabled` is on:
//! the host only calls [`SponsorNet::lookup`] then.

use std::sync::mpsc;
use std::time::Duration;

use wubble_core::sponsor::{self, Segment};
use wubble_core::tab::TabId;

/// An answer bigger than this is refused.
const MAX_BYTES: u64 = 1 << 20;
const TIMEOUT: Duration = Duration::from_secs(5);

struct Job {
    tab: TabId,
    video: String,
    endpoint: String,
}

/// The lookup thread. Dropping it ends the thread once its queue is empty.
pub struct SponsorNet {
    tx: mpsc::Sender<Job>,
}

impl SponsorNet {
    /// Starts the thread; each answer goes to `done(tab, video, segments)` on that thread.
    pub fn spawn(done: impl Fn(TabId, String, Vec<Segment>) + Send + 'static) -> Option<Self> {
        let (tx, rx) = mpsc::channel::<Job>();
        let started = std::thread::Builder::new()
            .name("wubble-sponsor".into())
            .spawn(move || {
                while let Ok(job) = rx.recv() {
                    let segments = match fetch(&job.endpoint, &job.video) {
                        Ok(body) => sponsor::parse(&body, &job.video),
                        Err(e) => {
                            eprintln!("wubble: sponsor segments: {e}");
                            vec![]
                        }
                    };
                    done(job.tab, job.video, segments);
                }
            });
        match started {
            Ok(_) => Some(Self { tx }),
            Err(e) => {
                eprintln!("wubble: cannot start the sponsor lookup: {e}");
                None
            }
        }
    }

    pub fn lookup(&self, tab: TabId, video: &str, endpoint: &str) {
        let _ = self.tx.send(Job {
            tab,
            video: video.to_owned(),
            endpoint: endpoint.to_owned(),
        });
    }
}

/// Whether `endpoint` is on this machine (only then may it be plain http).
pub fn is_loopback(endpoint: &str) -> bool {
    url::Url::parse(endpoint).is_ok_and(|u| {
        matches!(
            u.host(),
            Some(url::Host::Ipv4(ip)) if ip.is_loopback()
        ) || matches!(u.host(), Some(url::Host::Ipv6(ip)) if ip.is_loopback())
    })
}

fn fetch(endpoint: &str, video: &str) -> Result<String, String> {
    let url = sponsor::request_url(endpoint, &sponsor::hash_prefix(video));
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        .https_only(!is_loopback(endpoint))
        .http_status_as_error(false)
        .max_redirects(0)
        .user_agent("wubble")
        .build()
        .into();
    let mut response = agent.get(&url).call().map_err(|e| e.to_string())?;
    match response.status().as_u16() {
        200 => response
            .body_mut()
            .with_config()
            .limit(MAX_BYTES)
            .read_to_string()
            .map_err(|e| e.to_string()),
        // No segments for any video under the prefix.
        404 => Ok(String::new()),
        status => Err(format!("{endpoint} answered {status}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_loopback_endpoints_may_be_plain_http() {
        assert!(is_loopback("http://127.0.0.1:4567"));
        assert!(is_loopback("http://[::1]:80/"));
        assert!(!is_loopback("http://localhost.evil.test/"));
        assert!(!is_loopback(wubble_core::sponsor::DEFAULT_ENDPOINT));
        assert!(!is_loopback("not a url"));
    }
}
