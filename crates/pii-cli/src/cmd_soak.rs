//! `pii soak --connect <socket> --runs N --graph G [--concurrency C]` (F5.13).
//!
//! Drives N runs through the real control protocol over the real transport,
//! waits for each terminal, and prints `soak: runs=<N> ok=<n> fail=<n>
//! ms=<total>` — the load driver `xtask rss-gate` runs against a real daemon.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use pii_proto::client::Client;
use pii_proto::method::Method;
use pii_proto::sys::PeerInfo;

fn client_info() -> PeerInfo {
    PeerInfo {
        name: String::from("pii-soak"),
        version: env!("CARGO_PKG_VERSION").to_owned(),
        build: String::from("cli"),
    }
}

/// `pii soak ...`.
const USAGE: &str = "usage: pii soak --connect <socket> --runs N --graph G [--concurrency C]";

#[allow(clippy::redundant_pub_crate)] // bin crate: pub(crate) is the reachable form
pub(crate) fn cmd_soak(args: &mut std::env::ArgsOs) -> ExitCode {
    let mut connect: Option<PathBuf> = None;
    let mut runs = 0u32;
    let mut graph = String::new();
    let mut concurrency = 1u32;
    while let Some(a) = args.next().and_then(|a| a.into_string().ok()) {
        match a.as_str() {
            "--connect" => {
                connect = args
                    .next()
                    .and_then(|a| a.into_string().ok())
                    .map(PathBuf::from);
            }
            "--runs" => {
                runs = args
                    .next()
                    .and_then(|a| a.into_string().ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
            }
            "--graph" => {
                graph = args
                    .next()
                    .and_then(|a| a.into_string().ok())
                    .unwrap_or_default();
            }
            "--concurrency" => {
                concurrency = args
                    .next()
                    .and_then(|a| a.into_string().ok())
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);
            }
            _ => {}
        }
    }
    let Some(sock) = connect else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    if runs == 0 || graph.is_empty() {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("error: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    rt.block_on(soak_run(sock, runs, graph, concurrency))
}

async fn soak_run(sock: PathBuf, runs: u32, graph: String, concurrency: u32) -> ExitCode {
    let started = Instant::now();
    let sem = Arc::new(tokio::sync::Semaphore::new(
        usize::try_from(concurrency.max(1)).unwrap_or(1),
    ));
    let ok = Arc::new(AtomicU32::new(0));
    let fail = Arc::new(AtomicU32::new(0));
    let mut tasks = Vec::with_capacity(usize::try_from(runs).unwrap_or(0));
    for _ in 0..runs {
        let sock = sock.clone();
        let graph = graph.clone();
        let sem = Arc::clone(&sem);
        let ok = Arc::clone(&ok);
        let fail = Arc::clone(&fail);
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire_owned().await;
            if one_run(&sock, &graph).await {
                ok.fetch_add(1, Ordering::Relaxed);
            } else {
                fail.fetch_add(1, Ordering::Relaxed);
            }
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    let ms = started.elapsed().as_millis();
    let ok_n = ok.load(Ordering::Relaxed);
    let fail_n = fail.load(Ordering::Relaxed);
    println!("soak: runs={runs} ok={ok_n} fail={fail_n} ms={ms}");
    if fail_n == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

async fn one_run(sock: &Path, graph: &str) -> bool {
    let Ok(stream) = pii_uds::client_connect(sock).await else {
        return false;
    };
    let (r, w) = tokio::io::split(stream);
    let Ok((client, _hello)) = Client::connect(r, w, client_info()).await else {
        return false;
    };
    // B5: every path must release the connection — dropping a Client alone
    // leaves the split reader parked on the socket (server-side leak).
    let outcome = run_one(&client, graph).await;
    client.shutdown();
    outcome
}

/// Runs one graph and polls it to a terminal (shared by the soak driver).
async fn run_one(client: &Client, graph: &str) -> bool {
    let Ok(result) = client
        .call(
            Method::RunStart,
            serde_json::json!({"graph": graph, "input": {}}),
        )
        .await
    else {
        return false;
    };
    let Some(run_id) = result
        .get("run_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
    else {
        return false;
    };
    for _ in 0..600 {
        if let Ok(h) = client
            .call(Method::RunGet, serde_json::json!({"run_id": run_id}))
            .await
            && let Some(status) = h.get("status").and_then(|v| v.as_str())
        {
            match status {
                "succeeded" => return true,
                "failed" => return false,
                _ => {}
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}
