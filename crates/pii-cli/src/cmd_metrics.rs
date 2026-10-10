//! `pii metrics` (spec 105 R4): the daemon's counters and gauges in the
//! Prometheus text exposition shape, one `name{labels} value` line each.
//! Every metric name is a `pii_observe::names` const. The values come from the
//! `sys.status` payload, including the daemon's bounded latency histograms
//! (spec 115) — their resolution is printed next to them. Output goes through `writeln!`, not the banned raw prints.

use std::fmt::Write as _;
use std::io::Write as _;
use std::process::ExitCode;

use pii_observe::names;
use pii_proto::method::Method;

use crate::cmd_ctl::run_call;

fn num(v: &serde_json::Value, ptr: &str) -> u64 {
    v.pointer(ptr)
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0)
}

/// Main-store bytes per run (0 with no runs): the growth rate as one number.
fn main_bytes_per_run(v: &serde_json::Value) -> u64 {
    let runs = num(v, "/counts/runs");
    let bytes = v
        .pointer("/stores")
        .and_then(|s| s.as_array())
        .and_then(|a| a.iter().find(|s| s["name"] == "main"))
        .map_or(0, |s| num(s, "/bytes"));
    bytes.checked_div(runs).unwrap_or(0)
}

/// Run-lifecycle latency lines (spec 115 R4) and the standing notes (R3c).
/// A quantile is emitted only for an outcome that has samples.
fn render_latency(v: &serde_json::Value, out: &mut String) {
    let _ = writeln!(
        out,
        "# {}: {}",
        names::METRIC_RUN_LATENCY_MS,
        pii_observe::LATENCY_RESOLUTION
    );
    for l in v
        .pointer("/latency")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
    {
        let o = l["outcome"].as_str().unwrap_or("?");
        let count = num(l, "/count");
        let _ = writeln!(
            out,
            "{}{{outcome=\"{o}\"}} {count}",
            names::METRIC_RUN_LATENCY_COUNT
        );
        if count == 0 {
            continue;
        }
        for (q, key) in [
            ("0.5", "/p50_ms"),
            ("0.95", "/p95_ms"),
            ("0.99", "/p99_ms"),
            ("1", "/max_ms"),
        ] {
            let _ = writeln!(
                out,
                "{}{{outcome=\"{o}\",quantile=\"{q}\"}} {}",
                names::METRIC_RUN_LATENCY_MS,
                num(l, key)
            );
        }
    }
    for n in v
        .pointer("/notes")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
        .filter_map(|n| n.as_str())
    {
        let _ = writeln!(out, "# NOTE {n}");
    }
}

/// Renders a `sys.status` payload as metric lines.
fn render(v: &serde_json::Value) -> String {
    let mut out = String::new();
    let main_sessions = v
        .pointer("/stores")
        .and_then(|s| s.as_array())
        .and_then(|a| a.iter().find(|s| s["name"] == "main"))
        .map_or(0, |s| num(s, "/entries/session"));
    for (name, value) in [
        (names::METRIC_RUNS, num(v, "/counts/runs")),
        (names::METRIC_SESSIONS, main_sessions),
        (
            names::METRIC_APPROVALS_PENDING,
            num(v, "/counts/approvals_pending"),
        ),
        (
            names::METRIC_JOURNAL_EVENTS_TOTAL,
            num(v, "/counts/journal_events"),
        ),
        (names::METRIC_MEMORY_DOCS, num(v, "/counts/memory_docs")),
        (names::METRIC_CRON_ENTRIES, num(v, "/counts/cron_entries")),
        (names::METRIC_UPTIME_MS, num(v, "/uptime_ms")),
        (names::METRIC_PROCESS_RSS_KB, num(v, "/rss_kb")),
        (names::METRIC_STORE_BYTES_PER_RUN, main_bytes_per_run(v)),
    ] {
        let _ = writeln!(out, "{name} {value}");
    }
    render_latency(v, &mut out);
    for st in v
        .pointer("/stores")
        .and_then(|s| s.as_array())
        .into_iter()
        .flatten()
    {
        let store = st["name"].as_str().unwrap_or("?");
        let _ = writeln!(
            out,
            "{}{{store=\"{store}\"}} {}",
            names::METRIC_STORE_BYTES,
            num(st, "/bytes")
        );
        let _ = writeln!(
            out,
            "{}{{store=\"{store}\"}} {}",
            names::METRIC_STORE_FORMAT_VERSION,
            num(st, "/format_version")
        );
    }
    out
}

