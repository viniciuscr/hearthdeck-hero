//! Simkl as a Watch account: the client id this application carries, and the
//! device ("PIN") flow a TV signs in with.
//!
//! Simkl is a **tracker**, not a media provider: it keeps what a person watched
//! in step across devices. Like TMDB, it authenticates every call with an
//! *application* credential — a **client id**, which belongs to the app and is
//! asked for once (the bootstrap writes it, or the build carries it) — and the
//! person's part is a short code, typed on their phone. Neither the client id
//! nor the resulting tokens are ever logged.
//!
//! Only signing in lives here for now: the API calls the tokens unlock come
//! later, in their own slice.

pub mod auth;

pub use auth::{DeviceCode, Session, Step, Tokens};

/// The name the credentials of this provider are kept under.
pub const PROVIDER: &str = "simkl";

/// The file a machine keeps the app's client id in, under the config folder
/// (`~/.config/hearthdeck/simkl-client-id`): what the bootstrap writes, adopted
/// into the credential store once.
pub const CLIENT_ID_FILE: &str = "simkl-client-id";

/// The client id this build carries (`HEARTHDECK_SIMKL_CLIENT_ID` when it was
/// compiled). Simkl needs one for every call, the sign-in included — there is no
/// anonymous way in — so the application is what holds one.
pub fn built_in_client_id() -> Option<&'static str> {
    option_env!("HEARTHDECK_SIMKL_CLIENT_ID").map(str::trim).filter(|id| !id.is_empty())
}

/// The client id in `dir/` ([`CLIENT_ID_FILE`]), when the file is there and
/// holds something.
pub fn client_id_file(dir: &std::path::Path) -> Option<String> {
    let text = std::fs::read_to_string(dir.join(CLIENT_ID_FILE)).ok()?;
    let line = text.lines().next().unwrap_or_default().trim();
    // What a person pastes carries whatever the source had around it.
    let client_id = line.trim_matches(|c: char| c == '"' || c == '\'' || c.is_whitespace()).to_owned();
    if client_id.is_empty() {
        return None;
    }
    Some(client_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_id_file_is_read_past_what_a_paste_carries() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(client_id_file(dir.path()), None, "no file, no client id");
        std::fs::write(dir.path().join(CLIENT_ID_FILE), "\"abc123\"\nignored\n").unwrap();
        assert_eq!(client_id_file(dir.path()).as_deref(), Some("abc123"));
        std::fs::write(dir.path().join(CLIENT_ID_FILE), "   \n").unwrap();
        assert_eq!(client_id_file(dir.path()), None, "a blank file is no client id");
    }
}
