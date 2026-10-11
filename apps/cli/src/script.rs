//! `sagat script test|publish`: try a script in the platform's sandbox, and build it so that a
//! workflow can pin it.
//!
//! The source of a script is a file in the repository like any other: the tool creates the script
//! in the namespace (named after the file) the first time and replaces its source after, then asks
//! for a test run or a version. The platform runs the script in a sandbox that has no network,
//! no files and no secrets, and what comes back is the script's own result and log.

use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::args::Common;
use crate::client::{CallError, Client};
use crate::{
    EXIT_DONE, EXIT_FAILED, EXIT_TIMEOUT, Env, Io, namespace_of, one_line, remote, resolve, session,
};

/// Longest part of a result or a log that is shown.
const SHOWN: usize = 2000;

fn clip(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '\n' { '⏎' } else { c })
        .collect();
    if flat.chars().count() <= SHOWN {
        flat
    } else {
        format!("{}…", flat.chars().take(SHOWN).collect::<String>())
    }
}

/// What to do about each way a test run can fail.
fn failure_help(code: &str) -> &'static str {
    match code {
        "SCRIPT_ERROR" => "the script threw or exited with an error: the log has the message",
        "SCRIPT_TIMEOUT" => {
            "the script ran out of time; look for a loop that does not end, or ask for more in the script's policy"
        }
        "SCRIPT_OUT_OF_MEMORY" => "the script ran out of memory; process less at a time",
        "SCRIPT_OUTPUT_LIMIT" => {
            "the script returned more than the platform keeps of an output; return a reference, not the data"
        }
        "SCRIPT_DENIED" => {
            "the script tried something the sandbox forbids (the network, files, secrets, a process): the denied list says what"
        }
        "SCRIPT_SCHEMA_VIOLATION" => {
            "the input or the result does not fit the schema the script declares"
        }
        "SCRIPT_NO_RESULT" => "the script ended without a result: return a value",
        "SCRIPT_BUILD_REJECTED" => "the script could not be built: the log says why",
        "SCRIPT_SANDBOX_UNAVAILABLE" => {
            "no sandbox runner took the run: the platform has none running (docs/runbooks/scripts.md)"
        }
        _ => "see the log",
    }
}

fn language_of(file: &Path, flag: Option<&str>) -> Result<String, String> {
    if let Some(language) = flag {
        return match language {
            "typescript" | "python" => Ok(language.to_owned()),
            "csharp" | "go" | "java" => Err(format!(
                "{language} scripts are not built yet: the sandbox runs TypeScript and Python"
            )),
            other => Err(format!(
                "'{}' is not a script language (typescript or python)",
                one_line(other)
            )),
        };
    }
    match file.extension().and_then(|e| e.to_str()) {
        Some("ts") => Ok("typescript".to_owned()),
        Some("py") => Ok("python".to_owned()),
        _ => Err("the language is not clear from the file name: pass --language typescript or --language python".to_owned()),
    }
}

/// The name of the script: the flag, or the file's, as the platform takes it.
fn name_of(file: &Path, flag: Option<&str>) -> Result<String, String> {
    let raw = flag.map(str::to_owned).unwrap_or_else(|| {
        file.file_stem().map_or_else(String::new, |s| {
            s.to_string_lossy().to_lowercase().replace(['_', '.'], "-")
        })
    });
    let bytes = raw.as_bytes();
    let ok = (3..=63).contains(&bytes.len())
        && bytes[0].is_ascii_lowercase()
        && bytes[bytes.len() - 1] != b'-'
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
    if ok {
        Ok(raw)
    } else {
        Err(format!(
            "'{}' is not a script name (lower case letters, digits and hyphens, 3 to 63 characters, starting with a letter): pass --name",
            one_line(&raw)
        ))
    }
}

struct Ready {
    namespace_id: String,
    script_id: String,
    name: String,
    language: String,
}

