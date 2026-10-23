//! Session durability (P4.2, spec 16): persistence, real token counts,
//! `session.list`, boot seeding and run reconciliation.
//!
//! D3/D6 (026 report 5.0.4): before this pass, [`Engine::persist_session`]
//! re-serialised the ENTIRE `Session` (including its monotonically growing
//! `messages` transcript) and rewrote it whole on every call — one call per
//! turn, so cumulative bytes written over a session's lifetime were O(n^2)
//! in message count. The fix: [`Session`][crate::Session] meta
//! (`cwd`/`tools`/`graph`/`status`/`run_id`) still goes through
//! [`pii_store::Store::put_session`] (small, constant-size, unaffected by
//! transcript length), but transcript messages append one row each via
//! [`pii_store::Store::append_session_message`] (keyed `(session_id, seq)`,
//! mirroring `pii-graph`'s run journal pattern in `journal.rs`) instead of
//! being embedded in the meta blob. A full-session snapshot is taken every
//! [`SESSION_SNAPSHOT_EVERY`] appended messages so [`Engine::rebuild_session`]
//! never has to replay a session's entire history to reconstruct it — only
//! the tail since the latest snapshot.
//!
//! A persist failure (any `Result::Err` from the store on this path) is
//! surfaced through [`pii_core::Observer`] as
//! [`pii_core::ObsEvent::PersistError`] rather than silently discarded —
//! callers on this path are all best-effort by design (a session write must
//! not abort a run), but "best-effort" must not mean "unobservable".

use pii_abi::PiiError;
use pii_core::Ctx;
use pii_proto::session::{SessionStatus, WireMessage};

use crate::Engine;
use crate::node_kinds::nodes::coder_runtime::ContainerRuntime;

use super::orphans::{self, ProcProbe, Reaped};

/// A full-session snapshot is taken every time
/// [`Engine::append_session_message`] lands on a message seq that is a
/// nonzero multiple of this (mirrors `pii-graph`'s `SNAPSHOT_EVERY`).
pub const SESSION_SNAPSHOT_EVERY: u64 = 32;

impl Engine {
    /// Session snapshot (F1.5 `session.state`).
    #[must_use]
    pub fn session_state(
        &self,
        session_id: &str,
    ) -> Option<pii_proto::session::SessionStateResult> {
        let map = self.sessions.lock();
        let s = map.get(session_id)?;
        // R2 (spec 16 part2): real token counts from the durable store, not
        // the hardcoded (0,0) — `session.state` must not lie.
        let (tokens_in, tokens_out) = self
            .store
            .get_session_tokens(session_id)
            .unwrap_or(None)
            .unwrap_or((0, 0));
        let result = pii_proto::session::SessionStateResult {
            session_id: s.id.clone(),
            cwd: s.cwd.clone(),
            status: s.status,
            messages: s.messages.clone(),
            tokens_in,
            tokens_out,
            run_id: s.run_id.clone(),
        };
        drop(map);
        Some(result)
    }

    /// Persists a session's META fields to the durable store (spec 16
    /// part2 R1; narrowed by D3/D6 to meta only — the transcript is
    /// durable via [`Engine::append_session_message`], appended
    /// incrementally as messages are added, never re-written here). A
    /// store failure is surfaced through the engine's [`pii_core::Observer`]
    /// as [`pii_core::ObsEvent::PersistError`] (D3/D6) instead of being
    /// silently discarded — this call remains best-effort (it never panics
    /// or returns a `Result`, so no caller's control flow changes), but the
    /// failure is now observable.
    pub(crate) fn persist_session(&self, session_id: &str) {
        let s = self.sessions.lock().get(session_id).cloned();
        if let Some(s) = s {
            let bytes = serde_json::to_vec(&SessionMeta::from(&s)).unwrap_or_default();
            if let Err(e) = self.store.put_session(session_id, &bytes) {
                self.report_persist_error(session_id, &e);
            }
        }
    }

    /// Appends one transcript message to a session, both in-RAM (the
    /// engine's live `Session.messages`, for the fast `session.state` path)
    /// and durably via [`pii_store::Store::append_session_message`] (D3/D6:
    /// one row per message, not a whole-session rewrite). Snapshots the
    /// full session every [`SESSION_SNAPSHOT_EVERY`] durable appends so a
    /// restart's rebuild cost stays bounded. A durable-write failure is
    /// surfaced through the observer (D3/D6) — the in-RAM message is kept
    /// either way, matching this module's existing best-effort persistence
    /// contract (a durability hiccup must not lose the live transcript the
    /// running session is using).
    pub(crate) fn append_message(&self, session_id: &str, msg: &WireMessage) {
        let mut map = self.sessions.lock();
        let Some(s) = map.get_mut(session_id) else {
            return;
        };
        s.messages.push(msg.clone());
        let snapshot_target = map.get(session_id).cloned();
        drop(map);

        let bytes = serde_json::to_vec(&msg).unwrap_or_default();
        match self.store.append_session_message(session_id, &bytes) {
            Ok(seq) => {
                if seq > 0
                    && seq.is_multiple_of(SESSION_SNAPSHOT_EVERY)
                    && let Some(full) = snapshot_target
                {
                    let snap_bytes = serde_json::to_vec(&full).unwrap_or_default();
                    if let Err(e) = self
                        .store
                        .put_session_snapshot(session_id, seq, &snap_bytes)
                    {
                        self.report_persist_error(session_id, &e);
                    }
                }
            }
            Err(e) => self.report_persist_error(session_id, &e),
        }
    }

