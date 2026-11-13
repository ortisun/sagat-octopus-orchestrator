mod builders;
mod runtime;
use sagat_octopus_orchestrator_sdk::Value;
use std::path::Path;
fn read(path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&std::fs::read(path)?)?)
}
fn templated(value: &mut Value, unique: &str) {
    match value {
        Value::String(text) => *text = text.replace("{{unique}}", unique),
        Value::Array(values) => {
            for value in values {
                templated(value, unique);
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                templated(value, unique);
            }
        }
        _ => {}
    }
}
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let mode = args.first().map(String::as_str).unwrap_or_default();
    if !matches!((mode, args.len()), ("build", 2) | ("run", 4)) {
        return Err("usage: runner build <scenario> | run <scenario> <config> <out>".into());
    }
    let mut scenario = read(&args[1])?;
    if mode == "build" {
        println!("{}", serde_json::to_string(&builders::build(&scenario)?)?);
        return Ok(());
    }
    let config = read(&args[2])?;
    templated(
        &mut scenario,
        config["unique"]
            .as_str()
            .ok_or("config unique is missing")?,
    );
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(280),
        runtime::run(&scenario, &config),
    )
    .await??;
    std::fs::write(Path::new(&args[3]), serde_json::to_vec(&result)?)?;
    Ok(())
}
