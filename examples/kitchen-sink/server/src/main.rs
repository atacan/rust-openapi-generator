//! Thin Axum bootstrap for the kitchen-sink demo: binds 127.0.0.1 on
//! `--port <N>` (default 8099), wires process-default body limits and NoOp
//! hooks around the shared demo application, prints the bound URL, and serves.

use std::sync::Arc;

use kitchen_sink_server::app;

fn parse_port(mut args: impl Iterator<Item = String>) -> u16 {
    let mut port = 8099_u16;
    while let Some(arg) = args.next() {
        if arg == "--port" {
            match args.next().and_then(|value| value.parse::<u16>().ok()) {
                Some(value) => port = value,
                None => {
                    eprintln!("--port requires a number");
                    std::process::exit(2);
                }
            }
        }
    }
    port
}

#[tokio::main]
async fn main() {
    let port = parse_port(std::env::args().skip(1));
    // UNIQUE per-process directory under std::env::temp_dir(): uploaded
    // streams land here chunk-by-chunk; downloads re-stream from these files.
    let objects_dir =
        std::env::temp_dir().join(format!("kitchen-sink-objects-{}", std::process::id()));
    let app = Arc::new(app::KitchenSinkApp::new(objects_dir));
    let router = app::demo_router(app);
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .unwrap_or_else(|error| panic!("bind 127.0.0.1:{port}: {error}"));
    let address = listener.local_addr().expect("bound address");
    println!("kitchen-sink server listening on http://{address}");
    axum::serve(listener, router)
        .await
        .expect("server terminated unexpectedly");
}
