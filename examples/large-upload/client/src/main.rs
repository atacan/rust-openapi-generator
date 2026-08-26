//! Thin driver for the large-upload demo.
//!
//! Takes the server base URL from `--base-url <url>`
//! (default http://127.0.0.1:8097) and the payload size from
//! `--size-mib <N>` (default 1024 = the full 1 GiB demonstration;
//! `--keep` preserves the synthesized file for inspection).
//!
//! Flow: synthesize a deterministic WAV file chunk-wise → stream it through
//! BOTH documented media types (octet-stream `/blobs/{id}` and audio/wav
//! `/audio/{id}`) with live progress/RSS lines → verify both receipts →
//! print the memory report and exit non-zero if any step failed OR the peak
//! RSS delta breached the bounded-memory threshold.

use large_upload_client::transfers;
use large_upload_memmon as memmon;

const DEFAULT_BASE_URL: &str = "http://127.0.0.1:8097";
const DEFAULT_SIZE_MIB: u64 = 1024;

struct ClientArgs {
    base_url: String,
    size_mib: u64,
    keep: bool,
}

fn parse_args(mut args: impl Iterator<Item = String>) -> ClientArgs {
    let mut parsed = ClientArgs {
        base_url: DEFAULT_BASE_URL.to_owned(),
        size_mib: DEFAULT_SIZE_MIB,
        keep: false,
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--base-url" => match args.next() {
                Some(value) => parsed.base_url = value,
                None => {
                    eprintln!("--base-url requires a value");
                    std::process::exit(2);
                }
            },
            "--size-mib" => match args.next().and_then(|value| value.parse::<u64>().ok()) {
                Some(value) => parsed.size_mib = value,
                None => {
                    eprintln!("--size-mib requires a number");
                    std::process::exit(2);
                }
            },
            "--keep" => parsed.keep = true,
            other => {
                eprintln!(
                    "unknown argument {other}; usage: large-upload-client \
                     [--base-url <url>] [--size-mib <N>] [--keep]"
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
    println!(
        "large-upload client: {} MiB payload through octet-stream AND audio/wav against {}",
        args.size_mib, args.base_url
    );

    let work_dir = std::env::temp_dir().join(format!("large-upload-client-{}", std::process::id()));
    std::fs::create_dir_all(&work_dir).expect("create client work dir");

    let client = match transfers::build_client(&args.base_url) {
        Ok(client) => client,
        Err(error) => {
            eprintln!(
                "failed to build the client against {}: {error}",
                args.base_url
            );
            std::process::exit(2);
        }
    };

    let (steps, report) = transfers::run_transfers(&client, &work_dir, args.size_mib).await;

    for step in &steps {
        println!(
            "{:>15}  {:>4}  {}",
            step.op,
            if step.ok { "ok" } else { "FAIL" },
            step.message
        );
    }

    let threshold = memmon::threshold_mib();
    println!("{}", report.render(threshold));

    if !args.keep {
        let _unused = std::fs::remove_dir_all(&work_dir);
    }

    let failed_steps = steps.iter().filter(|step| !step.ok).count();
    let breached = report.breaches(threshold);
    if failed_steps > 0 || breached {
        eprintln!(
            "{failed_steps} of {} steps failed; bounded-memory check: {}",
            steps.len(),
            if breached { "BREACHED" } else { "pass" }
        );
        std::process::exit(1);
    }
    println!("all {} steps passed within the memory budget", steps.len());
}
