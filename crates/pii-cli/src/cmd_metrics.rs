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

/// HAL operation counters (spec 121): one line per kind, then the refusals.
/// Emitted only when a `[hal]` backend is configured (the kinds map is then
/// non-empty), so a daemon with the HAL off prints no HAL lines at all.
fn render_hal(v: &serde_json::Value, out: &mut String) {
    let Some(ops) = v.pointer("/counts/hal_ops").and_then(|o| o.as_object()) else {
        return;
    };
    for (kind, n) in ops {
        let _ = writeln!(
            out,
            "{}{{kind=\"{kind}\"}} {}",
            names::METRIC_HAL_OPS_TOTAL,
            n.as_u64().unwrap_or(0)
        );
    }
    if !ops.is_empty() {
        let _ = writeln!(
            out,
            "{} {}",
            names::METRIC_HAL_REFUSALS_TOTAL,
            num(v, "/counts/hal_refusals")
        );
    }
}

/// WASM plugin call counters (spec 122): one line per outcome, then the
/// refusals. Emitted only when a `[wasm]` host is configured (the outcome map
/// is then non-empty), so a daemon with the host off prints no WASM lines.
fn render_wasm(v: &serde_json::Value, out: &mut String) {
    let Some(calls) = v.pointer("/counts/wasm_calls").and_then(|o| o.as_object()) else {
        return;
    };
    for (outcome, n) in calls {
        let _ = writeln!(
            out,
            "{}{{outcome=\"{outcome}\"}} {}",
            names::METRIC_WASM_CALLS_TOTAL,
            n.as_u64().unwrap_or(0)
        );
    }
    if !calls.is_empty() {
        let _ = writeln!(
            out,
            "{} {}",
            names::METRIC_WASM_REFUSALS_TOTAL,
            num(v, "/counts/wasm_refusals")
        );
    }
}

/// Verdicts of the last verification pass (spec 124); nothing before a pass.
fn render_verdicts(v: &serde_json::Value, out: &mut String) {
    let Some(vs) = v
        .pointer("/counts/learn_verdicts")
        .and_then(|o| o.as_object())
    else {
        return;
    };
    for (verdict, n) in vs {
        let _ = writeln!(
            out,
            "{}{{verdict=\"{verdict}\"}} {}",
            names::METRIC_LEARN_VERDICTS,
            n.as_u64().unwrap_or(0)
        );
    }
}

/// Promotions by state and refusals (spec 125); nothing until one exists.
fn render_promotions(v: &serde_json::Value, out: &mut String) {
    let Some(ps) = v
        .pointer("/counts/learn_promotions")
        .and_then(|o| o.as_object())
    else {
        return;
    };
    for (state, n) in ps {
        let _ = writeln!(
            out,
            "{}{{state=\"{state}\"}} {}",
            names::METRIC_LEARN_PROMOTIONS,
            n.as_u64().unwrap_or(0)
        );
    }
}

/// Candidate lessons by kind and the refusals (spec 123). Emitted only once a
/// `pii learn scan` has left a status (the kinds map is then non-empty).
fn render_learn(v: &serde_json::Value, out: &mut String) {
    render_verdicts(v, out);
    render_promotions(v, out);
    let Some(kinds) = v
        .pointer("/counts/learn_candidates")
        .and_then(|o| o.as_object())
    else {
        return;
    };
    for (kind, n) in kinds {
        let _ = writeln!(
            out,
            "{}{{kind=\"{kind}\"}} {}",
            names::METRIC_LEARN_CANDIDATES,
            n.as_u64().unwrap_or(0)
        );
    }
    if !kinds.is_empty() {
        let _ = writeln!(
            out,
            "{} {}",
            names::METRIC_LEARN_REFUSALS_TOTAL,
            num(v, "/counts/learn_refusals")
        );
    }
}

