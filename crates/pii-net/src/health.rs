//! [`Health`] for the egress service: which listeners are actually bound.

use pii_observe::{Health, SubsystemHealth};

use crate::service::NetService;

impl Health for NetService {
    fn health(&self) -> SubsystemHealth {
        let listeners = self.listeners();
        let mut h = SubsystemHealth::ok("net");
        h.identity
            .insert("listeners".into(), listeners.len().to_string());
        for l in listeners {
            h.identity.insert(
                format!("listener.{}", l.id),
                format!("{:?} {}", l.proto, l.addr),
            );
        }
        h
    }
}
