mod cluster;
mod config;
mod crd;
mod discovery;
mod format;
mod generator;
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
    #[command(subcommand)]
    command: Option<Command>,

    #[command(flatten)]
    serve: ServeArgs,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Serve the dashboard (the default).
    Serve(ServeArgs),
    /// Write Widget resources for services annotated `widget.type: auto`.
    Generate(GenerateArgs),
    /// Generate a widget script for one Ingress and print it, without
    /// storing anything. A starting point for a hand-written Widget.
    Draft(DraftArgs),
}

#[derive(clap::Args, Debug, Clone)]
struct ServeArgs {
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
    /// status dots, resource usage, Secret references or Widget resources.
    #[arg(long, env = "HUGIN_NO_KUBE")]
    no_kube: bool,
}

#[derive(clap::Args, Debug, Clone)]
struct AiArgs {
    /// Chat-completions endpoint of an OpenAI-compatible API.
    #[arg(
        long,
        env = "HUGIN_AI_URL",
        default_value = "https://api.openai.com/v1/chat/completions"
    )]
    ai_url: String,

    #[arg(long, env = "HUGIN_AI_MODEL")]
    ai_model: String,

    #[arg(long, env = "HUGIN_AI_KEY", hide_env_values = true)]
    ai_key: String,

    /// Passed through as `reasoning_effort` when set.
    #[arg(long, env = "HUGIN_AI_REASONING_EFFORT")]
    ai_reasoning_effort: Option<String>,

    /// Attempts the model gets per service.
    #[arg(long, env = "HUGIN_GENERATE_ROUNDS", default_value_t = 6)]
    rounds: u32,
}

#[derive(clap::Args, Debug, Clone)]
struct DraftArgs {
    /// The Ingress to write a widget for, as `namespace/name`.
    ingress: String,

    /// Use this base URL instead of the Ingress's `widget.url` annotation.
    #[arg(long)]
    url: Option<String>,

    /// What you would like the card to show.
    #[arg(long)]
    hint: Option<String>,

    #[command(flatten)]
    ai: AiArgs,
}

#[derive(clap::Args, Debug, Clone)]
struct GenerateArgs {
    /// Same configuration file as the dashboard; only `discovery` and
    /// `intervals.requestTimeoutSeconds` are used.
    #[arg(long, env = "HUGIN_CONFIG", default_value = "/config/config.json")]
    config: PathBuf,

    #[command(flatten)]
    ai: AiArgs,

    /// Namespace generated Widget resources are written to.
    #[arg(long, env = "POD_NAMESPACE", default_value = "hugin")]
    namespace: String,

    /// Seconds between scans for services without a widget.
    #[arg(long, env = "HUGIN_GENERATE_INTERVAL", default_value_t = 60)]
    interval: u64,

    /// Hours to wait before retrying a service whose generation failed.
    #[arg(long, env = "HUGIN_GENERATE_RETRY_HOURS", default_value_t = 6)]
    retry_hours: u64,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        // stdout is for output (`hugin draft` prints a script there).
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("hugin=info,warn")),
        )
        .init();
    let cli = Cli::parse();
    // One thread is plenty for a page of a few dozen cards, and keeps the
    // allocator from spreading across per-thread arenas.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(4)
        .build()?;
    match cli.command {
        Some(Command::Generate(args)) => runtime.block_on(generate(args)),
        Some(Command::Draft(args)) => runtime.block_on(draft(args)),
        Some(Command::Serve(args)) => runtime.block_on(run(args)),
        None => runtime.block_on(run(cli.serve)),
    }
}

async fn generate(args: GenerateArgs) -> anyhow::Result<()> {
    let cfg = config::Config::load(&args.config)?;
    let request_timeout = Duration::from_secs(cfg.intervals.request_timeout_seconds.max(1));
    let service_http: Arc<dyn widgets::Http> = Arc::new(http::Client::new(request_timeout)?);
    // Model calls are slow compared to a status API.
    let llm: Arc<dyn widgets::Http> = Arc::new(http::Client::new(Duration::from_secs(300))?);
    let client = kube::Client::try_default().await?;
    let settings = settings(
        args.ai,
        args.namespace,
        Duration::from_secs(args.interval.max(10)),
        Duration::from_secs(args.retry_hours * 3600),
        cfg.discovery.traefik_ingress_routes,
    );
    tokio::select! {
        result = generator::run(client, llm, service_http, settings) => result,
        _ = shutdown_signal() => Ok(()),
    }
}

fn settings(
    ai: AiArgs,
    namespace: String,
    interval: Duration,
    retry_after: Duration,
    traefik_ingress_routes: bool,
) -> generator::Settings {
    generator::Settings {
        api_url: ai.ai_url,
        model: ai.ai_model,
        api_key: ai.ai_key,
        reasoning_effort: ai.ai_reasoning_effort.filter(|e| !e.is_empty()),
        namespace,
        interval,
        max_rounds: ai.rounds.clamp(1, 12),
        retry_after,
        traefik_ingress_routes,
    }
}

async fn draft(args: DraftArgs) -> anyhow::Result<()> {
    let (namespace, name) = args
        .ingress
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("expected <namespace>/<ingress name>"))?;
    let service_http: Arc<dyn widgets::Http> =
        Arc::new(http::Client::new(Duration::from_secs(10))?);
    let llm: Arc<dyn widgets::Http> = Arc::new(http::Client::new(Duration::from_secs(300))?);
    let client = kube::Client::try_default().await?;
    let settings = settings(
        args.ai,
        String::new(),
        Duration::ZERO,
        Duration::ZERO,
        false,
    );
    let generated = generator::draft(
        client,
        llm,
        service_http,
        &settings,
        namespace,
        name,
        args.url,
        args.hint,
    )
    .await?;
    eprintln!(
        "# accepted after {} round(s); the card shows: {}",
        generated.rounds,
        generated
            .blocks
            .iter()
            .map(|(label, value)| format!("{label}={value}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("{}", generated.script);
    Ok(())
}

async fn run(cli: ServeArgs) -> anyhow::Result<()> {
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
        if state.cfg.discovery.widgets {
            tokio::spawn(cluster::watch_widgets(state.clone(), client.clone()));
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
