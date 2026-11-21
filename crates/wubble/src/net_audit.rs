//! The proof that a run sent nothing off the machine: Chromium's net log (`log-net-log`, passed
//! through `WUBBLE_PROBE_SWITCHES`), read after the quit.
//!
//! Every host Chromium's resolver was asked for, and every socket it connected, must be loopback.
//! Two things are named, not failed: a URL whose host `host-resolver-rules` maps to `~NOTFOUND`
//! (tried, refused at name resolution: nothing looked up, no socket), and the IPv6 reachability
//! probe (`HostResolverManager::StartGloballyReachableCheck`, `kIPv6ProbeAddress`), a UDP
//! `connect()` whose only effect is the kernel's route lookup: it must have no bytes-sent event.
//! A connect to port 53 is a lookup leaving for the system resolver (the stub on 127.0.0.53
//! counts as loopback by address, not by effect), so it fails like any other.
//! The dictionary download (`gvt1.com`, `.bdic`) is named on its own.

use std::collections::{BTreeSet, HashMap, HashSet};

use serde_json::Value;

/// Where the IPv6 reachability probe connects its UDP socket.
const PROBES: &[&str] = &["[2001:4860:4860::8888]:443"];

/// What an audited log held.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// URL requests, all hosts.
    pub urls: usize,
    /// Hosts asked for and refused at name resolution (never resolved, never connected).
    pub refused: BTreeSet<String>,
    /// Socket connects, loopback ones.
    pub connects: usize,
    /// The reachability probe's connects (no bytes sent).
    pub probes: usize,
}

impl Report {
    /// One line for a smoke's output: what was refused is named as refused, not as asked for.
    pub fn line(&self) -> String {
        format!(
            "net log: {} requests, {} loopback connects, no lookup or connect off loopback; \
             refused at name resolution (never resolved or connected): {:?}; reachability probes \
             (UDP connect, nothing sent): {}",
            self.urls, self.connects, self.refused, self.probes
        )
    }
}

/// The `Switch` value that makes a run write its net log to `path`.
pub fn probe_switch(path: &std::path::Path) -> String {
    format!("log-net-log={}", path.display())
}

fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or("");
    if host.starts_with('[') {
        return host
            .split_once(']')
            .map_or(host, |(h, _)| &host[..=h.len()]);
    }
    host.rsplit_once(':').map_or(host, |(h, _)| h)
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "127.0.0.1" | "[::1]" | "localhost") || host.ends_with(".localhost")
}

fn is_loopback_address(address: &str) -> bool {
    (address.starts_with("127.") || address.starts_with("[::1]:")) && !address.ends_with(":53")
}

fn parse(log: &str) -> Result<Value, String> {
    serde_json::from_str(log)
        .or_else(|_| {
            // A log cut short ends inside the events array.
            serde_json::from_str(&format!("{}]}}", log.trim_end().trim_end_matches(',')))
        })
        .map_err(|e| format!("the net log does not parse: {e}"))
}

