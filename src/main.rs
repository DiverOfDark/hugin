mod cluster;
mod config;
mod discovery;
mod format;
mod http;
mod metrics;
mod model;
mod poller;
mod render;
mod secrets;
mod server;
mod state;
mod weather;
mod widgets;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

/// Small server-rendered homelab dashboard.
#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Address the dashboard listens on.
    #[arg(long, env = "HUGIN_BIND", default_value = "0.0.0.0:8080")]
    bind: SocketAddr,

    /// Address for /healthz and /metrics.
    #[arg(long, env = "HUGIN_OPS_BIND", default_value = "0.0.0.0:8081")]
    ops_bind: SocketAddr,

    /// JSON configuration file.
    #[arg(long, env = "HUGIN_CONFIG", default_value = "/config/config.json")]
    config: PathBuf,

    /// Run without a Kubernetes cluster: only static services, no discovery,
    /// status dots, resource usage or Secret references.
    #[arg(long, env = "HUGIN_NO_KUBE")]
    no_kube: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("hugin=info,warn")),
        )
        .init();
    let cli = Cli::parse();
    // One thread is plenty for a page of a few dozen cards, and keeps the
    // allocator from spreading across per-thread arenas.
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(4)
        .build()?
        .block_on(run(cli))
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let cfg = config::Config::load(&cli.config)?;
    let request_timeout = Duration::from_secs(cfg.intervals.request_timeout_seconds.max(1));
    let state = Arc::new(AppState::new(cfg));
    let http: Arc<dyn widgets::Http> = Arc::new(http::Client::new(request_timeout)?);

    let client = if cli.no_kube {
        None
    } else {
        Some(kube::Client::try_default().await?)
    };
    let secrets = Arc::new(secrets::SecretReader::new(client.clone()));

    if let Some(client) = client {
        if state.cfg.discovery.ingress {
            tokio::spawn(cluster::watch_ingresses(state.clone(), client.clone()));
        }
        if state.cfg.discovery.traefik_ingress_routes {
            tokio::spawn(cluster::watch_ingressroutes(state.clone(), client.clone()));
        }
        tokio::spawn(cluster::watch_pods(state.clone(), client.clone()));
        tokio::spawn(poller::resources_loop(state.clone(), client));
    }
    tokio::spawn(poller::widgets_loop(state.clone(), http.clone(), secrets));
    tokio::spawn(poller::weather_loop(state.clone(), http));
    tokio::spawn(poller::render_loop(state.clone()));

    let public = tokio::net::TcpListener::bind(cli.bind).await?;
    let ops = tokio::net::TcpListener::bind(cli.ops_bind).await?;
    tracing::info!(
        addr = %cli.bind,
        ops = %cli.ops_bind,
        widgets = ?state.registry.kinds(),
        static_services = state.static_services.len(),
        "hugin listening"
    );

    let public_app = server::public_router(state.clone()).fallback(server::not_found);
    let ops_app = server::ops_router(state).fallback(server::not_found);
    tokio::select! {
        result = axum::serve(public, public_app) => result?,
        result = axum::serve(ops, ops_app) => result?,
        _ = shutdown_signal() => tracing::info!("shutting down"),
    }
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
