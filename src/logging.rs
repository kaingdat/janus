use std::sync::{Arc, Mutex};

use anyhow::Context;
use async_trait::async_trait;
use pingora::server::{Server, ShutdownWatch};
use pingora::services::background::{BackgroundService, background_service};
use tracing::{error, info};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, Registry, reload};

use crate::config::main_config::MainConfig;

type BoxedLayer = Box<dyn Layer<Registry> + Send + Sync>;

fn parse_level(level: &str) -> LevelFilter {
    match level.to_ascii_lowercase().as_str() {
        "off" => LevelFilter::OFF,
        "error" => LevelFilter::ERROR,
        "warn" => LevelFilter::WARN,
        "info" => LevelFilter::INFO,
        "debug" => LevelFilter::DEBUG,
        "trace" => LevelFilter::TRACE,
        other => {
            eprintln!("unknown log_level '{other}', falling back to 'info'");
            LevelFilter::INFO
        }
    }
}

fn open_log_file(path: &str) -> anyhow::Result<std::fs::File> {
    std::fs::File::options()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("opening log file {path}"))
}

/// No per-layer `with_filter` here: a `Filtered` layer only gets its `FilterId`
/// when the subscriber is assembled, and one swapped in later via
/// `reload::Handle::reload` never does — debug builds assert, release builds
/// filter wrongly in silence. The level goes on the subscriber instead (`init`).
fn build_layer<W>(writer: W, ansi: bool) -> BoxedLayer
where
    W: for<'w> MakeWriter<'w> + Send + Sync + 'static,
{
    tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(ansi)
        .boxed()
}

struct LogReloader {
    handle: reload::Handle<BoxedLayer, Registry>,
    path: String,
}

impl LogReloader {
    fn install_non_blocking(&self) -> anyhow::Result<WorkerGuard> {
        let (writer, guard) = tracing_appender::non_blocking(open_log_file(&self.path)?);
        self.handle
            .reload(build_layer(writer, false))
            .context("swapping in the non-blocking log writer")?;
        Ok(guard)
    }
}

struct WriterUpgrade {
    reloader: LogReloader,
    guard: Arc<Mutex<Option<WorkerGuard>>>,
}

#[async_trait]
impl BackgroundService for WriterUpgrade {
    async fn start(&self, _shutdown: ShutdownWatch) {
        match self.reloader.install_non_blocking() {
            Ok(guard) => {
                *self.guard.lock().unwrap() = Some(guard);
                info!("file logging switched to the non-blocking writer");
            }
            Err(e) => error!("keeping the synchronous log writer: {e:#}"),
        }
    }
}

pub struct Logging {
    reloader: Option<LogReloader>,
}

impl Logging {
    pub fn attach(self, server: &mut Server) -> LogGuard {
        let guard: Arc<Mutex<Option<WorkerGuard>>> = Arc::new(Mutex::new(None));

        if let Some(reloader) = self.reloader {
            server.add_service(background_service(
                "log writer upgrade",
                WriterUpgrade {
                    reloader,
                    guard: guard.clone(),
                },
            ));
        }

        LogGuard { guard }
    }
}

pub struct LogGuard {
    guard: Arc<Mutex<Option<WorkerGuard>>>,
}

impl LogGuard {
    pub fn flush(self) {
        drop(self.guard.lock().unwrap().take());
    }
}

/// Install the global subscriber, writing synchronously so the setup survives
/// the daemonize fork. `.init()` also bridges Pingora `log` facade into `tracing`
pub fn init(cfg: &MainConfig) -> anyhow::Result<Logging> {
    let level = parse_level(&cfg.log_level);

    let Some(path) = cfg.log_file.clone() else {
        tracing_subscriber::registry()
            .with(build_layer(std::io::stdout, true))
            .with(level)
            .init();
        return Ok(Logging { reloader: None });
    };

    let (layer, handle) = reload::Layer::new(build_layer(open_log_file(&path)?, false));
    tracing_subscriber::registry()
        .with(layer)
        .with(level)
        .init();

    Ok(Logging {
        reloader: Some(LogReloader { handle, path }),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_log_file_appends_instead_of_truncating() {
        let path = std::env::temp_dir().join(format!("janus-append-{}.log", std::process::id()));
        std::fs::write(&path, "first\n").unwrap();

        let file = open_log_file(path.to_str().unwrap()).unwrap();
        drop(file);

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "first\n");
        std::fs::remove_file(&path).ok();
    }
}
