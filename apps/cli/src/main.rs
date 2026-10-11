//! `sagat` executable: arguments, environment and standard streams only.

use std::process::ExitCode;

use sagat_cli::{Env, run};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    // A byte that is not text in an argument is a mistake to report, not a reason to panic.
    let args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let env = Env::from_process();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let code = run(&args, &env, &mut out, &mut err).await;
    ExitCode::from(code)
}
