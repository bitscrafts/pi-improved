//! Spec 121: the HAL contract, proved on the SIMULATED backend only.
//!
//! Every result here is a simulation result. No test in this file touches a
//! GPIO line, an I2C adapter or an SPI bus; nothing on this host can.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;

use pii_abi::PiiError;
use pii_hal::{Direction, Hal, HalBackend, HalHub, HalScope, PinLevel, SimBackend};
use pii_observe::{Health, Recorder, State, names};

const LED: u32 = 17;
const OTHER: u32 = 18;

fn scope() -> HalScope {
    HalScope::empty()
        .with_pin(LED)
        .with_i2c(1, 0x48)
        .with_spi(0)
}

fn rig() -> (Hal, Arc<SimBackend>, Arc<Recorder>) {
    let sim = Arc::new(SimBackend::standard());
    let rec = Arc::new(Recorder::new());
    let hal = Hal::new(sim.clone(), scope(), Some(rec.clone()));
    (hal, sim, rec)
}

fn denied(r: Result<impl std::fmt::Debug, PiiError>) -> String {
    match r {
        Err(PiiError::Denied(m)) => m,
        other => panic!("expected Denied, got {other:?}"),
    }
}

#[test]
fn an_unclaimed_pin_is_refused_for_write_and_read() {
    let (hal, sim, _) = rig();
    let w = denied(hal.write_pin(LED, PinLevel::High));
    assert!(w.contains("not claimed"), "{w}");
    let r = denied(hal.read_pin(LED));
    assert!(r.contains("not claimed"), "{r}");
    // The refusal happened before the backend: the line never moved.
    assert_eq!(sim.level(LED), Some(PinLevel::Low));
}

#[test]
fn a_pin_outside_the_scope_is_refused_even_though_it_exists_on_the_device() {
    let (hal, sim, _) = rig();
    assert!(sim.has_pin(OTHER), "the device has the pin");
    let c = denied(hal.claim_pin(OTHER, Direction::Out));
    assert!(c.contains("outside the declared scope"), "{c}");
    let w = denied(hal.write_pin(OTHER, PinLevel::High));
    assert!(w.contains("outside the declared scope"), "{w}");
    assert!(!sim.is_claimed(OTHER), "the backend never saw the claim");
    assert_eq!(sim.level(OTHER), Some(PinLevel::Low));
}

#[test]
fn a_double_claim_is_refused_not_idempotent_and_release_frees_it() {
    let (hal, _, _) = rig();
    hal.claim_pin(LED, Direction::Out).unwrap();
    let m = denied(hal.claim_pin(LED, Direction::Out));
    assert!(m.contains("already claimed"), "{m}");
    hal.release_pin(LED).unwrap();
    hal.claim_pin(LED, Direction::Out).unwrap();
}

#[test]
fn releasing_a_pin_that_is_not_claimed_is_refused() {
    let (hal, _, _) = rig();
    let m = denied(hal.release_pin(LED));
    assert!(m.contains("not claimed"), "{m}");
}

#[test]
fn a_pin_the_device_lacks_is_not_found_not_denied() {
    let sim = Arc::new(SimBackend::standard());
    let hal = Hal::new(sim, HalScope::empty().with_pin(999), None);
    assert!(matches!(
        hal.claim_pin(999, Direction::In),
        Err(PiiError::NotFound(_))
    ));
}

#[test]
fn writing_a_pin_claimed_as_input_is_refused() {
    let (hal, sim, _) = rig();
    hal.claim_pin(LED, Direction::In).unwrap();
    let m = denied(hal.write_pin(LED, PinLevel::High));
    assert!(m.contains("claimed as input"), "{m}");
    assert_eq!(sim.level(LED), Some(PinLevel::Low));
}

