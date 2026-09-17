//! Channel router (F6.5): poll → start a run → advance the cursor →
//! (reply happens reactively in [`super::finish_run`] when the run
//! terminates).

use std::sync::Arc;
use std::time::Duration;

use pii_core::{Budget, CapKind, CapSet, Ctx, PathScope, Principal, TraceCtx};
use pii_proto::event::Event;

use crate::Engine;
use crate::daemon::status::DaemonContext;

/// Spawns one polling task per configured channel (F6.5). Each task runs
/// until aborted; a channel error is absorbed (F6.3, recorded on the
/// channel cursor) and never kills the daemon.
pub fn spawn_all(
    engine: &Arc<Engine>,
    ctx: &Arc<DaemonContext>,
) -> Vec<tokio::task::JoinHandle<()>> {
    ctx.channels
        .lock()
        .iter()
        .map(|(n, c)| (n.clone(), c.poll_interval_ms))
        .collect::<Vec<(String, u64)>>()
        .into_iter()
        .map(|(name, interval_ms)| {
            let engine = Arc::clone(engine);
            let ctx = Arc::clone(ctx);
            tokio::spawn(async move {
                loop {
                    poll_once(&engine, &ctx, &name);
                    tokio::time::sleep(Duration::from_millis(interval_ms.max(10))).await;
                }
            })
        })
        .collect()
}

fn channel_ctx(channels_scope: &std::path::Path, name: &str) -> Ctx {
    // The channel needs NET (Discord) AND FS (the cli channel's inbox/
    // outbox files are read/written through Ctx::fs). Capability denial is
    // enforced by Ctx::fs/ctx.net; the router grants both.
    //
    // Phase 5.0 D2: the fs scope is `<runtime_dir>/channels/<name>` —
    // derived from the daemon's resolved runtime dir (`channels_scope`,
    // set in `daemon/mod.rs::Daemon::build` from `resolve_runtime_dir`),
    // never a hardcoded literal.
    let scope = channels_scope.join(name);
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET | CapKind::FS_READ | CapKind::FS_WRITE;
    caps.fs_read = PathScope::new(vec![scope.clone()]);
    caps.fs_write = PathScope::new(vec![scope.clone()]);
    Ctx::with_observer(
        caps,
        Budget::unlimited(),
        Arc::new(pii_core::NullObserver),
        TraceCtx {
            run: pii_abi::RunId::from("channel"),
            span: 0,
        },
    )
    .child(
        Principal::Channel(name.to_owned()),
        &{
            let mut c = CapSet::empty();
            c.kinds = CapKind::NET | CapKind::FS_READ | CapKind::FS_WRITE;
            c.fs_read = PathScope::new(vec![scope.clone()]);
            c.fs_write = PathScope::new(vec![scope]);
            c
        },
        &pii_core::BudgetShare::Shared,
    )
}

fn poll_once(engine: &Arc<Engine>, ctx: &Arc<DaemonContext>, name: &str) {
    let Some((channel, graph)) = ctx
        .channels
        .lock()
        .get(name)
        .map(|c| (Arc::clone(&c.channel), c.graph.clone()))
    else {
        return;
    };
    let cursor = ctx.channel_store.get(name).ok().flatten().map(|c| c.cursor);
    let rctx = channel_ctx(&ctx.channels_scope, name);
    let Ok(incoming) = channel.poll(&rctx, cursor) else {
        let _ = ctx.channel_store.record_error(name);
        return;
    };
    for msg in incoming {
        let input = serde_json::json!({
            "text": msg.text, "chat": msg.chat, "channel": msg.channel, "user": msg.user,
        });
        let Ok(run_id) = engine.start_run(&graph, &input) else {
            continue;
        };
        let _ = ctx.channel_store.advance(name, msg.cursor, run_id.as_str());
        ctx.channel_runs
            .lock()
            .insert(run_id.to_string(), (name.to_owned(), msg.chat.clone()));
        engine.bus.broadcast(Event::ChannelMessage {
            channel: name.to_owned(),
            chat: msg.chat,
            run_id: run_id.to_string(),
        });
    }
}

/// Sends the reply for a finished channel run (F6.5): `Success` sends the
/// output text, `Failure` sends `"error: <message>"`. A send failure is
/// retried once, then dropped — never kills the daemon.
pub fn reply(ctx: &Arc<DaemonContext>, channel: &str, chat: &str, text: &str) {
    let Some(c) = ctx
        .channels
        .lock()
        .get(channel)
        .map(|c| Arc::clone(&c.channel))
    else {
        return;
    };
    let rctx = channel_ctx(&ctx.channels_scope, channel);
    if c.send(&rctx, chat, text).is_err() {
        let _ = c.send(&rctx, chat, text);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::channel_ctx;

    /// Phase 5.0 D2: the channel fs scope is `<channels_scope>/<name>`,
    /// derived from the daemon's runtime dir — never a hardcoded literal.
    /// A path inside that scope resolves `Allowed` (the write succeeds); a
    /// path outside it resolves `Denied`.
    #[test]
    fn channel_ctx_scope_denies_outside_allows_inside() {
        let tmp_root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../_tmp"));
        std::fs::create_dir_all(tmp_root).expect("mkdir _tmp root");
        let root = tempfile::Builder::new()
            .prefix("channel-scope-")
            .tempdir_in(tmp_root)
            .expect("tempdir");
        // Canonicalize: `channel_ctx`'s scope check compares against the
        // *canonical* path (symlinks/`..` resolved by `canonical_for`), and
        // `tmp_root` is built from `CARGO_MANIFEST_DIR/../../_tmp` which
        // still has `..` components at this point.
        let root_path = dunce::canonicalize(root.path()).expect("canonicalize tempdir");
        let channels_scope = root_path.join("channels");
        std::fs::create_dir_all(&channels_scope).expect("mkdir channels_scope");

        let rctx = channel_ctx(&channels_scope, "cli-test");

        // Inside: <channels_scope>/cli-test/inbox.jsonl -> Allowed.
        let inside = channels_scope.join("cli-test").join("inbox.jsonl");
        let write_result = rctx.fs().expect("fs cap").write(&inside, b"hello");
        assert!(
            write_result.is_ok(),
            "path inside the configured channel scope must be Allowed: {write_result:?}"
        );
        assert_eq!(std::fs::read(&inside).expect("read back"), b"hello");

        // Outside: a sibling directory not under <channels_scope>/cli-test -> Denied.
        let outside = root.path().join("other-channel").join("inbox.jsonl");
        let denied_result = rctx.fs().expect("fs cap").write(&outside, b"nope");
        assert!(
            matches!(denied_result, Err(pii_abi::PiiError::Denied(_))),
            "path outside the configured channel scope must be Denied: {denied_result:?}"
        );
    }
}
