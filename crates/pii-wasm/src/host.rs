//! [`WasmHost`]: compiles a module from the scoped directory, instantiates it
//! with **no imports**, and calls one export under four runtime-enforced
//! limits. Every failure is a [`PiiError`]; nothing a guest controls can
//! panic the host.
//!
//! # ABI
//! The module exports `memory`, `alloc(len: i32) -> i32` and an entry
//! (default `run`) of type `(ptr: i32, len: i32) -> i64`. The host writes the
//! input JSON into memory it got from `alloc`, calls the entry, and reads the
//! output JSON from `(ret >> 32, ret & 0xffff_ffff)` as `(ptr, len)`.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use pii_abi::PiiError;
use pii_observe::{
    Event, Health, Level, Observable, Observed, Observer, State, SubsystemHealth, names,
};
use wasmtime::{Config, Engine, Instance, Memory, Module, Store, Trap};

use crate::WasmCounts;
use crate::config::{DEFAULT_ENTRY, MAX_MODULE_BYTES};
use crate::state::{Inner, Limits};
use crate::{CallOpts, Fuel, WasmConfig};

/// The wasmtime version this crate is built against (a test pins it to `Cargo.lock`).
pub const WASMTIME_VERSION: &str = "48.0.5";

/// Epoch tick: the wall-clock resolution of the deadline.
const TICK: Duration = Duration::from_millis(10);

/// Why a call failed after the module was accepted.
enum Failure {
    /// The runtime stopped it (trap, epoch, fuel, memory).
    Runtime(wasmtime::Error),
    /// The module broke the ABI (missing export, wild pointer).
    Abi(String),
    /// The module returned bytes that are not JSON.
    Output(String),
}

/// What identifies a call in events and errors.
struct Target {
    module: String,
    entry: String,
}

/// The plugin host.
pub struct WasmHost {
    cfg: WasmConfig,
    dir: PathBuf,
    engine: Engine,
    observer: Option<Arc<dyn Observer>>,
    inner: Mutex<Inner>,
    /// Dropping this stops the epoch ticker thread.
    _stop_ticker: mpsc::Sender<()>,
}

impl Observable for WasmHost {
    fn observer(&self) -> Option<&dyn Observer> {
        self.observer.as_deref()
    }
}

impl WasmHost {
    /// A host over `cfg`, reporting to `observer`.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] when `cfg.dir` is not a directory;
    /// [`PiiError::Internal`] when the engine or its epoch ticker cannot start.
    pub fn new(cfg: WasmConfig, observer: Option<Arc<dyn Observer>>) -> Result<Self, PiiError> {
        let dir = dunce::canonicalize(&cfg.dir)
            .map_err(|e| PiiError::Invalid(format!("wasm: dir {}: {e}", cfg.dir.display())))?;
        if !dir.is_dir() {
            return Err(PiiError::Invalid(format!(
                "wasm: dir {} is not a directory",
                dir.display()
            )));
        }
        let mut wc = Config::new();
        wc.epoch_interruption(true).consume_fuel(true);
        let engine =
            Engine::new(&wc).map_err(|e| PiiError::Internal(format!("wasm: engine setup: {e}")))?;
        let (stop, rx) = mpsc::channel::<()>();
        let ticking = engine.clone();
        std::thread::Builder::new()
            .name(String::from("pii-wasm-epoch"))
            .spawn(move || {
                while rx.recv_timeout(TICK) == Err(RecvTimeoutError::Timeout) {
                    ticking.increment_epoch();
                }
            })
            .map_err(|e| PiiError::Internal(format!("wasm: epoch ticker: {e}")))?;
        Ok(Self {
            cfg,
            dir,
            engine,
            observer,
            inner: Mutex::new(Inner::default()),
            _stop_ticker: stop,
        })
    }

    /// Counts so far.
    #[must_use]
    pub fn counts(&self) -> WasmCounts {
        self.inner.lock().counts
    }

