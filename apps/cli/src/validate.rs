//! `sagat validate`: the manifest and every document, locally with the platform's own codec and,
//! where the platform is reachable, against what only it knows (secrets, scripts).

use serde_json::{Value, json};

use crate::args::Common;
use crate::{EXIT_DONE, EXIT_FAILED, Env, Io, Session, manifest, remote, session};

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
    let docs = io.docs().map(str::to_owned);
    for item in &loaded {
        let (diagnostics, classification) = crate::diagnostics::analyze_document(&item.document);
        let errors = diagnostics.iter().filter(|d| !d.warning).count();
        if errors == 0 {
            io.say(format!("ok    {}", item.entry.name));
        } else {
            failed += 1;
            io.say(format!("FAIL  {}", item.entry.name));
        }
        for line in crate::diagnostics::render(&diagnostics, docs.as_deref()) {
            io.say(format!("        {line}"));
        }
        if let Some(analysis) = classification {
            for line in crate::diagnostics::classification_report(
                &item.entry.name,
                &analysis.report,
                analysis.report_truncated,
            ) {
                io.say(format!("        {line}"));
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
        if !check_on_platform(&session, &existing.id, &item.value, &item.entry.name, io).await? {
            problems_found += 1;
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

/// Asks the platform whether it would publish `document` as the workflow `id`, and prints what it
/// found: the warnings, and the problems with their help. `true` when there are no problems.
pub(crate) async fn check_on_platform(
    session: &Session,
    id: &str,
    document: &Value,
    label: &str,
    io: &mut Io<'_>,
) -> Result<bool, String> {
    let body = json!({ "ir": document });
    let (_, answer) = session
        .client
        .call(
            "POST",
            &format!("/workflows/{id}/validate"),
            Some(&body),
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    let docs = io.docs().map(str::to_owned);
    let problems = answer["problems"].as_array().cloned().unwrap_or_default();
    let warnings: Vec<_> = answer["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(crate::diagnostics::of_server_problem)
        // Classification is a pure document analysis already printed by the local codec above.
        .filter(|warning| !warning.code.starts_with("CLASSIFICATION_"))
        .collect();
    for line in crate::diagnostics::render(&warnings, docs.as_deref()) {
        io.say(format!("        {line}"));
    }
    if !problems.is_empty() {
        io.say(format!("FAIL  {label} (the platform)"));
        let found: Vec<_> = problems
            .iter()
            .map(crate::diagnostics::of_server_problem)
            .collect();
        for line in crate::diagnostics::render(&found, docs.as_deref()) {
            io.say(format!("        {line}"));
        }
    }
    Ok(problems.is_empty())
}
