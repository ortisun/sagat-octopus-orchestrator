//! `sagat validate`: the manifest and every document, locally with the platform's own codec and,
//! where the platform is reachable, against what only it knows (secrets, scripts).

use sagat_application::definitions::IrCodec;
use sagat_infra_ir::JsonIrCodec;
use serde_json::{Value, json};

use crate::args::Common;
use crate::{EXIT_DONE, EXIT_FAILED, Env, Io, manifest, remote, session};

/// What the codec says about a document, one line per problem. Empty when it would be published.
pub(crate) fn local_problems(document: &[u8]) -> Vec<String> {
    match JsonIrCodec.parse(document) {
        Ok(_) => Vec::new(),
        Err(error) => sagat_application::definitions::problems_of(&error)
            .into_iter()
            .map(|p| {
                let place = p
                    .node
                    .or(p.path)
                    .map(|w| format!(" at {w}"))
                    .unwrap_or_default();
                format!("{}{place}: {}", p.code, p.message)
            })
            .collect(),
    }
}

pub(crate) async fn run(common: &Common, env: &Env, io: &mut Io<'_>) -> Result<u8, String> {
    let path = crate::manifest_path(common, env);
    let (manifest, loaded) = match manifest::load(&path) {
        Ok(found) => found,
        Err(problems) => {
            for problem in problems {
                io.warn(format!("  {problem}"));
            }
            io.warn(format!("{} is not valid", path.display()));
            return Ok(EXIT_FAILED);
        }
    };
    let mut failed = 0usize;
    for item in &loaded {
        let problems = local_problems(&item.document);
        if problems.is_empty() {
            io.say(format!("ok    {}", item.entry.name));
        } else {
            failed += 1;
            io.say(format!("FAIL  {}", item.entry.name));
            for problem in problems {
                io.say(format!("        {problem}"));
            }
        }
    }
    if failed > 0 {
        io.warn(format!(
            "{failed} of {} documents are not valid",
            loaded.len()
        ));
        return Ok(EXIT_FAILED);
    }
    let namespace = common
        .namespace
        .clone()
        .unwrap_or(manifest.namespace.clone());
    // Nothing given means a local check, said as such. Something given that cannot be used (a token
    // file that is not there, an address that is not one) is a failure: a gate that passes the
    // flags and silently skips what they were for is worse than one that stops.
    let given = common.server.is_some()
        || common.token_file.is_some()
        || env.server.is_some()
        || env.token.is_some();
    let credentials = crate::credentials(common, env);
    if given {
        credentials.as_ref().map_err(Clone::clone)?;
    }
    if credentials.is_err() {
        io.say(format!(
            "{} documents are valid as documents. Not checked against the platform (secrets, scripts): \
             pass --server and a token to do that.",
            loaded.len()
        ));
        return Ok(EXIT_DONE);
    }
    let session = session(common, env, &namespace)?;
    let known = remote::workflows(&session.client).await?;
    let mut problems_found = 0usize;
    let mut unchecked = Vec::new();
    for item in &loaded {
        let Some(existing) = known.get(&item.entry.name) else {
            unchecked.push(item.entry.name.as_str());
            continue;
        };
        let body = json!({ "ir": item.value });
        let (_, answer) = session
            .client
            .call(
                "POST",
                &format!("/workflows/{}/validate", existing.id),
                Some(&body),
                None,
            )
            .await
            .map_err(|e| e.to_string())?;
        let problems = answer["problems"].as_array().cloned().unwrap_or_default();
        for warning in answer["warnings"].as_array().into_iter().flatten() {
            io.say(format!("warn  {}  {}", item.entry.name, describe(warning)));
        }
        if !problems.is_empty() {
            problems_found += 1;
            io.say(format!("FAIL  {} (the platform)", item.entry.name));
            for problem in &problems {
                io.say(format!("        {}", describe(problem)));
            }
        }
    }
    if problems_found > 0 {
        io.warn(format!(
            "{problems_found} documents are refused by the platform"
        ));
        return Ok(EXIT_FAILED);
    }
    if !unchecked.is_empty() {
        io.say(format!(
            "not checked by the platform, which has no workflow of these names yet: {}",
            unchecked.join(", ")
        ));
    }
    io.say(format!(
        "{} documents are valid ({} checked by the platform)",
        loaded.len(),
        loaded.len() - unchecked.len()
    ));
    Ok(EXIT_DONE)
}

fn describe(problem: &Value) -> String {
    let text = |key: &str| problem.get(key).and_then(Value::as_str);
    let place = text("nodeId")
        .or_else(|| text("path"))
        .map(|w| format!(" at {w}"))
        .unwrap_or_default();
    format!(
        "{}{place}: {}",
        text("code").unwrap_or("PROBLEM"),
        text("message").unwrap_or("")
    )
}