    /// Runs `module` (a path inside the configured dir) with `input`, and
    /// returns the JSON value it produced.
    ///
    /// # Errors
    /// * [`PiiError::Denied`] — outside the dir, or the module imports anything;
    /// * [`PiiError::NotFound`] / [`PiiError::Invalid`] — unreadable, too large,
    ///   not a module, ABI broken, or the output is not valid JSON;
    /// * [`PiiError::Timeout`] — the wall-clock budget elapsed;
    /// * [`PiiError::BudgetExhausted`] — out of fuel, or past the memory cap;
    /// * [`PiiError::Internal`] — the module trapped (message carries the wasm backtrace).
    pub fn call(
        &self,
        module: &str,
        input: &serde_json::Value,
        opts: &CallOpts,
    ) -> Result<serde_json::Value, PiiError> {
        let t = Target {
            module: module.to_owned(),
            entry: opts
                .entry
                .clone()
                .unwrap_or_else(|| DEFAULT_ENTRY.to_owned()),
        };
        let path = self.resolve(&t)?;
        let t = Target {
            module: path.display().to_string(),
            ..t
        };
        let compiled = self.load(&path, &t)?;
        let budget = opts
            .timeout
            .map_or(self.cfg.timeout, |d| d.min(self.cfg.timeout));
        let fuel = match opts.fuel {
            Fuel::Off => None,
            Fuel::Default => Some(self.cfg.fuel),
            Fuel::Limit(n) => Some(n.min(self.cfg.fuel)),
        };
        let mut store = Store::new(
            &self.engine,
            Limits {
                max_memory: self.cfg.max_memory,
                denied: false,
            },
        );
        store.limiter(|l| l);
        store
            .set_fuel(fuel.unwrap_or(u64::MAX))
            .map_err(|e| PiiError::Internal(format!("wasm: set fuel: {e}")))?;
        let ticks = u64::try_from(budget.as_millis() / TICK.as_millis()).unwrap_or(u64::MAX);
        store.set_epoch_deadline(ticks.saturating_add(1));

        self.emit(
            &Self::tag(Event::new("wasm", Level::Info, names::WASM_INSTANTIATE), &t)
                .field("memory_limit", self.cfg.max_memory as u64)
                .field("timeout_ms", millis(budget))
                .field("fuel", fuel.unwrap_or(0)),
        );
        let started = Instant::now();
        let outcome = Self::exec(&compiled, &mut store, input, &t.entry);
        let elapsed = started.elapsed();
        match outcome {
            Ok((value, out_bytes)) => {
                self.inner.lock().counts.ok += 1;
                self.emit(
                    &Self::tag(Event::new("wasm", Level::Info, names::WASM_CALL), &t)
                        .field("elapsed_ms", millis(elapsed))
                        .field("out_bytes", out_bytes as u64),
                );
                Ok(value)
            }
            Err(f) => Err(self.fail(&t, f, store.data().denied, budget, elapsed, fuel)),
        }
    }

    /// Instantiates and calls; returns the value and its serialized size.
    fn exec(
        module: &Module,
        store: &mut Store<Limits>,
        input: &serde_json::Value,
        entry: &str,
    ) -> Result<(serde_json::Value, usize), Failure> {
        let instance = Instance::new(&mut *store, module, &[]).map_err(Failure::Runtime)?;
        let abi = |what: &str| Failure::Abi(format!("module does not export {what}"));
        let memory: Memory = instance
            .get_memory(&mut *store, "memory")
            .ok_or_else(|| abi("a `memory`"))?;
        let alloc = instance
            .get_typed_func::<i32, i32>(&mut *store, "alloc")
            .map_err(|_| abi("`alloc: (i32) -> i32`"))?;
        let run = instance
            .get_typed_func::<(i32, i32), i64>(&mut *store, entry)
            .map_err(|_| abi(&format!("an entry `{entry}: (i32, i32) -> i64`")))?;

        let bytes = serde_json::to_vec(input)
            .map_err(|e| Failure::Abi(format!("input is not serializable: {e}")))?;
        let len = i32::try_from(bytes.len())
            .map_err(|_| Failure::Abi(String::from("input is too large for the 32-bit ABI")))?;
        let ptr = alloc.call(&mut *store, len).map_err(Failure::Runtime)?;
        memory
            .write(
                &mut *store,
                usize::try_from(ptr.cast_unsigned()).unwrap_or(usize::MAX),
                &bytes,
            )
            .map_err(|_| {
                Failure::Abi(String::from("alloc returned a region outside its memory"))
            })?;
        let ret = run
            .call(&mut *store, (ptr, len))
            .map_err(Failure::Runtime)?
            .cast_unsigned();

        let wild = || {
            Failure::Abi(String::from(
                "the returned (ptr, len) lies outside its memory",
            ))
        };
        let out_ptr = usize::try_from(ret >> 32).map_err(|_| wild())?;
        let out_len = usize::try_from(ret & 0xffff_ffff).map_err(|_| wild())?;
        if out_ptr
            .checked_add(out_len)
            .is_none_or(|end| end > memory.data_size(&*store))
        {
            return Err(wild());
        }
        let mut out = vec![0_u8; out_len];
        memory
            .read(&*store, out_ptr, &mut out)
            .map_err(|_| wild())?;
        let value = serde_json::from_slice(&out)
            .map_err(|e| Failure::Output(format!("module returned invalid JSON: {e}")))?;
        Ok((value, out.len()))
    }