    /// Emits [`pii_core::ObsEvent::PersistError`] on the engine's observer
    /// (D3/D6): the single choke point every session-durability failure in
    /// this module routes through.
    fn report_persist_error(&self, session_id: &str, err: &PiiError) {
        self.observer().event(&pii_core::ObsEvent::PersistError {
            context: format!("session:{session_id}"),
            message: err.to_string(),
        });
    }

    /// Trims `session_runs[session_id]` to `cfg.session_runs_cap` entries,
    /// evicting the OLDEST run ids first (LRU by insertion order — the
    /// vec is append-only, so the front is always the oldest). No-op when
    /// the session has no runs yet or is already within the cap. Config
    /// key: `[core].session_runs_cap` (D3/D6, 026 report 5.0.4).
    pub(crate) fn trim_session_runs(&self, session_id: &str) {
        let cap = self.cfg.session_runs_cap.max(1);
        let mut map = self.session_runs.lock();
        if let Some(runs) = map.get_mut(session_id)
            && runs.len() > cap
        {
            let excess = runs.len() - cap;
            runs.drain(0..excess);
        }
    }

    /// `session.list` (spec 16 part2 R3): every session, ordered by id.
    pub fn session_list(&self) -> Vec<pii_proto::session::SessionListEntry> {
        let map = self.sessions.lock();
        map.values()
            .map(|s| pii_proto::session::SessionListEntry {
                session_id: s.id.clone(),
                cwd: s.cwd.clone(),
                status: s.status,
                run_id: s.run_id.clone(),
            })
            .collect()
    }

    /// Session status transition after a run terminal.
    pub fn session_finished(&self, run_id: &str) {
        let mut map = self.sessions.lock();
        let sid = map
            .values()
            .find(|s| s.run_id.as_deref() == Some(run_id))
            .map(|s| s.id.clone());
        if let Some(sid) = sid {
            if let Some(s) = map.get_mut(&sid) {
                s.status = SessionStatus::Idle;
                s.run_id = None;
            }
            drop(map);
            // P4.2 (spec 16): persist the Idle transition.
            self.persist_session(&sid);
        }
    }

    /// Records the assistant reply from a run terminal into the session
    /// transcript (F20, opus review): `session.state` must be a full
    /// snapshot, not just the user's prompts. Also accumulates real token
    /// counts into the durable store (spec 16 part2 R2) so `session.state`
    /// reports real numbers, not the fake (0,0).
    pub fn record_assistant(&self, session_id: &str, terminal: &pii_abi::Terminal) {
        // Accumulate real token usage (R2): the agent surfaces per-turn
        // tokens_in/out in its ok-port output; add them to the session total.
        if let pii_abi::Terminal::Success { output } = terminal {
            let din = output
                .get("tokens_in")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let dout = output
                .get("tokens_out")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            if din > 0 || dout > 0 {
                let (cur_in, cur_out) = self
                    .store
                    .get_session_tokens(session_id)
                    .unwrap_or(None)
                    .unwrap_or((0, 0));
                if let Err(e) = self.store.set_session_tokens(
                    session_id,
                    cur_in.saturating_add(din),
                    cur_out.saturating_add(dout),
                ) {
                    self.report_persist_error(session_id, &e);
                }
            }
        }
        let text = match terminal {
            // 2026-09-05 (session memory): use the agent's actual answer
            // text when present, not the raw `{"text":...,"turns":...}`
            // JSON blob — history built from this must read like a real
            // transcript, or every future turn re-seeds the model with
            // its own tool-call bookkeeping instead of clean dialogue.
            pii_abi::Terminal::Success { output } => output
                .get("text")
                .and_then(serde_json::Value::as_str)
                .map_or_else(|| output.to_string(), str::to_owned),
            pii_abi::Terminal::Failure { error } => error.clone(),
            pii_abi::Terminal::Suspended { .. } => String::from("(suspended)"),
        };
        // D3/D6: appends the message both in-RAM and durably (one row, not
        // a whole-session rewrite) — replaces the old push-then-
        // `persist_session` pattern that re-serialised the whole transcript.
        self.append_message(
            session_id,
            &WireMessage {
                role: "assistant".to_owned(),
                content: text,
            },
        );
        // Meta (status/run_id) is unchanged by an assistant reply, but the
        // pre-fix code always persisted here too; keep that behaviour for
        // any caller relying on `persist_session`'s side effects (e.g. a
        // stale run_id staying in sync) — it's now cheap (meta-only).
        self.persist_session(session_id);
    }