#[test]
fn the_blink_writes_reads_back_and_emits_the_sequence() {
    let (hal, sim, rec) = rig();
    hal.claim_pin(LED, Direction::Out).unwrap();
    hal.write_pin(LED, PinLevel::High).unwrap();
    assert_eq!(sim.level(LED), Some(PinLevel::High), "simulated line state");
    assert_eq!(hal.read_pin(LED).unwrap(), PinLevel::High);
    hal.write_pin(LED, PinLevel::Low).unwrap();
    assert_eq!(hal.read_pin(LED).unwrap(), PinLevel::Low);
    hal.release_pin(LED).unwrap();

    let kinds: Vec<_> = rec.events().into_iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            names::HAL_CLAIM,
            names::HAL_WRITE,
            names::HAL_READ,
            names::HAL_WRITE,
            names::HAL_READ,
            names::HAL_RELEASE
        ]
    );
    let levels: Vec<_> = rec
        .events()
        .iter()
        .filter_map(|e| e.fields.get("level").cloned())
        .collect();
    assert_eq!(
        levels,
        ["high", "high", "low", "low"].map(serde_json::Value::from)
    );
    assert!(
        rec.events()
            .iter()
            .all(|e| e.fields.get("pin") == Some(&serde_json::json!(LED))),
        "every event carries the pin"
    );
}

#[test]
fn a_scripted_input_sequence_is_replayed_then_holds_its_last_value() {
    let (hal, sim, _) = rig();
    sim.script_input(LED, &[PinLevel::High, PinLevel::Low, PinLevel::High]);
    hal.claim_pin(LED, Direction::In).unwrap();
    let got: Vec<_> = (0..5).map(|_| hal.read_pin(LED).unwrap()).collect();
    assert_eq!(
        got,
        [
            PinLevel::High,
            PinLevel::Low,
            PinLevel::High,
            PinLevel::High,
            PinLevel::High
        ]
    );
}

#[test]
fn an_i2c_transaction_writes_a_register_and_reads_it_back() {
    let (hal, _, rec) = rig();
    hal.claim_i2c(1, 0x48).unwrap();
    // Register pointer 0x10, two data bytes; then read two bytes from 0x10.
    hal.i2c_transaction(1, 0x48, &[0x10, 0xAB, 0xCD], 0)
        .unwrap();
    let back = hal.i2c_transaction(1, 0x48, &[0x10], 2).unwrap();
    assert_eq!(back, [0xAB, 0xCD]);
    // The preloaded identity register of the fake device.
    let id = hal.i2c_transaction(1, 0x48, &[0x00], 1).unwrap();
    assert_eq!(id, [0x5A]);
    let t = rec.events_named(names::HAL_TRANSACTION);
    assert_eq!(t.len(), 3);
    assert_eq!(t[0].fields.get("bus"), Some(&serde_json::json!(1)));
    assert_eq!(t[0].fields.get("addr"), Some(&serde_json::json!(0x48)));
}

#[test]
fn i2c_refuses_unclaimed_and_out_of_scope_addresses() {
    let (hal, sim, _) = rig();
    let u = denied(hal.i2c_transaction(1, 0x48, &[0], 1));
    assert!(u.contains("not claimed"), "{u}");
    assert!(sim.has_i2c(1, 0x49), "the device exists");
    let o = denied(hal.claim_i2c(1, 0x49));
    assert!(o.contains("outside the declared scope"), "{o}");
}

#[test]
fn spi_is_claimed_scoped_and_loops_back() {
    let (hal, _, _) = rig();
    let u = denied(hal.spi_transfer(0, &[1, 2]));
    assert!(u.contains("not claimed"), "{u}");
    let o = denied(hal.claim_spi(1));
    assert!(o.contains("outside the declared scope"), "{o}");
    hal.claim_spi(0).unwrap();
    assert_eq!(hal.spi_transfer(0, &[1, 2, 3]).unwrap(), [1, 2, 3]);
}

