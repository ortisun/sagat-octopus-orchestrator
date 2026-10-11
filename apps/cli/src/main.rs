//! `sagat` executable: arguments, environment and standard streams only.

use std::process::ExitCode;

use sagat_cli::{Env, run};

#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let env = Env::from_process();
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let code = run(&args, &env, &mut out, &mut err).await;
    ExitCode::from(code)
}