    /// The run ids spawned by a session, oldest first (for session-scoped
    /// `event.subscribe`). Empty when the session is unknown or has no runs.
    #[must_use]
    pub fn session_run_ids(&self, session_id: &str) -> Vec<String> {
        self.session_runs
            .lock()
            .get(session_id)
            .cloned()
            .unwrap_or_default()
    }

    /// Seeds the in-memory session map from the durable store (spec 16
    /// part2 R1, "daemon restart is amnesia" fix). Best-effort — a store
    /// read failure leaves the map empty rather than failing daemon startup.
    /// D3/D6: rebuilds each session's transcript from its latest snapshot
    /// (if any) plus the message rows appended since, instead of a single
    /// whole-session blob — bounding restart cost.
    pub fn seed_sessions_from_store(&self) {
        let Ok(ids) = self.store.all_sessions() else {
            return;
        };
        let mut map = self.sessions.lock();
        for id in ids {
            if let Ok(Some(bytes)) = self.store.get_session(&id)
                && let Ok(meta) = serde_json::from_slice::<SessionMeta>(&bytes)
            {
                let messages = self.rebuild_transcript(&id);
                map.insert(id.clone(), meta.into_session(id, messages));
            }
        }
    }

    /// Rebuilds a session's transcript from the latest snapshot (if any)
    /// plus messages appended since (D3/D6): bounds a restart's replay cost
    /// to "one snapshot read + the tail" rather than the whole history.
    fn rebuild_transcript(&self, session_id: &str) -> Vec<WireMessage> {
        let (since_seq, mut messages) = match self.store.latest_session_snapshot(session_id) {
            Ok(Some((seq, bytes))) => {
                let msgs = serde_json::from_slice::<SessionMeta>(&bytes)
                    .map(|s| s.messages)
                    .unwrap_or_default();
                (seq, msgs)
            }
            _ => (0, Vec::new()),
        };
        let tail = if since_seq == 0 {
            self.store.session_messages(session_id).unwrap_or_default()
        } else {
            self.store
                .session_messages_since(session_id, since_seq)
                .unwrap_or_default()
        };
        for (_, bytes) in tail {
            if let Ok(msg) = serde_json::from_slice::<WireMessage>(&bytes) {
                messages.push(msg);
            }
        }
        messages
    }

    /// Boot reconciliation (spec 16 part2 R4): runs left `Running` by a
    /// crash are marked `Failed` so `run.list --status running` stops
    /// accumulating permanent zombies. Appends a `Finished(Failure)`
    /// terminal event and repoints `run_by_status` to `Failed`. Never
    /// auto-resumes (G8).
    ///
    /// Spec 154: every failure is reported as a
    /// [`pii_core::ObsEvent::PersistError`] and every reconciled run as one
    /// [`pii_core::ObsEvent::RunReconciled`].
    pub fn reconcile_boot(&self) {
        self.reconcile_with(&[]);
    }

    /// Boot recovery (spec 242): kills the agents the dead daemon started, journals each reap on its run, then
    /// reconciles the runs it left `Running`. The failure text says an agent was reaped when one was killed.
    pub fn recover_boot(&self, ctx: &Ctx, runtime: &dyn ContainerRuntime, proc: &dyn ProcProbe) {
        let reaped = orphans::reap_orphans(&self.store, ctx, runtime, proc);
        self.reconcile_with(&reaped);
    }

    fn reconcile_with(&self, reaped: &[Reaped]) {
        const REASON: &str = "interrupted by daemon restart";
        const REAPED_REASON: &str = "interrupted by daemon restart; agent reaped";
        let running = match self.store.runs_by_status(pii_abi::RunStatus::Running) {
            Ok(running) => running,
            Err(e) => {
                self.report_reconcile_error("list", &e);
                return;
            }
        };
        for run in running {
            let mut header = match self.store.get_run(&run) {
                Ok(Some(header)) => header,
                Ok(None) => continue,
                Err(e) => {
                    self.report_reconcile_error(run.as_str(), &e);
                    continue;
                }
            };
            let ours: Vec<&Reaped> = reaped.iter().filter(|r| r.run == run.as_str()).collect();
            let reason = if ours.iter().any(|r| r.result == "killed") {
                REAPED_REASON
            } else {
                REASON
            };
            let finished = pii_abi::RunEvent::Finished {
                terminal: pii_abi::Terminal::Failure {
                    error: String::from(reason),
                },
                output_hash: String::new(),
            };
            let journal: Vec<pii_abi::RunEvent> = ours.iter().map(|r| r.event()).collect();
            let mut events: Vec<&pii_abi::RunEvent> = journal.iter().collect();
            events.push(&finished);
            header.status = pii_abi::RunStatus::Failed;
            let written = self.store.append_events(&mut header, &events);
            match written {
                Ok(_) => self.observer().event(&pii_core::ObsEvent::RunReconciled {
                    run: run.as_str().to_owned(),
                    reason: String::from(reason),
                }),
                Err(e) => self.report_reconcile_error(run.as_str(), &e),
            }
        }
    }

