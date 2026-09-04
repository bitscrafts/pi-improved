//! Test fakes (F2.11, feature `testing`): served through `NetService::listen`
//! so no test ever needs a raw socket and `net-check` stays clean.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex as PLMutex;
use serde::Deserialize;

use pii_abi::{HttpRequest, HttpResponse, PiiError};
use pii_core::{Budget, CapKind, CapSet, Ctx};

use crate::listen::{HttpHandler, ListenSpec, ListenerId, Protocol};
use crate::service::{NetConfig, NetService};

/// One recorded request (F2.11).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedRequest {
    /// Method name.
    pub method: String,
    /// Path with query.
    pub path: String,
    /// Raw request body.
    pub body: Vec<u8>,
}

/// A route fixture entry.
#[derive(Debug, Clone, Deserialize)]
struct Route {
    method: String,
    path_prefix: String,
    status: u16,
    body: serde_json::Value,
    /// Optional delay before answering (ms), for deadline tests.
    #[serde(default)]
    delay_ms: u64,
    /// Optional use cap: once exhausted, this route is skipped and matching
    /// falls through to the next route (stateful sequencing, e.g. "one batch
    /// of messages, then empty batches" for a polling fixture).
    #[serde(default)]
    max_uses: Option<u32>,
}

#[derive(Deserialize)]
struct RoutesFixture {
    routes: Vec<Route>,
}

struct Router {
    routes: Vec<Route>,
    uses: Vec<std::sync::atomic::AtomicU32>,
    recorded: PLMutex<Vec<RecordedRequest>>,
}

impl Router {
    fn snapshot(&self) -> Vec<RecordedRequest> {
        self.recorded.lock().clone()
    }
}

impl HttpHandler for Router {
    fn handle(&self, req: &HttpRequest) -> HttpResponse {
        self.recorded.lock().push(RecordedRequest {
            method: req.method.to_string(),
            path: req.url.clone(),
            body: req.body.clone(),
        });
        let m = req.method.to_string();
        for (i, r) in self.routes.iter().enumerate() {
            if !(r.method.eq_ignore_ascii_case(&m) && req.url.starts_with(&r.path_prefix)) {
                continue;
            }
            if let Some(max) = r.max_uses {
                let used = self
                    .uses
                    .get(i)
                    .map_or(0, |c| c.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
                if used >= max {
                    continue; // exhausted: fall through to the next route
                }
            }
            if r.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(r.delay_ms));
            }
            return HttpResponse {
                status: r.status,
                headers: vec![(
                    String::from("Content-Type"),
                    String::from("application/json"),
                )],
                body: serde_json::to_vec(&r.body).unwrap_or_default(),
            };
        }
        HttpResponse {
            status: 404,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
}

/// The fake HTTP server (F2.11). Binds `127.0.0.1:0`, so tests never pick
/// ports; every dial goes through `NetService` with an open local grant.
pub struct FakeHttpServer {
    service: NetService,
    id: ListenerId,
    /// Loopback address the fake listens on.
    pub addr: std::net::SocketAddr,
    router: Arc<Router>,
    /// A client service pointed at the fake (open local grants).
    pub client: NetService,
}

impl FakeHttpServer {
    /// Starts a fake from a routes fixture JSON.
    ///
    /// # Errors
    /// [`PiiError::Invalid`] for bad JSON; bind failures are
    /// [`PiiError::Internal`].
    pub fn start(routes_json: &str) -> Result<Self, PiiError> {
        let fixture: RoutesFixture = serde_json::from_str(routes_json)
            .map_err(|e| PiiError::Invalid(format!("fake routes: {e}")))?;
        let uses = fixture
            .routes
            .iter()
            .map(|_| std::sync::atomic::AtomicU32::new(0))
            .collect();
        let router = Arc::new(Router {
            routes: fixture.routes,
            uses,
            recorded: PLMutex::new(Vec::new()),
        });
        let grants = Arc::new(crate::grant::StaticGrants::default());
        let service = NetService::new(NetConfig {
            resolver: Arc::new(crate::resolve::StaticResolver::new()),
            grants: Arc::clone(&grants) as Arc<dyn crate::grant::GrantStore>,
            clock: Arc::new(crate::clock::FixedClock::new()),
        });
        let id = service.listen(
            &fake_ctx(),
            ListenSpec::Tcp {
                addr: std::net::SocketAddr::from(([127, 0, 0, 1], 0)),
            },
            Protocol::Http,
            Arc::clone(&router) as Arc<dyn HttpHandler>,
        )?;
        let addr = service
            .listeners()
            .iter()
            .find(|l| l.id == id.0)
            .map(|l| l.addr)
            .ok_or_else(|| PiiError::Internal(String::from("fake: listener vanished")))?;
        // The client side: open local grant + a resolver pinning the fake.
        let mut resolver = crate::resolve::StaticResolver::new();
        resolver.insert("fake.local", addr.port(), vec![addr]);
        let mut grants = crate::grant::StaticGrants::default();
        grants.insert("*", "default", crate::grant::NetGrant::open_local());
        grants.insert("*", "http:fake.local", crate::grant::NetGrant::open_local());
        let client = NetService::new(NetConfig {
            resolver: Arc::new(resolver),
            grants: Arc::new(grants),
            clock: Arc::new(crate::clock::FixedClock::new()),
        });
        Ok(Self {
            service,
            id,
            addr,
            router,
            client,
        })
    }

    /// Every request the fake has received so far (F2.11).
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.router.snapshot()
    }

    /// Dumps recorded requests as ndjson (F2.11).
    ///
    /// # Errors
    /// I/O failure.
    pub fn dump(&self, path: &std::path::Path) -> Result<(), PiiError> {
        let mut lines = String::new();
        for r in self.router.snapshot() {
            let obj = serde_json::json!({
                "method": r.method,
                "path": r.path,
                "body": String::from_utf8_lossy(&r.body),
            });
            lines.push_str(&serde_json::to_string(&obj).unwrap_or_default());
            lines.push('\n');
        }
        std::fs::write(path, lines)
            .map_err(|e| PiiError::Internal(format!("dump {}: {e}", path.display())))
    }

    /// Stops the fake.
    pub fn stop(&self) {
        self.service.close(self.id);
    }
}

fn fake_ctx() -> Ctx {
    let mut caps = CapSet::empty();
    caps.kinds = CapKind::NET;
    Ctx::root(caps, Budget::unlimited())
}
