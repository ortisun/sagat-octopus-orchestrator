//! `sagat dev token`: a token from the development identity provider of the local stack, so that
//! the first day does not begin with setting up an identity provider.

use std::path::Path;

use serde_json::json;

use crate::client::{CallError, Client};
use crate::{EXIT_DONE, Env, Io, one_line, resolve};

/// Where the stack's provider listens on this machine (`infra/docker/compose.yaml`).
const DEFAULT_IDP: &str = "http://127.0.0.1:18090";
/// A token is a few hundred bytes; this is a provider that is not the one it should be.
const MAX_TOKEN: usize = 8 * 1024;

pub(crate) struct Ask<'a> {
    pub idp: Option<&'a str>,
    pub subject: &'a str,
    pub tenant: &'a str,
    pub ttl: Option<u64>,
    pub out: Option<&'a Path>,
}

pub(crate) async fn token(env: &Env, ask: &Ask<'_>, io: &mut Io<'_>) -> Result<u8, String> {
    let idp = ask
        .idp
        .map(str::to_owned)
        .or_else(|| env.idp.clone())
        .unwrap_or_else(|| DEFAULT_IDP.to_owned());
    // The client wants a token to carry; this call carries none.
    let client = Client::new(&idp, "none", "")?;
    let mut body = json!({ "subject": ask.subject, "tenant": ask.tenant });
    if let Some(ttl) = ask.ttl {
        body["ttlSeconds"] = json!(ttl);
    }
    let (status, answer) = client
        .post_public("/token", &body)
        .await
        .map_err(|e| match e {
            CallError::Transport(why) => format!(
                "no answer from the development identity provider at {}: {why}. Is the stack up (docker compose up)? It listens on {DEFAULT_IDP} unless SAGAT_DEV_IDP or --idp says otherwise",
                one_line(&idp)
            ),
            CallError::Refused(failure) => failure.to_string(),
        })?;
    if status != 200 {
        let why = answer["message"]
            .as_str()
            .unwrap_or("it refused the request");
        return Err(format!(
            "the development identity provider refused ({status}): {}",
            one_line(why)
        ));
    }
    let token = answer["access_token"].as_str().unwrap_or_default();
    if token.is_empty() || token.len() > MAX_TOKEN || !token.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("the development identity provider gave no token this tool can use".to_owned());
    }
    let seconds = answer["expires_in"].as_u64().unwrap_or(0);
    match ask.out {
        Some(path) => {
            let path = resolve(env, path);
            write_private(&path, token)
                .map_err(|e| format!("could not write {}: {e}", path.display()))?;
            io.say(format!(
                "wrote a token for {} of {} to {} (valid {seconds} s); use it with --token-file",
                one_line(ask.subject),
                one_line(ask.tenant),
                path.display()
            ));
        }
        None => {
            // Only the token on the output, so that `export SAGAT_TOKEN=$(sagat dev token)` works.
            io.say(token);
            io.warn(format!(
                "a token for {} of {}, valid {seconds} s",
                one_line(ask.subject),
                one_line(ask.tenant)
            ));
        }
    }
    Ok(EXIT_DONE)
}

/// The token in a file only its owner reads, replacing what was there.
fn write_private(path: &Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    // Not through a link: a file somebody planted there must not be overwritten with a token.
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(std::io::Error::other(
            "it is a symbolic link; name a file, or remove the link",
        ));
    }
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    #[cfg(unix)]
    {
        // A file that existed keeps the mode it had: make it the owner's alone.
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(token.as_bytes())?;
    file.write_all(b"\n")
}
