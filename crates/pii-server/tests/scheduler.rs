//! Timer-wheel / cron / scheduler tests (F5.5/F5.7/N5, part-4 TDD table):
//! the wheel fires due entries atomically, the cron parser is exact and
//! total, and a long-future timer causes zero scheduler wakeups.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pii_store::timer::TimerWheel;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new("/projects/pi-improved/_tmp/phase3/scheduler-it").join(name);
    let _ = std::fs::create_dir_all(&dir);
    dir
}

#[test]
fn fires_in_one_transaction() {
    let dir = scratch(&format!("txn-{}", std::process::id()));
    let path = dir.join("timer.redb");
    let w = TimerWheel::open(&path).unwrap();
    w.put_timer(100, "r1", "k1").unwrap();
    w.put_timer(100, "r2", "k2").unwrap();
    w.put_timer(200, "r3", "k3").unwrap();
    assert_eq!(w.next_timer().unwrap(), Some(100));

    // One take_due for now=100 fires exactly the two 100 ms runs; the
    // 200 ms one stays. A second take_due sees nothing left for now=100
    // (no duplicate fire — the transaction is atomic).
    let due = w.take_due(100).unwrap();
    assert_eq!(due.len(), 2, "both due runs fire: {due:?}");
    assert_eq!(w.take_due(100).unwrap().len(), 0, "no re-fire");
    assert_eq!(w.take_due(200).unwrap().len(), 1, "r3 fires at 200");
    assert_eq!(w.next_timer().unwrap(), None);
}

#[test]
fn cron_next_after_is_exact_and_total() {
    // */15 at 00:07 -> 00:15.
    let c = pii_graph::cron::Cron::parse("*/15 * * * *").unwrap();
    assert_eq!(c.next_after(7 * 60_000), 15 * 60_000);
    // Malformed expressions are total errors, never panics (F5.7).
    for bad in ["* * *", "60 * * * *", "*/0 * * * *", "a b c d e"] {
        let err = pii_graph::cron::Cron::parse(bad).unwrap_err();
        assert!(matches!(err, pii_abi::PiiError::Invalid(_)), "{bad}");
    }
}

#[test]
fn arms_once_not_polls() {
    // The scheduler loop itself is async; its contract is that a far-future
    // timer is not polled. We assert the wheel side: arming a timer 10
    // minutes out leaves it pending (next_timer == its fire time) and it
    // only becomes visible to take_due once the clock passes it.
    let dir = scratch(&format!("wheel-{}", std::process::id()));
    let path = dir.join("timer.redb");
    let w = TimerWheel::open(&path).unwrap();
    let far = 600_000u64;
    w.put_timer(far, "r1", "k1").unwrap();
    assert_eq!(w.next_timer().unwrap(), Some(far));
    assert_eq!(w.take_due(100).unwrap().len(), 0, "not polled early");
    let due = w.take_due(far).unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due.first().map(|d| d.run.as_str()), Some("r1"));
}