#[test]
fn counters_count_operations_by_kind_and_refusals_actually_move() {
    let (hal, _, _) = rig();
    assert_eq!(hal.counts().refusals, 0, "starts at zero");
    let _ = hal.write_pin(LED, PinLevel::High); // refused: unclaimed
    let _ = hal.claim_pin(OTHER, Direction::Out); // refused: out of scope
    hal.claim_pin(LED, Direction::Out).unwrap();
    let _ = hal.claim_pin(LED, Direction::Out); // refused: double
    hal.write_pin(LED, PinLevel::High).unwrap();
    hal.read_pin(LED).unwrap();
    hal.release_pin(LED).unwrap();
    let c = hal.counts();
    assert_eq!(c.refusals, 3);
    assert_eq!((c.claim, c.write, c.read, c.release), (1, 1, 1, 1));
}

#[test]
fn every_refusal_emits_an_error_event_with_a_reason() {
    let (hal, _, rec) = rig();
    let _ = hal.write_pin(LED, PinLevel::High);
    let _ = hal.claim_pin(OTHER, Direction::Out);
    let errs = rec.events_named(names::HAL_ERROR);
    let reasons: Vec<_> = errs
        .iter()
        .map(|e| e.fields.get("reason").cloned())
        .collect();
    assert_eq!(
        reasons,
        [Some("unclaimed".into()), Some("out_of_scope".into())]
    );
    assert_eq!(errs[0].fields.get("pin"), Some(&serde_json::json!(LED)));
    assert_eq!(errs[1].fields.get("pin"), Some(&serde_json::json!(OTHER)));
}

#[test]
fn health_reports_the_claimed_pins_and_the_last_error() {
    let (hal, _, _) = rig();
    let h = hal.health();
    assert_eq!(h.name, "hal");
    assert_eq!(h.state, State::Ok);
    assert_eq!(h.identity.get("backend").map(String::as_str), Some("sim"));
    assert_eq!(h.identity.get("claimed_pins").map(String::as_str), Some(""));
    hal.claim_pin(LED, Direction::Out).unwrap();
    let _ = hal.claim_pin(OTHER, Direction::Out);
    let h = hal.health();
    assert_eq!(
        h.identity.get("claimed_pins").map(String::as_str),
        Some("17")
    );
    assert!(
        h.identity["last_error"].contains("outside the declared scope"),
        "{:?}",
        h.identity
    );
}

#[test]
fn the_hal_names_are_registered_in_all() {
    for n in [
        names::HAL_CLAIM,
        names::HAL_RELEASE,
        names::HAL_READ,
        names::HAL_WRITE,
        names::HAL_TRANSACTION,
        names::HAL_ERROR,
        names::METRIC_HAL_OPS_TOTAL,
        names::METRIC_HAL_REFUSALS_TOTAL,
    ] {
        assert!(names::ALL.contains(&n), "{n} missing from ALL");
    }
}

#[test]
fn a_scope_intersection_never_widens() {
    let a = HalScope::empty().with_pin(1).with_pin(2).with_i2c(1, 0x10);
    let b = HalScope::empty().with_pin(2).with_pin(3);
    let i = a.intersect(&b);
    assert!(i.permits_pin(2));
    assert!(!i.permits_pin(1) && !i.permits_pin(3));
    assert!(!i.permits_i2c(1, 0x10));
}

#[test]
fn off_unless_configured_an_empty_hub_refuses_loudly() {
    let hub = HalHub::default();
    assert!(hub.get().is_none());
    let m = denied(hub.require());
    assert!(m.contains("not configured"), "{m}");
    let (hal, _, _) = rig();
    hub.set(Arc::new(hal));
    assert!(hub.require().is_ok());
}

#[test]
fn scope_strings_parse_and_a_bad_one_is_refused() {
    assert_eq!(HalScope::parse_i2c("1:0x48").unwrap(), (1, 0x48));
    assert_eq!(HalScope::parse_i2c("2:72").unwrap(), (2, 72));
    assert!(HalScope::parse_i2c("nope").is_err());
    assert!(HalScope::parse_i2c("1:0x200").is_err());
}
