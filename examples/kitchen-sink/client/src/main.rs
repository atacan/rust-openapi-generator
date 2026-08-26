//! Thin driver for the kitchen-sink demo: takes the server base URL from
//! `--base-url <url>` (default http://127.0.0.1:8099), builds the generated
//! client, runs the shared full-operation sweep (every documented operation
//! once), prints one line per step, and exits non-zero only AFTER everything
//! ran if any step failed.

use kitchen_sink_client::sweep;

fn parse_base_url(mut args: impl Iterator<Item = String>) -> String {
    let mut base_url = "http://127.0.0.1:8099".to_owned();
    while let Some(arg) = args.next() {
        if arg == "--base-url" {
            match args.next() {
                Some(value) => base_url = value,
                None => {
                    eprintln!("--base-url requires a value");
                    std::process::exit(2);
                }
            }
        }
    }
    base_url
}

#[tokio::main]
async fn main() {
    let base_url = parse_base_url(std::env::args().skip(1));
    let client = match sweep::build_client(&base_url) {
        Ok(client) => client,
        Err(error) => {
            eprintln!("failed to build the kitchen-sink client against {base_url}: {error}");
            std::process::exit(2);
        }
    };

    let steps = sweep::run_sweep(&client).await;
    for step in &steps {
        println!(
            "{:>20}  {:>4}  {}",
            step.op,
            if step.ok { "ok" } else { "FAIL" },
            step.message
        );
    }

    let failed = steps.iter().filter(|step| !step.ok).count();
    if failed > 0 {
        eprintln!(
            "{failed} of {} steps failed against {base_url}",
            steps.len()
        );
        std::process::exit(1);
    }
    println!("all {} steps passed against {base_url}", steps.len());
}
