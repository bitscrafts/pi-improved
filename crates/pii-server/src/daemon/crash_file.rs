//! The panic hook's crash record: one JSON file per panic under the crashes directory (moved out of `mod.rs` verbatim
//! to keep that file under the line rule; spec 362 added the model-roles wiring there).

use std::path::Path;

use pii_core::Clock;

pub(super) fn write_crash_file(crashes_dir: &Path, info: &std::panic::PanicHookInfo<'_>) {
    let ts_ms = pii_core::SystemClock.now_ms();
    let thread = std::thread::current()
        .name()
        .unwrap_or("unnamed")
        .to_owned();
    let location = info.location().map_or_else(String::new, |l| {
        format!("{}:{}:{}", l.file(), l.line(), l.column())
    });
    let message = panic_message(info);
    let _ = std::fs::create_dir_all(crashes_dir);
    let path = crashes_dir.join(format!("{ts_ms}.json"));
    let payload = serde_json::json!({
        "ts_ms": ts_ms, "thread": thread, "location": location, "message": message,
        "run": Option::<String>::None,
    });
    let _ = std::fs::write(&path, serde_json::to_vec(&payload).unwrap_or_default());
}

fn panic_message(info: &std::panic::PanicHookInfo<'_>) -> String {
    info.payload().downcast_ref::<&str>().map_or_else(
        || {
            info.payload()
                .downcast_ref::<String>()
                .map_or_else(|| String::from("non-string panic payload"), String::clone)
        },
        |s| (*s).to_owned(),
    )
}