    fn report_reconcile_error(&self, what: &str, err: &PiiError) {
        self.observer().event(&pii_core::ObsEvent::PersistError {
            context: format!("reconcile:{what}"),
            message: err.to_string(),
        });
    }

    /// `session.interrupt` (B5): cancels the session's active run, if any.
    /// Idempotent — a session with no active run is a no-op success.
    ///
    /// # Errors
    /// Never errors (a missing session is a no-op success).
    pub fn interrupt_session(&self, session_id: &str) -> Result<(), PiiError> {
        let run_id = self
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.run_id.clone());
        if let Some(run_id) = run_id {
            self.cancel(&run_id);
        }
        Ok(())
    }

    /// `session.close` (B5): marks the session `Closed` and cancels its
    /// active run, if any. Idempotent.
    ///
    /// # Errors
    /// [`PiiError::NotFound`] when the session does not exist.
    pub fn close_session(&self, session_id: &str) -> Result<(), PiiError> {
        let run_id = self
            .sessions
            .lock()
            .get(session_id)
            .and_then(|s| s.run_id.clone());
        if let Some(run_id) = run_id {
            self.cancel(&run_id);
        }
        let mut map = self.sessions.lock();
        let s = map
            .get_mut(session_id)
            .ok_or_else(|| PiiError::NotFound(format!("session '{session_id}'")))?;
        s.status = SessionStatus::Closed;
        s.run_id = None;
        drop(map);
        self.persist_session(session_id);
        Ok(())
    }

    /// `graph.apply` (B5): stores a graph body under its content hash and
    /// returns the hash key.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn apply_graph(&self, toml: &str) -> Result<String, PiiError> {
        self.store.put_graph(toml.as_bytes())
    }

    /// `graph.get` (B5): fetches a stored graph body by hash.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn get_graph(&self, hash: &str) -> Result<Option<Vec<u8>>, PiiError> {
        self.store.get_graph(hash)
    }

    /// `graph.list` (B5): lists every stored graph body key.
    ///
    /// # Errors
    /// [`PiiError::Internal`] on a redb failure.
    pub fn list_graphs(&self) -> Result<Vec<String>, PiiError> {
        self.store.all_graphs()
    }
}

/// The durable meta-only encoding of a [`Session`][crate::Session] (D3/D6):
/// everything except the transcript, which lives in the append-only
/// `SESSION_MSG` table instead of being embedded here. Kept as a distinct
/// type (rather than reusing `Session` with an empty `messages`) so the
/// wire format change is explicit and `serde` never accidentally
/// round-trips a stale embedded transcript.
///
/// `messages` is retained ONLY as the payload shape for
/// [`SESSION_SNAPSHOT`][pii_store] rows (a full-session snapshot legitimately
/// wants the transcript inline, unlike the meta blob) — meta writes via
/// [`Engine::persist_session`] always serialise it empty.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct SessionMeta {
    id: String,
    cwd: String,
    tools: Vec<String>,
    graph: Option<String>,
    status: SessionStatus,
    #[serde(default)]
    messages: Vec<WireMessage>,
    run_id: Option<String>,
}

impl From<&crate::Session> for SessionMeta {
    fn from(s: &crate::Session) -> Self {
        Self {
            id: s.id.clone(),
            cwd: s.cwd.clone(),
            tools: s.tools.clone(),
            graph: s.graph.clone(),
            status: s.status,
            messages: Vec::new(),
            run_id: s.run_id.clone(),
        }
    }
}

impl SessionMeta {
    /// Rehydrates a full [`Session`][crate::Session] from meta plus an
    /// externally-rebuilt transcript (D3/D6:
    /// [`Engine::rebuild_transcript`] supplies `messages` from the
    /// snapshot + append tail — never from this struct's own `messages`
    /// field, which meta writes always leave empty).
    fn into_session(self, id: String, messages: Vec<WireMessage>) -> crate::Session {
        crate::Session {
            id,
            cwd: self.cwd,
            tools: self.tools,
            graph: self.graph,
            status: self.status,
            messages,
            run_id: self.run_id,
        }
    }
}