/// Checks `log`; the first thing that left (or could have left) the machine is the error.
pub fn audit(log: &str) -> Result<Report, String> {
    let log = parse(log)?;
    let names: HashMap<u64, &str> = log["constants"]["logEventTypes"]
        .as_object()
        .ok_or("the net log has no event type table")?
        .iter()
        .filter_map(|(name, id)| Some((id.as_u64()?, name.as_str())))
        .collect();
    let events = log["events"]
        .as_array()
        .ok_or("the net log has no events")?;
    let mut report = Report::default();
    let mut probe_sources = HashSet::new();
    let mut sent = HashSet::new();
    for event in events {
        let name = names
            .get(&event["type"].as_u64().unwrap_or(u64::MAX))
            .copied()
            .unwrap_or("");
        let source = event["source"]["id"].as_u64().unwrap_or(0);
        let params = &event["params"];
        let phase = event["phase"].as_u64().unwrap_or(0);
        match name {
            "REQUEST_ALIVE" if phase == 1 => {
                let url = params["url"].as_str().unwrap_or("");
                report.urls += 1;
                if url.contains("gvt1.com") || url.contains(".bdic") {
                    return Err(format!("a dictionary download was asked for: {url}"));
                }
                if !is_loopback_host(host_of(url)) {
                    report.refused.insert(host_of(url).to_owned());
                }
            }
            "HOST_RESOLVER_MANAGER_REQUEST" if phase == 1 => {
                let host = host_of(params["host"].as_str().unwrap_or(""));
                if !is_loopback_host(host) && host != "~notfound" {
                    return Err(format!("a host was looked up: {host}"));
                }
            }
            "TCP_CONNECT_ATTEMPT" | "SOCKET_CONNECT" | "UDP_CONNECT"
                if params["address"].is_string() =>
            {
                let address = params["address"].as_str().unwrap_or("");
                if PROBES.contains(&address) && name != "TCP_CONNECT_ATTEMPT" {
                    probe_sources.insert(source);
                    report.probes += 1;
                } else if is_loopback_address(address) {
                    report.connects += 1;
                } else {
                    return Err(format!("{name} to {address}"));
                }
            }
            n if n.contains("BYTES_SENT") => {
                sent.insert(source);
            }
            _ => {}
        }
    }
    if let Some(source) = probe_sources.intersection(&sent).next() {
        return Err(format!(
            "the reachability probe (source {source}) sent bytes"
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A log of `events` (type name, source id, phase, params) with the type table they need.
    fn log(events: &[(&str, u64, u64, &str)]) -> String {
        let types = [
            "REQUEST_ALIVE",
            "HOST_RESOLVER_MANAGER_REQUEST",
            "TCP_CONNECT_ATTEMPT",
            "SOCKET_CONNECT",
            "UDP_CONNECT",
            "UDP_BYTES_SENT",
        ];
        let table: Vec<String> = types
            .iter()
            .enumerate()
            .map(|(i, t)| format!("\"{t}\":{i}"))
            .collect();
        let events: Vec<String> = events
            .iter()
            .map(|(t, s, p, params)| {
                let ty = types.iter().position(|x| x == t).unwrap();
                format!(
                    "{{\"type\":{ty},\"source\":{{\"id\":{s}}},\"phase\":{p},\"params\":{params}}}"
                )
            })
            .collect();
        format!(
            "{{\"constants\":{{\"logEventTypes\":{{{}}}}},\"events\":[{}]}}",
            table.join(","),
            events.join(",")
        )
    }

    #[test]
    fn loopback_and_refused_pass() {
        let l = log(&[
            ("REQUEST_ALIVE", 1, 1, r#"{"url":"http://127.0.0.1:9/p"}"#),
            (
                "REQUEST_ALIVE",
                2,
                1,
                r#"{"url":"https://update.googleapis.com/x"}"#,
            ),
            (
                "HOST_RESOLVER_MANAGER_REQUEST",
                2,
                1,
                r#"{"host":"https://~notfound"}"#,
            ),
            ("TCP_CONNECT_ATTEMPT", 3, 1, r#"{"address":"127.0.0.1:9"}"#),
            (
                "UDP_CONNECT",
                4,
                1,
                r#"{"address":"[2001:4860:4860::8888]:443"}"#,
            ),
        ]);
        let r = audit(&l).unwrap();
        assert_eq!((r.urls, r.connects, r.probes), (2, 1, 1));
        assert_eq!(
            r.refused.into_iter().collect::<Vec<_>>(),
            ["update.googleapis.com"]
        );
    }

    #[test]
    fn a_log_cut_short_still_reads() {
        let l = log(&[("REQUEST_ALIVE", 1, 1, r#"{"url":"http://localhost/"}"#)]);
        let cut = l.strip_suffix("]}").unwrap();
        assert_eq!(audit(cut).unwrap().urls, 1);
    }

    #[test]
    fn a_resolved_or_connected_stranger_fails() {
        let looked_up = log(&[(
            "HOST_RESOLVER_MANAGER_REQUEST",
            1,
            1,
            r#"{"host":"https://content-autofill.googleapis.com"}"#,
        )]);
        assert!(audit(&looked_up).unwrap_err().contains("looked up"));
        let tcp = log(&[(
            "TCP_CONNECT_ATTEMPT",
            1,
            1,
            r#"{"address":"[2001:4860:4847:400::]:443"}"#,
        )]);
        assert!(audit(&tcp).is_err());
        let dns = log(&[("UDP_CONNECT", 1, 1, r#"{"address":"127.0.0.53:53"}"#)]);
        assert!(audit(&dns).is_err());
        let tcp_to_probe = log(&[(
            "TCP_CONNECT_ATTEMPT",
            1,
            1,
            r#"{"address":"[2001:4860:4860::8888]:443"}"#,
        )]);
        assert!(audit(&tcp_to_probe).is_err());
    }

    #[test]
    fn a_probe_that_sends_fails() {
        let l = log(&[
            (
                "UDP_CONNECT",
                4,
                1,
                r#"{"address":"[2001:4860:4860::8888]:443"}"#,
            ),
            ("UDP_BYTES_SENT", 4, 0, "{}"),
        ]);
        assert!(audit(&l).unwrap_err().contains("sent bytes"));
    }

    #[test]
    fn the_dictionary_download_is_named() {
        let l = log(&[(
            "REQUEST_ALIVE",
            1,
            1,
            r#"{"url":"https://redirector.gvt1.com/x/en-US-10-2.bdic"}"#,
        )]);
        assert!(audit(&l).unwrap_err().contains("dictionary"));
    }

    #[test]
    fn hosts_are_read_out_of_urls() {
        assert_eq!(host_of("https://a.b:443/x?y"), "a.b");
        assert_eq!(host_of("http://[::1]:80/"), "[::1]");
        assert_eq!(host_of("http://127.0.0.1@evil.com/"), "evil.com");
        assert!(is_loopback_host("t1.localhost"));
        assert!(!is_loopback_host("127.0.0.1.evil.com"));
    }
}
