//! `sagat export`: the active version of every workflow of a namespace, written as a directory a
//! manifest can publish back. The way to bring a namespace that was built in the Studio under Git.

use std::path::Path;

use serde_json::json;

use crate::args::Common;
use crate::manifest::{API_VERSION, KIND, is_slug};
use crate::{EXIT_DONE, Env, Io, namespace_of, remote, session};

/// The name of the manifest written next to the documents.
const MANIFEST: &str = "sagat.yaml";

/// Writes a file that does not exist yet: an existing file, and a link in the way (even one that
/// points nowhere), are refused and not followed, so exporting never writes where somebody planted
/// a link.
fn write_new(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|e| format!("{}: {e} (export into an empty directory)", path.display()))?;
    file.write_all(text.as_bytes())
        .map_err(|e| format!("{}: {e}", path.display()))
}

pub(crate) async fn run(
    common: &Common,
    env: &Env,
    out: &Path,
    io: &mut Io<'_>,
) -> Result<u8, String> {
    let namespace = namespace_of(common, env)?;
    let session = session(common, env, &namespace)?;
    let client = &session.client;
    let dir = if out.is_absolute() {
        out.to_path_buf()
    } else {
        env.cwd.join(out)
    };
    if dir.join(MANIFEST).exists() {
        return Err(format!(
            "{} already has a {MANIFEST}: export into an empty directory, so nothing is overwritten",
            dir.display()
        ));
    }
    let workflows = remote::workflows(client).await?;
    if workflows.len() > crate::manifest::MAX_WORKFLOWS {
        return Err(format!(
            "{namespace} has {} workflows and a manifest lists at most {}: nothing was written",
            workflows.len(),
            crate::manifest::MAX_WORKFLOWS
        ));
    }
    let mut entries = Vec::new();
    let mut skipped = Vec::new();
    let mut documents = Vec::new();
    for (name, workflow) in &workflows {
        // A name comes from the platform and becomes a file name: it is checked, not trusted.
        if !is_slug(name) {
            skipped.push(format!("{name} (not a name this tool writes)"));
            continue;
        }
        let Some(active) = &workflow.active else {
            skipped.push(format!("{name} (no active version)"));
            continue;
        };
        let document = remote::version(client, &workflow.id, active.number).await?;
        let file = format!("workflows/{name}.json");
        let text = serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?;
        documents.push((file.clone(), text));
        entries.push(json!({ "name": name, "file": file }));
    }
    let manifest = json!({
        "apiVersion": API_VERSION,
        "kind": KIND,
        "namespace": namespace,
        "workflows": entries,
    });
    std::fs::create_dir_all(dir.join("workflows"))
        .map_err(|e| format!("{}: {e}", dir.display()))?;
    for (file, text) in &documents {
        write_new(&dir.join(file), &format!("{text}\n"))?;
    }
    let text = serde_saphyr::to_string(&manifest).map_err(|e| e.to_string())?;
    write_new(&dir.join(MANIFEST), &text)?;
    io.say(format!(
        "exported {} workflows of {namespace} to {}",
        documents.len(),
        dir.display()
    ));
    for skipped in skipped {
        io.warn(format!("skipped {skipped}"));
    }
    Ok(EXIT_DONE)
}