    /// The scoped path: inside the configured dir after symlinks and `..`.
    fn resolve(&self, t: &Target) -> Result<PathBuf, PiiError> {
        let given = Path::new(&t.module);
        let joined = if given.is_absolute() {
            given.to_path_buf()
        } else {
            self.dir.join(given)
        };
        let canon = dunce::canonicalize(&joined).map_err(|e| {
            self.refuse(
                t,
                "unreadable",
                PiiError::NotFound(format!("wasm: module {}: {e}", t.module)),
            )
        })?;
        if canon.starts_with(&self.dir) {
            return Ok(canon);
        }
        Err(self.refuse(
            t,
            "out_of_scope",
            PiiError::Denied(format!(
                "wasm: module {} is outside the configured dir {}",
                t.module,
                self.dir.display()
            )),
        ))
    }

    /// Reads (bounded) and compiles the module, and refuses any import.
    fn load(&self, path: &Path, t: &Target) -> Result<Module, PiiError> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)
            .and_then(|f| f.take(MAX_MODULE_BYTES + 1).read_to_end(&mut bytes))
            .map_err(|e| {
                self.refuse(
                    t,
                    "unreadable",
                    PiiError::NotFound(format!("wasm: read {}: {e}", t.module)),
                )
            })?;
        if bytes.len() as u64 > MAX_MODULE_BYTES {
            return Err(self.refuse(
                t,
                "too_large",
                PiiError::Invalid(format!(
                    "wasm: module {} exceeds {MAX_MODULE_BYTES} bytes",
                    t.module
                )),
            ));
        }
        let module = Module::new(&self.engine, &bytes).map_err(|e| {
            self.refuse(
                t,
                "bad_module",
                PiiError::Invalid(format!("wasm: {} is not a valid module: {e}", t.module)),
            )
        })?;
        self.emit(
            &Self::tag(Event::new("wasm", Level::Info, names::WASM_LOAD), t)
                .field("bytes", bytes.len() as u64),
        );
        if let Some(i) = module.imports().next() {
            return Err(self.refuse(
                t,
                "imports",
                PiiError::Denied(format!(
                    "wasm: module {} imports {}::{}, but the host provides no imports (no WASI, no filesystem, no env, no network)",
                    t.module,
                    i.module(),
                    i.name()
                )),
            ));
        }
        Ok(module)
    }

    fn tag(ev: Event, t: &Target) -> Event {
        ev.field("module", t.module.as_str())
            .field("entry", t.entry.as_str())
    }

    /// Records and emits a refusal; returns the error to hand back.
    fn refuse(&self, t: &Target, reason: &str, err: PiiError) -> PiiError {
        {
            let mut g = self.inner.lock();
            g.counts.refusals += 1;
            g.last_error = Some(err.to_string());
        }
        self.emit(
            &Self::tag(Event::new("wasm", Level::Warn, names::WASM_REFUSAL), t)
                .field("reason", reason)
                .field("error", err.to_string()),
        );
        err
    }

    /// Classifies a runtime failure, counts it, emits its named event.
    fn fail(
        &self,
        t: &Target,
        f: Failure,
        memory_denied: bool,
        budget: Duration,
        elapsed: Duration,
        fuel: Option<u64>,
    ) -> PiiError {
        let wasm_err = match f {
            Failure::Abi(m) => {
                return self.refuse(
                    t,
                    "bad_abi",
                    PiiError::Invalid(format!("wasm: {} {m}", t.module)),
                );
            }
            Failure::Output(m) => return self.invalid_output(t, &m),
            Failure::Runtime(e) => e,
        };
        let trap = wasm_err.downcast_ref::<Trap>().copied();
        let (err, kind) = if memory_denied {
            let msg = format!(
                "wasm: {} exceeded its memory limit of {} bytes",
                t.module, self.cfg.max_memory
            );
            (PiiError::BudgetExhausted(msg), "memory")
        } else if trap == Some(Trap::Interrupt) {
            (
                PiiError::Timeout {
                    after_ms: millis(budget),
                },
                "timeout",
            )
        } else if trap == Some(Trap::OutOfFuel) {
            let msg = format!(
                "wasm: {} ran out of fuel ({} units)",
                t.module,
                fuel.unwrap_or(0)
            );
            (PiiError::BudgetExhausted(msg), "fuel")
        } else {
            let what = trap.map_or_else(|| String::from("error"), |k| k.to_string());
            let msg = format!(
                "wasm: {} entry {} trapped: {what}\n{wasm_err:?}",
                t.module, t.entry
            );
            (PiiError::Internal(msg), "trap")
        };
        {
            let mut g = self.inner.lock();
            match kind {
                "memory" => g.counts.memory += 1,
                "timeout" => g.counts.timeout += 1,
                "fuel" => g.counts.fuel += 1,
                _ => g.counts.trap += 1,
            }
        }
        self.note(&err);
        let name = match kind {
            "memory" => names::WASM_MEMORY_LIMIT,
            "timeout" => names::WASM_TIMEOUT,
            "fuel" => names::WASM_FUEL_EXHAUSTED,
            _ => names::WASM_TRAP,
        };
        let mut ev = Self::tag(Event::new("wasm", Level::Warn, name), t);
        ev = match kind {
            "memory" => ev.field("memory_limit", self.cfg.max_memory as u64),
            "timeout" => ev
                .field("elapsed_ms", millis(elapsed))
                .field("budget_ms", millis(budget)),
            "fuel" => ev.field("fuel", fuel.unwrap_or(0)),
            _ => ev
                .field(
                    "trap",
                    trap.map_or_else(|| String::from("error"), |k| k.to_string()),
                )
                .field("backtrace", format!("{wasm_err:?}")),
        };
        self.emit(&ev);
        err
    }

    /// A module returned bytes that are not JSON: counted apart from a refusal.
    fn invalid_output(&self, t: &Target, m: &str) -> PiiError {
        self.inner.lock().counts.invalid += 1;
        let err = PiiError::Invalid(format!("wasm: {} {m}", t.module));
        self.note(&err);
        self.emit(
            &Self::tag(Event::new("wasm", Level::Warn, names::WASM_REFUSAL), t)
                .field("reason", "invalid_output")
                .field("error", err.to_string()),
        );
        err
    }

    fn note(&self, err: &PiiError) {
        self.inner.lock().last_error = Some(err.to_string());
    }
}

fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

impl Health for WasmHost {
    fn health(&self) -> SubsystemHealth {
        let (c, last_error) = {
            let g = self.inner.lock();
            (g.counts, g.last_error.clone())
        };
        let mut h = SubsystemHealth::ok("wasm");
        // A contained guest failure is the host doing its job, not a degraded host.
        h.state = State::Ok;
        let id = &mut h.identity;
        id.insert("runtime".into(), format!("wasmtime {WASMTIME_VERSION}"));
        id.insert("dir".into(), self.dir.display().to_string());
        id.insert("timeout_ms".into(), millis(self.cfg.timeout).to_string());
        id.insert("max_memory_bytes".into(), self.cfg.max_memory.to_string());
        id.insert("fuel_ceiling".into(), self.cfg.fuel.to_string());
        id.insert("calls_ok".into(), c.ok.to_string());
        id.insert(
            "calls_failed".into(),
            (c.trap + c.timeout + c.fuel + c.memory + c.invalid).to_string(),
        );
        id.insert("refusals".into(), c.refusals.to_string());
        if let Some(e) = &last_error {
            id.insert(
                "last_error".into(),
                e.lines().next().unwrap_or_default().to_owned(),
            );
        }
        h
    }
}