/// `pii metrics`.
#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_metrics(args: &mut std::env::ArgsOs) -> ExitCode {
    if args.next().is_some() {
        let _ = writeln!(std::io::stderr(), "usage: pii metrics");
        return ExitCode::from(2);
    }
    match run_call(Method::SysStatus, serde_json::json!({})) {
        Ok(v) => {
            let _ = write!(std::io::stdout(), "{}", render(&v));
            ExitCode::SUCCESS
        }
        Err(e) => {
            let _ = writeln!(
                std::io::stderr(),
                "error: no daemon answered ({e}); is `pii daemon` running?"
            );
            ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::render;

    fn status() -> serde_json::Value {
        serde_json::json!({
            "uptime_ms": 42, "rss_kb": 86372,
            "counts": {"runs": 4, "journal_events": 32},
            "stores": [{"name": "main", "bytes": 10_000, "format_version": 1}],
            "latency": [
                {"outcome": "succeeded", "count": 3, "p50_ms": 65, "p95_ms": 132, "p99_ms": 173, "max_ms": 302},
                {"outcome": "refused", "count": 0, "p50_ms": 0, "p95_ms": 0, "p99_ms": 0, "max_ms": 0}
            ],
            "notes": ["the main store never shrinks on its own: ..."]
        })
    }

    /// Spec 115 R3/R4: the loaded-system numbers are in `pii metrics`.
    #[test]
    fn render_carries_rss_growth_latency_and_the_store_note() {
        let text = render(&status());
        for line in [
            "pii_process_rss_kb 86372",
            "pii_store_bytes_per_run 2500",
            "pii_run_latency_count{outcome=\"succeeded\"} 3",
            "pii_run_latency_ms{outcome=\"succeeded\",quantile=\"0.5\"} 65",
            "pii_run_latency_ms{outcome=\"succeeded\",quantile=\"0.95\"} 132",
            "pii_run_latency_ms{outcome=\"succeeded\",quantile=\"0.99\"} 173",
            "pii_run_latency_ms{outcome=\"succeeded\",quantile=\"1\"} 302",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line}\n{text}");
        }
        assert!(
            text.contains("# NOTE the main store never shrinks"),
            "{text}"
        );
        assert!(
            text.contains("# pii_run_latency_ms:"),
            "resolution stated: {text}"
        );
        // No samples, no invented quantiles.
        assert!(!text.contains("outcome=\"refused\",quantile"), "{text}");
        assert!(text.contains("pii_run_latency_count{outcome=\"refused\"} 0"));
    }

    #[test]
    fn render_emits_every_named_metric_with_its_value() {
        let v = serde_json::json!({
            "uptime_ms": 42,
            "counts": {"runs": 3, "journal_events": 9, "approvals_pending": 1,
                       "cron_entries": 2, "memory_docs": 5},
            "stores": [
                {"name": "main", "bytes": 8192, "format_version": 1, "entries": {"session": 4}},
                {"name": "heal", "bytes": 4096, "format_version": 1}
            ]
        });
        let text = render(&v);
        for line in [
            "pii_runs 3",
            "pii_sessions 4",
            "pii_approvals_pending 1",
            "pii_journal_events_total 9",
            "pii_memory_docs 5",
            "pii_cron_entries 2",
            "pii_uptime_ms 42",
            "pii_store_bytes{store=\"main\"} 8192",
            "pii_store_format_version{store=\"heal\"} 1",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line}\n{text}");
        }
    }
}
