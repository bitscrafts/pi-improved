//! The observability seam of [`HttpFetchTool`]: one outcome event per
//! completed fetch and one denial event per allowlist refusal.

use std::sync::Arc;

use pii_abi::PiiError;
use pii_observe::{Event, Level, Observable, Observed, Observer, names};

use crate::HttpFetchTool;

impl Observable for HttpFetchTool {
    fn observer(&self) -> Option<&dyn Observer> {
        self.observer.as_deref()
    }
}

/// The host of an absolute http(s) URL: no scheme, userinfo, port, path or
/// query, so nothing secret-bearing is reported.
fn host_of(url: &str) -> &str {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
    host.split(':').next().unwrap_or("")
}

impl HttpFetchTool {
    /// Attaches the observer that fetch outcomes are reported to.
    #[must_use]
    pub fn with_observer(mut self, observer: Arc<dyn Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    /// Reports one finished fetch: [`names::FETCH_OUTCOME`] always, plus
    /// [`names::FETCH_ALLOWLIST_DENIED`] when the chokepoint denied the
    /// target. Only the host is reported — never the URL path or query.
    pub(crate) fn observe_outcome(&self, url: &str, outcome: &Result<(u16, usize), PiiError>) {
        let host = host_of(url);
        match outcome {
            Ok((status, bytes_in)) => self.emit(
                &Event::new("fetch", Level::Info, names::FETCH_OUTCOME)
                    .field("host", host)
                    .field("status", *status)
                    .field("bytes_in", *bytes_in as u64),
            ),
            Err(e) => {
                let denied = matches!(e, PiiError::Denied(_));
                self.emit(
                    &Event::new("fetch", Level::Warn, names::FETCH_OUTCOME)
                        .field("host", host)
                        .field("error", if denied { "denied" } else { "failed" }),
                );
                if denied {
                    self.emit(
                        &Event::new("fetch", Level::Warn, names::FETCH_ALLOWLIST_DENIED)
                            .field("host", host),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::host_of;

    #[test]
    fn host_strips_everything_but_the_host() {
        assert_eq!(
            host_of("https://u:p@a.example:8443/x?token=s#f"),
            "a.example"
        );
        assert_eq!(host_of("http://b.example"), "b.example");
    }
}
