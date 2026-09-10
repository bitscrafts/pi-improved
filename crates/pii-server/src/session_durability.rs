//! Session durability (P4.2, spec 16): persistence, real token counts,
//! `session.list`, boot seeding and run reconciliation.

use pii_abi::PiiError;
use pii_proto::session::SessionStatus;

use crate::Engine;

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

    /// Persists a session to the durable store (spec 16 part2 R1).
    pub(crate) fn persist_session(&self, session_id: &str) {
        let s = self.sessions.lock().get(session_id).cloned();
        if let Some(s) = s {
            let bytes = serde_json::to_vec(&s).unwrap_or_default();
            let _ = self.store.put_session(session_id, &bytes);
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
                let _ = self.store.set_session_tokens(
                    session_id,
                    cur_in.saturating_add(din),
                    cur_out.saturating_add(dout),
                );
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
        let mut map = self.sessions.lock();
        if let Some(s) = map.get_mut(session_id) {
            s.messages.push(pii_proto::session::WireMessage {
                role: "assistant".to_owned(),
                content: text,
            });
        }
        drop(map);
        // P4.2 (spec 16): persist the transcript so a restart keeps it.
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
    pub fn seed_sessions_from_store(&self) {
        let Ok(ids) = self.store.all_sessions() else {
            return;
        };
        let mut map = self.sessions.lock();
        for id in ids {
            if let Ok(Some(bytes)) = self.store.get_session(&id)
                && let Ok(s) = serde_json::from_slice::<crate::Session>(&bytes)
            {
                map.insert(id, s);
            }
        }
    }

    /// Boot reconciliation (spec 16 part2 R4): runs left `Running` by a
    /// crash are marked `Failed` so `run.list --status running` stops
    /// accumulating permanent zombies. Appends a `Finished(Failure)`
    /// terminal event and repoints `run_by_status` to `Failed`. Never
    /// auto-resumes (G8).
    pub fn reconcile_boot(&self) {
        let Ok(running) = self.store.runs_by_status(pii_abi::RunStatus::Running) else {
            return;
        };
        for run in running {
            let Ok(Some(mut header)) = self.store.get_run(&run) else {
                continue;
            };
            header.status = pii_abi::RunStatus::Failed;
            let _ = self.store.append_event(
                &mut header,
                &pii_abi::RunEvent::Finished {
                    terminal: pii_abi::Terminal::Failure {
                        error: String::from("interrupted by daemon restart"),
                    },
                    output_hash: String::new(),
                },
            );
        }
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