/// The memory graph's gauges and counters (spec 126). Emitted only once the graph has been
/// used (the writes map is then non-empty), so a fresh daemon prints no entity lines.
fn render_memory_graph(v: &serde_json::Value, out: &mut String) {
    let Some(writes) = v
        .pointer("/counts/memory_writes")
        .and_then(|o| o.as_object())
    else {
        return;
    };
    if writes.is_empty() {
        return;
    }
    let _ = writeln!(
        out,
        "{} {}",
        names::METRIC_MEMORY_ENTITIES,
        num(v, "/counts/memory_entities")
    );
    let _ = writeln!(
        out,
        "{} {}",
        names::METRIC_MEMORY_RELATIONSHIPS,
        num(v, "/counts/memory_relationships")
    );
    for (kind, n) in writes {
        let _ = writeln!(
            out,
            "{}{{kind=\"{kind}\"}} {}",
            names::METRIC_MEMORY_WRITES_TOTAL,
            n.as_u64().unwrap_or(0)
        );
    }
    for (ptr, label, metric) in [
        ("/counts/memory_tiers", "tier", names::METRIC_MEMORY_TIER),
        (
            "/counts/memory_tier_changes",
            "change",
            names::METRIC_MEMORY_TIER_CHANGES_TOTAL,
        ),
        (
            "/counts/memory_archive_ops",
            "op",
            names::METRIC_MEMORY_ARCHIVE_OPS_TOTAL,
        ),
    ] {
        for (key, n) in v
            .pointer(ptr)
            .and_then(|o| o.as_object())
            .into_iter()
            .flatten()
        {
            let _ = writeln!(
                out,
                "{metric}{{{label}=\"{key}\"}} {}",
                n.as_u64().unwrap_or(0)
            );
        }
    }
    for (ptr, metric) in [
        (
            "/counts/memory_write_refusals",
            names::METRIC_MEMORY_WRITE_REFUSALS_TOTAL,
        ),
        (
            "/counts/memory_query_refusals",
            names::METRIC_MEMORY_QUERY_REFUSALS_TOTAL,
        ),
    ] {
        for (reason, n) in v
            .pointer(ptr)
            .and_then(|o| o.as_object())
            .into_iter()
            .flatten()
        {
            let _ = writeln!(
                out,
                "{metric}{{reason=\"{reason}\"}} {}",
                n.as_u64().unwrap_or(0)
            );
        }
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
        (
            names::METRIC_MQTT_RECEIVED_TOTAL,
            num(v, "/counts/mqtt_received"),
        ),
        (
            names::METRIC_MQTT_DROPPED_TOTAL,
            num(v, "/counts/mqtt_dropped"),
        ),
        (
            names::METRIC_MQTT_PUBLISHED_TOTAL,
            num(v, "/counts/mqtt_published"),
        ),
        (
            names::METRIC_CHANNEL_POSTS_TOTAL,
            num(v, "/counts/channel_posts"),
        ),
        (
            names::METRIC_CHANNEL_POST_REFUSALS_TOTAL,
            num(v, "/counts/channel_post_refusals"),
        ),
        (
            names::METRIC_WORKER_REQUESTS_REFUSED_TOTAL,
            num(v, "/counts/worker_requests_refused"),
        ),
        (names::METRIC_PROCESS_RSS_KB, num(v, "/rss_kb")),
        (
            names::METRIC_RSS_BYTES_PER_RUN,
            num(v, "/rss_bytes_per_run"),
        ),
        (names::METRIC_STORE_BYTES_PER_RUN, main_bytes_per_run(v)),
    ] {
        let _ = writeln!(out, "{name} {value}");
    }
    render_latency(v, &mut out);
    render_hal(v, &mut out);
    render_wasm(v, &mut out);
    render_learn(v, &mut out);
    render_memory_graph(v, &mut out);
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
            "uptime_ms": 42, "rss_kb": 86372, "rss_bytes_per_run": 155,
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
            "pii_rss_bytes_per_run 155",
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

    /// Spec 121 R5: HAL counters appear by kind with the refusals, and not at
    /// all when the HAL is off.
    #[test]
    fn render_carries_hal_ops_by_kind_and_refusals_only_when_configured() {
        let on = render(&serde_json::json!({
            "counts": {"hal_ops": {"claim": 1, "read": 2, "write": 3}, "hal_refusals": 4}
        }));
        for line in [
            "pii_hal_ops_total{kind=\"claim\"} 1",
            "pii_hal_ops_total{kind=\"read\"} 2",
            "pii_hal_ops_total{kind=\"write\"} 3",
            "pii_hal_refusals_total 4",
        ] {
            assert!(on.lines().any(|l| l == line), "missing {line}\n{on}");
        }
        let off = render(&serde_json::json!({"counts": {"hal_ops": {}, "hal_refusals": 0}}));
        assert!(
            !off.contains("pii_hal"),
            "HAL off prints no HAL lines: {off}"
        );
    }

    /// Spec 122 R5: WASM counters appear by outcome with the refusals, and not
    /// at all when the host is off.
    #[test]
    fn render_carries_wasm_calls_by_outcome_and_refusals_only_when_configured() {
        let on = render(&serde_json::json!({
            "counts": {"wasm_calls": {"ok": 5, "timeout": 1, "trap": 2}, "wasm_refusals": 3}
        }));
        for line in [
            "pii_wasm_calls_total{outcome=\"ok\"} 5",
            "pii_wasm_calls_total{outcome=\"timeout\"} 1",
            "pii_wasm_calls_total{outcome=\"trap\"} 2",
            "pii_wasm_refusals_total 3",
        ] {
            assert!(on.lines().any(|l| l == line), "missing {line}\n{on}");
        }
        let off = render(&serde_json::json!({"counts": {"wasm_calls": {}, "wasm_refusals": 0}}));
        assert!(
            !off.contains("pii_wasm"),
            "host off prints no WASM lines: {off}"
        );
    }

    /// Spec 123 R7: candidate gauges by kind and the refusals appear once a
    /// scan has left a status, and not before.
    #[test]
    fn render_carries_learn_candidates_by_kind_only_after_a_scan() {
        let on = render(&serde_json::json!({
            "counts": {"learn_candidates": {"limit_hit": 2, "stuck_loop": 0}, "learn_refusals": 1}
        }));
        for line in [
            "pii_learn_candidates{kind=\"limit_hit\"} 2",
            "pii_learn_candidates{kind=\"stuck_loop\"} 0",
            "pii_learn_refusals_total 1",
        ] {
            assert!(on.lines().any(|l| l == line), "missing {line}\n{on}");
        }
        let off =
            render(&serde_json::json!({"counts": {"learn_candidates": {}, "learn_refusals": 0}}));
        assert!(
            !off.contains("pii_learn"),
            "no scan prints no learn lines: {off}"
        );
    }

    /// Spec 125 R7: promotion counters render in the same place, after a
    /// promotion exists and not before.
    #[test]
    fn render_carries_learn_promotions_only_once_one_exists() {
        let on = render(&serde_json::json!({
            "counts": {"learn_promotions": {"active": 1, "reverted": 0, "refused": 2}}
        }));
        for line in [
            "pii_learn_promotions{state=\"active\"} 1",
            "pii_learn_promotions{state=\"reverted\"} 0",
            "pii_learn_promotions{state=\"refused\"} 2",
        ] {
            assert!(on.lines().any(|l| l == line), "missing {line}\n{on}");
        }
        let off = render(&serde_json::json!({"counts": {"learn_promotions": {}}}));
        assert!(!off.contains("pii_learn_promotions"), "{off}");
    }

    /// Spec 124 R7: verdict gauges render in the same place as the extraction
    /// counters, after a pass and not before.
    #[test]
    fn render_carries_learn_verdicts_only_after_a_pass() {
        let on = render(&serde_json::json!({
            "counts": {"learn_verdicts": {"confirmed": 1, "refuted": 2, "inconclusive": 0}}
        }));
        for line in [
            "pii_learn_verdicts{verdict=\"confirmed\"} 1",
            "pii_learn_verdicts{verdict=\"refuted\"} 2",
            "pii_learn_verdicts{verdict=\"inconclusive\"} 0",
        ] {
            assert!(on.lines().any(|l| l == line), "missing {line}\n{on}");
        }
        let off = render(&serde_json::json!({"counts": {"learn_verdicts": {}}}));
        assert!(!off.contains("pii_learn_verdicts"), "{off}");
    }

    /// Spec 126 R8: the memory graph's counters render once it has been used, not before.
    #[test]
    fn render_carries_memory_graph_counters_only_once_used() {
        let on = render(&serde_json::json!({
            "counts": {"memory_entities": 2, "memory_relationships": 1,
                       "memory_writes": {"entity": 3, "relationship": 1},
                       "memory_write_refusals": {"no_provenance": 4},
                       "memory_query_refusals": {"result_cap": 1},
                       "memory_tiers": {"hot": 2, "cold": 1},
                       "memory_tier_changes": {"hot_to_cold": 1},
                       "memory_archive_ops": {"archived": 1}}
        }));
        for line in [
            "pii_memory_entities 2",
            "pii_memory_relationships 1",
            "pii_memory_writes_total{kind=\"entity\"} 3",
            "pii_memory_writes_total{kind=\"relationship\"} 1",
            "pii_memory_write_refusals_total{reason=\"no_provenance\"} 4",
            "pii_memory_query_refusals_total{reason=\"result_cap\"} 1",
            "pii_memory_tier{tier=\"hot\"} 2",
            "pii_memory_tier{tier=\"cold\"} 1",
            "pii_memory_tier_changes_total{change=\"hot_to_cold\"} 1",
            "pii_memory_archive_ops_total{op=\"archived\"} 1",
        ] {
            assert!(on.lines().any(|l| l == line), "missing {line}\n{on}");
        }
        let off = render(&serde_json::json!({"counts": {"memory_writes": {}}}));
        assert!(!off.contains("pii_memory_entities"), "{off}");
    }

    /// Spec 128 R6: the post counters are printed from `sys.status` counts.
    #[test]
    fn render_carries_the_channel_post_counters() {
        let text = render(&serde_json::json!({
            "counts": {"channel_posts": 3, "channel_post_refusals": 1}
        }));
        for line in [
            "pii_channel_posts_total 3",
            "pii_channel_post_refusals_total 1",
        ] {
            assert!(text.lines().any(|l| l == line), "missing {line}\n{text}");
        }
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
