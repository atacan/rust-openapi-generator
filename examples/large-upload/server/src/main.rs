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
//!   processes end-to-end. Requires building with `--features proxy`
//!   (`cargo run -p large-upload-server --features proxy`); a default build
//!   rejects the flag instead of silently pulling reqwest into its graph.
//!
//! A memory monitor samples the process RSS from boot; after a graceful
//! shutdown (Ctrl-C) the full report is printed and the process exits
//! non-zero if the peak delta breached the bounded-memory threshold.

use std::sync::Arc;

use large_upload_server::app::{self, LargeUploadApp};
use large_upload_server::memmon;

const DEFAULT_PORT: u16 = 8097;

struct ServerArgs {
    port: u16,
    /// Proxy target base; parsed only under the `proxy` feature (the flag is
    /// rejected otherwise) and always `None` on a default build.
    #[cfg(feature = "proxy")]
    proxy_url: Option<String>,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> ServerArgs {
    let mut parsed = ServerArgs {
        port: DEFAULT_PORT,
        #[cfg(feature = "proxy")]
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
            "--proxy-url" => {
                #[cfg(feature = "proxy")]
                match args.next() {
                    Some(value) => parsed.proxy_url = Some(value),
                    None => {
                        eprintln!("--proxy-url requires a value");
                        std::process::exit(2);
                    }
                }
                #[cfg(not(feature = "proxy"))]
                {
                    let _unused = args.next();
                    eprintln!(
                        "--proxy-url requires building this crate with \
                         --features proxy (cargo run -p large-upload-server \
                         --features proxy -- --proxy-url <base>)"
                    );
                    std::process::exit(2);
                }
            }
            other => {
                eprintln!(
                    "unknown argument {other}; usage: large-upload-server \
                     [--port <N>] [--proxy-url <base> (needs --features proxy)]"
                );
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
    let app = Arc::new(LargeUploadApp::new(
        store_dir,
        #[cfg(feature = "proxy")]
        args.proxy_url.clone(),
        #[cfg(not(feature = "proxy"))]
        None,
    ));
    #[cfg(feature = "proxy")]
    let mode = if app.is_proxy_mode() {
        format!(
            "PROXY MODE, forwarding to {}",
            args.proxy_url.as_deref().unwrap_or_default()
        )
    } else {
        "DISK MODE".to_owned()
    };
    // Without the feature the flag was already rejected above; the process
    // is always a pure disk-mode store here.
    #[cfg(not(feature = "proxy"))]
    let mode = String::from("DISK MODE");

    let listener = tokio::net::TcpListener::bind(("127.0.0.1", args.port))
        .await
        .unwrap_or_else(|error| panic!("bind 127.0.0.1:{}: {error}", args.port));
    let address = listener.local_addr().expect("bound address");
    println!("large-upload server listening on http://{address} ({mode})");

    axum::serve(listener, app::demo_router(app))
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
