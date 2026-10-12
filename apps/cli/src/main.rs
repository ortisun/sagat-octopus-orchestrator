//! `sagat` executable: arguments, environment and standard streams only.

use std::process::ExitCode;

use sagat_cli::{Env, run, telemetry::Onboarding};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // A byte that is not text in an argument is a mistake to report, not a reason to panic.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let telemetry = match Onboarding::from_process(&args) {
        Ok(telemetry) => telemetry,
        Err(error) => {
            eprintln!("sagat: invalid telemetry configuration: {error}");
            return ExitCode::from(2);
        }
    };
    let env = Env::from_process();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let code = telemetry
        .observe(run(&args, &env, &mut out, &mut err))
        .await;
    if code != 0 {
        eprintln!("sagat: trace {}", telemetry.trace_id());
    }
    if let Err(error) = telemetry.finish(code).await {
        eprintln!("sagat: onboarding telemetry: {error}");
    }
    ExitCode::from(code)
}
