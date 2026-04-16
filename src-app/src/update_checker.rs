//! Background update checker — queries GitHub Releases API at startup,
//! deposits the result into a shared slot for the main thread to pick up.

use semver::Version;

const GITHUB_API: &str = "https://api.github.com/repos/ArthurDEV44/paneflow/releases/latest";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq)]
pub enum UpdateStatus {
    Checking,
    Available { version: String, url: String },
    UpToDate,
    Failed,
}

pub type SharedUpdateSlot = std::sync::Arc<std::sync::Mutex<Option<UpdateStatus>>>;

/// Spawn a detached thread that checks GitHub for a newer release.
/// The result is deposited into the returned shared slot.
pub fn spawn_check() -> SharedUpdateSlot {
    let slot: SharedUpdateSlot =
        std::sync::Arc::new(std::sync::Mutex::new(Some(UpdateStatus::Checking)));
    let writer = std::sync::Arc::clone(&slot);
    std::thread::spawn(move || {
        let status = check_github_release();
        *writer.lock().unwrap_or_else(|e| e.into_inner()) = Some(status);
    });
    slot
}

#[derive(serde::Deserialize)]
struct GitHubRelease {
    tag_name: String,
    html_url: String,
}

fn check_github_release() -> UpdateStatus {
    let response = ureq::get(GITHUB_API)
        .header("User-Agent", &format!("paneflow/{CURRENT_VERSION}"))
        .header("Accept", "application/vnd.github.v3+json")
        .call();

    let mut response = match response {
        Ok(r) => r,
        Err(e) => {
            log::warn!("update check failed: {e}");
            return UpdateStatus::Failed;
        }
    };

    let release: GitHubRelease = match response.body_mut().read_json() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("update check: failed to parse response: {e}");
            return UpdateStatus::Failed;
        }
    };

    let remote_tag = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);

    let remote = match Version::parse(remote_tag) {
        Ok(v) => v,
        Err(e) => {
            log::warn!(
                "update check: invalid remote version '{}': {e}",
                release.tag_name
            );
            return UpdateStatus::Failed;
        }
    };
    let local = match Version::parse(CURRENT_VERSION) {
        Ok(v) => v,
        Err(_) => return UpdateStatus::Failed,
    };

    if remote > local {
        log::info!("update available: v{remote} (current: v{local})");
        UpdateStatus::Available {
            version: remote.to_string(),
            url: release.html_url,
        }
    } else {
        log::info!("up to date (v{local})");
        UpdateStatus::UpToDate
    }
}
