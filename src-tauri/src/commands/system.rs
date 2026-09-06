//! System-level IPC commands (SPEC.md §7): autostart, `open_in_explorer`.

use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_opener::OpenerExt;

use crate::error::AppError;

/// Enables or disables launching oSystems Sync on Windows login.
#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), AppError> {
    let autolaunch = app.autolaunch();
    if enabled {
        autolaunch.enable()?;
    } else {
        autolaunch.disable()?;
    }
    Ok(())
}

/// Reveals `path` in the OS file manager (Finder/Explorer/Nautilus), selecting it —
/// used by the "Mostrar no Finder/Explorer" row action (SPEC.md §7). Distinct from
/// `commands::logs::open_logs_folder`, which just opens a directory without
/// selecting anything inside it.
#[tauri::command]
pub async fn open_in_explorer(app: AppHandle, path: String) -> Result<(), AppError> {
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(AppError::from)
}

/// The author's public profiles, rendered as the credits block under the
/// sidebar's folder card.
///
/// Modelled as a closed enum rather than a `url: String` parameter on
/// purpose: `open_url` hands its argument to the OS handler, so a command
/// that accepts an arbitrary URL from the renderer is a launcher for whatever
/// the renderer can be talked into sending. There is nothing here the user
/// picks, so nothing needs to be parameterised — the renderer names *which*
/// profile, and this module owns the address. The capability allowlist in
/// `capabilities/default.json` is the second, independent gate.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub enum AuthorLink {
    Instagram,
    Facebook,
    Linkedin,
    Github,
    Email,
    Whatsapp,
}

impl AuthorLink {
    fn url(self) -> &'static str {
        match self {
            Self::Instagram => "https://www.instagram.com/plocemourasouza",
            Self::Facebook => "https://www.facebook.com/plocemourasouza",
            Self::Linkedin => "https://www.linkedin.com/in/psouza/",
            Self::Github => "https://www.github.com/plocemourasouza",
            // `mailto:` reaches the OS default mail client. The plugin's
            // scope only gates its own `open_url` *command*; this Rust-side
            // call is not scoped, so no capability entry is what makes this
            // work — the closed enum is.
            Self::Email => "mailto:plocemourasouza@gmail.com",
            // `wa.me` is WhatsApp's own click-to-chat endpoint: it opens the
            // desktop app when installed and WhatsApp Web otherwise, with the
            // conversation already open. Digits only, country code first —
            // +55 (Brazil) 55 (DDD) 99125-1975.
            Self::Whatsapp => "https://wa.me/5555991251975",
        }
    }
}

/// Opens one of the author's profiles in the user's default browser
/// (RF-093). Uses `tauri-plugin-opener`, the same path as `open_remote`.
#[tauri::command]
pub async fn open_author_link(app: AppHandle, link: AuthorLink) -> Result<(), AppError> {
    app.opener()
        .open_url(link.url(), None::<&str>)
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    // The addresses are the point of the enum: if they can drift silently,
    // owning them in Rust bought nothing.
    #[test]
    fn every_profile_is_an_https_url_on_its_own_domain() {
        for (link, domain) in [
            (AuthorLink::Instagram, "instagram.com"),
            (AuthorLink::Facebook, "facebook.com"),
            (AuthorLink::Linkedin, "linkedin.com"),
            (AuthorLink::Github, "github.com"),
        ] {
            let url = link.url();
            assert!(url.starts_with("https://"), "{link:?} must be https: {url}");
            assert!(
                url.starts_with(&format!("https://www.{domain}/")),
                "{link:?} must point at {domain}: {url}"
            );
        }
    }

    // The two non-profile contacts have their own shapes, and both are easy to
    // get subtly wrong: a `mailto:` with a stray space, or a `wa.me` number
    // carrying the `+`, parentheses or dash that WhatsApp rejects.
    #[test]
    fn email_is_a_bare_mailto() {
        let url = AuthorLink::Email.url();

        assert_eq!(url, "mailto:plocemourasouza@gmail.com");
        assert!(!url.contains(char::is_whitespace));
    }

    #[test]
    fn whatsapp_is_a_digits_only_click_to_chat_link() {
        let url = AuthorLink::Whatsapp.url();
        let number = url
            .strip_prefix("https://wa.me/")
            .expect("must be a wa.me click-to-chat link");

        assert!(
            number.chars().all(|c| c.is_ascii_digit()),
            "wa.me rejects +, spaces, parentheses and dashes: {number}"
        );
        // Country code (55) + DDD (55) + 9-digit mobile.
        assert_eq!(number.len(), 13, "unexpected length for {number}");
        assert!(number.starts_with("55"), "must carry Brazil's country code");
    }
}
