//! Thin Axum bootstrap for the large-upload demo.
//!
//! Binds 127.0.0.1 on `--port <N>` (default 8097). Body handling is chosen
//! by flags:
//!
//! * default — DISK MODE: uploads persist chunk-by-chunk under a UNIQUE
//!   `std::env::temp_dir()` directory;
//! * `--proxy-url <base>` — PROXY MODE: every inbound body stream is
//!   forwarded verbatim to `{base}/blobs|audio/{id}` (zero buffering), so
//!   chaining onto a second disk-mode instance streams through three
//!   processes end-to-end.
//!
//! A memory monitor samples the process RSS from boot; after a graceful
//! shutdown (Ctrl-C) the full report is printed and the process exits
//! non-zero if the peak delta breached the bounded-memory threshold.

use std::sync::Arc;

use large_upload::demo::{self, LargeUploadApp};
use large_upload::memmon;

const DEFAULT_PORT: u16 = 8097;

struct ServerArgs {
    port: u16,
    proxy_url: Option<String>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> ServerArgs {
    let mut parsed = ServerArgs {
        port: DEFAULT_PORT,
        proxy_url: None,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--port" => match args.next().and_then(|value| value.parse::<u16>().ok()) {
                Some(value) => parsed.port = value,
                None => {
                    eprintln!("--port requires a number");
                    std::process::exit(2);
                }
            },
            "--proxy-url" => match args.next() {
                Some(value) => parsed.proxy_url = Some(value),
                None => {
                    eprintln!("--proxy-url requires a value");
                    std::process::exit(2);
                }
            },
            other => {
                eprintln!("unknown argument {other}; usage: large-upload-server [--port <N>] [--proxy-url <base>]");
                std::process::exit(2);
            }
        }
    }
    parsed
}

#[tokio::main]
async fn main() {
    let args = parse_args(std::env::args().skip(1));
    let monitor = memmon::Monitor::start("server");

    let store_dir = std::env::temp_dir().join(format!("large-upload-store-{}", std::process::id()));
    let app = Arc::new(LargeUploadApp::new(store_dir, args.proxy_url.clone()));
    let mode = if app.is_proxy_mode() {
        format!(
            "PROXY MODE, forwarding to {}",
            args.proxy_url.as_deref().unwrap_or_default()
        )
    } else {
        "DISK MODE".to_owned()
    };

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port))
        .await
        .unwrap_or_else(|error| panic!("bind 127.0.0.1:{}: {error}", args.port));
    let address = listener.local_addr().expect("bound address");
    println!("large-upload server listening on http://{address} ({mode})");

    axum::serve(listener, demo::demo_router(app))
        .with_graceful_shutdown(async {
            let _unused = tokio::signal::ctrl_c().await;
            println!("shutdown signal received");
        })
        .await
        .expect("server terminated unexpectedly");

    let threshold = memmon::threshold_mib();
    let report = monitor.finish().await;
    println!("{}", report.render(threshold));
    if report.breaches(threshold) {
        eprintln!(
            "bounded-memory threshold breached ({} MiB): see {THRESHOLD_HINT}",
            memmon::threshold_mib(),
            THRESHOLD_HINT = memmon::THRESHOLD_ENV_VAR,
        );
        std::process::exit(1);
    }
}