/// Creates the script or replaces its source, and says which.
async fn upsert(
    client: &Client,
    namespace: &str,
    file: &Path,
    env: &Env,
    language: Option<&str>,
    name: Option<&str>,
    io: &mut Io<'_>,
) -> Result<Ready, String> {
    let path = resolve(env, file);
    let source = crate::manifest::read_limited(&path, "script").map_err(|p| p.to_string())?;
    let language = language_of(&path, language)?;
    let name = name_of(&path, name)?;
    let namespace_id = remote::namespaces(client)
        .await?
        .into_iter()
        .find(|n| n.slug == namespace)
        .map(|n| n.id)
        .ok_or_else(|| format!("the tenant has no namespace '{namespace}'"))?;
    let base = format!("/namespaces/{namespace_id}/scripts");
    let existing = match client
        .call("GET", &format!("{base}?limit=200"), None, None)
        .await
    {
        Ok((_, page)) => page["items"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|s| s["name"] == name.as_str())
            .cloned(),
        Err(CallError::Refused(f)) if f.status == 404 => {
            return Err("this platform does not serve scripts (it has no script signing key configured: docs/runbooks/scripts.md)".to_owned());
        }
        Err(e) => return Err(e.to_string()),
    };
    let script = match existing {
        None => {
            let (_, created) = client
                .call(
                    "POST",
                    &base,
                    Some(&json!({ "name": name, "language": language, "source": source })),
                    None,
                )
                .await
                .map_err(|e| e.to_string())?;
            io.say(format!("created script {name} ({language})"));
            created
        }
        Some(found) => {
            let id = found["id"]
                .as_str()
                .ok_or("the platform's answer is not a script")?;
            if found["language"].as_str() != Some(language.as_str()) {
                return Err(format!(
                    "the script {name} is {} and this file is {language}: a script keeps its language; use another --name",
                    found["language"].as_str().unwrap_or("?")
                ));
            }
            let (_, current) = client
                .call("GET", &format!("{base}/{id}"), None, None)
                .await
                .map_err(|e| e.to_string())?;
            if current["source"].as_str() == Some(source.as_str()) {
                current
            } else {
                let (_, updated) = client
                    .call(
                        "PATCH",
                        &format!("{base}/{id}"),
                        Some(&json!({ "revision": current["revision"], "source": source })),
                        None,
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                io.say(format!("replaced the source of script {name}"));
                updated
            }
        }
    };
    Ok(Ready {
        namespace_id,
        script_id: script["id"]
            .as_str()
            .ok_or("the platform's answer is not a script")?
            .to_owned(),
        name,
        language,
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn test(
    common: &Common,
    env: &Env,
    file: &Path,
    input: Option<&str>,
    language: Option<&str>,
    name: Option<&str>,
    timeout: u64,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let input: Value = match input {
        None => json!({}),
        Some(at) if at.starts_with('@') => {
            let path = resolve(env, Path::new(&at[1..]));
            let text = crate::manifest::read_limited(&path, "input").map_err(|p| p.to_string())?;
            serde_json::from_str(&text).map_err(|e| format!("the input is not JSON ({e})"))?
        }
        Some(text) => {
            serde_json::from_str(text).map_err(|e| format!("the input is not JSON ({e})"))?
        }
    };
    let ready = upsert(client, &namespace, file, env, language, name, io).await?;
    let base = format!(
        "/namespaces/{}/scripts/{}",
        ready.namespace_id, ready.script_id
    );
    let (_, queued) = client
        .call(
            "POST",
            &format!("{base}/test-runs"),
            Some(&json!({ "input": input })),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    let id = queued["id"]
        .as_str()
        .ok_or("the platform's answer is not a test run")?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let (_, run) = client
            .call("GET", &format!("{base}/test-runs/{id}"), None, None)
            .await
            .map_err(|e| e.to_string())?;
        let status = run["status"].as_str().unwrap_or("?");
        if matches!(status, "SUCCEEDED" | "FAILED") {
            return Ok(report_test(&ready, &run, io));
        }
        if Instant::now() >= deadline {
            io.warn(format!(
                "the test run is still {status} after {timeout} s: the platform may have no sandbox runner (docs/runbooks/scripts.md)"
            ));
            return Ok(EXIT_TIMEOUT);
        }
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
}

fn report_test(ready: &Ready, run: &Value, io: &mut Io<'_>) -> u8 {
    let status = crate::one_line(run["status"].as_str().unwrap_or("?"));
    let status = status.as_str();
    io.say(format!(
        "script {} ({}) test run {status}{}",
        ready.name,
        ready.language,
        run["wallMs"]
            .as_u64()
            .map_or_else(String::new, |ms| format!(" in {ms} ms"))
    ));
    if status == "SUCCEEDED" {
        io.say(format!("result: {}", clip(&run["result"].to_string())));
    } else if let Some(code) = run["failure"].as_str() {
        io.say(format!("failure: {}", one_line(code)));
        io.say(format!("  help: {}", failure_help(code)));
        for denied in run["denied"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
        {
            io.say(format!("  denied: {}", one_line(denied)));
        }
    }
    let log = run["log"].as_str().unwrap_or("");
    if !log.trim().is_empty() {
        io.say(format!("log: {}", clip(log)));
    }
    if status == "SUCCEEDED" {
        EXIT_DONE
    } else {
        EXIT_FAILED
    }
}

pub(crate) async fn publish(
    common: &Common,
    env: &Env,
    file: &Path,
    language: Option<&str>,
    name: Option<&str>,
    timeout: u64,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let ready = upsert(client, &namespace, file, env, language, name, io).await?;
    let base = format!(
        "/namespaces/{}/scripts/{}",
        ready.namespace_id, ready.script_id
    );
    let (_, answer) = client
        .call("POST", &format!("{base}/versions"), None, None)
        .await
        .map_err(|e| e.to_string())?;
    let number = answer["version"]["number"]
        .as_u64()
        .ok_or("the platform's answer is not a script version")?;
    let deadline = Instant::now() + Duration::from_secs(timeout);
    loop {
        let (_, version) = client
            .call("GET", &format!("{base}/versions/{number}"), None, None)
            .await
            .map_err(|e| e.to_string())?;
        match version["status"].as_str() {
            Some("READY") => {
                let digest = version["artifactDigest"]
                    .as_str()
                    .ok_or("the version is ready and has no artifact")?;
                io.say(format!("script {} version {number} is ready", ready.name));
                let digest = one_line(digest);
                io.say(format!("artifactDigest {digest}"));
                io.say(format!(
                    "pin it in a workflow with the node: {{ \"id\": \"{}\", \"type\": \"script\", \"language\": \"{}\", \"artifactDigest\": \"{digest}\" }}",
                    ready.name, ready.language
                ));
                return Ok(EXIT_DONE);
            }
            Some("FAILED") => {
                io.say(format!(
                    "script {} version {number} was not built",
                    ready.name
                ));
                if let Some(why) = version["failure"].as_str() {
                    io.say(format!("failure: {}", one_line(why)));
                }
                if let Some(log) = version["buildLog"]
                    .as_str()
                    .filter(|l| !l.trim().is_empty())
                {
                    io.say(format!("build log: {}", clip(log)));
                }
                return Ok(EXIT_FAILED);
            }
            _ => {}
        }
        if Instant::now() >= deadline {
            io.warn(format!(
                "version {number} is still being built after {timeout} s: the platform may have no sandbox runner (docs/runbooks/scripts.md); look again with the Studio"
            ));
            return Ok(EXIT_TIMEOUT);
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}
