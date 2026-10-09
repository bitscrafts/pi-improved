//! Spec 105 R1/R2: every redb the daemon opens carries a format version and
//! refuses a newer one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::Path;

use crate::approval::ApprovalStore;
use crate::channel::ChannelStore;
use crate::heal::HealStore;
use crate::timer::TimerWheel;
use crate::{FORMAT_VERSION, MemStore};

/// Overwrites the stamp in a closed store file (simulates a newer binary).
fn bump(path: &Path, v: u64) {
    let db = redb::Database::open(path).unwrap();
    let write = db.begin_write().unwrap();
    write
        .open_table(crate::tables::META)
        .unwrap()
        .insert(crate::identity::FORMAT_VERSION_KEY, v)
        .unwrap();
    write.commit().unwrap();
}

fn dir() -> tempfile::TempDir {
    crate::testing::tmp_scratch(env!("CARGO_MANIFEST_DIR"), "stamp-").unwrap()
}

macro_rules! stamp_case {
    ($name:ident, $ty:ty, $file:literal) => {
        #[test]
        fn $name() {
            let d = dir();
            let path = d.path().join($file);
            let s = <$ty>::open(&path).unwrap();
            assert_eq!(s.format_version().unwrap(), FORMAT_VERSION);
            drop(s);
            // control: un-bumped store reopens
            drop(<$ty>::open(&path).unwrap());
            bump(&path, FORMAT_VERSION + 1);
            let msg = <$ty>::open(&path).err().expect("must refuse").to_string();
            assert!(
                msg.contains($file)
                    && msg.contains(&format!("format version {}", FORMAT_VERSION + 1))
                    && msg.contains(&format!("version {FORMAT_VERSION} this binary")),
                "{msg}"
            );
        }
    };
}

stamp_case!(memory_store_stamps_and_refuses, MemStore, "memory.redb");
stamp_case!(
    approval_store_stamps_and_refuses,
    ApprovalStore,
    "approval.redb"
);
stamp_case!(heal_store_stamps_and_refuses, HealStore, "heal.redb");
stamp_case!(
    channel_store_stamps_and_refuses,
    ChannelStore,
    "channel.redb"
);
stamp_case!(timer_wheel_stamps_and_refuses, TimerWheel, "timer.redb");
