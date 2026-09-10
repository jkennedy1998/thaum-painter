//! Passive release-status assessment for the Painter command bar.
//!
//! This seam only compares the compiled build against the public website
//! manifest. It never downloads, installs, or changes multiplayer behavior.

use std::{sync::mpsc, thread, time::Duration};

use semver::Version;
use serde::Deserialize;

pub const DOWNLOAD_PAGE_URL: &str = "https://jartanddesign.com/thaum-painter/";
const RELEASE_MANIFEST_URL: &str = "https://jartanddesign.com/thaum-painter/release.json";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseStatus {
    /// The public endpoint is unavailable, malformed, stale, or still being
    /// checked. This deliberately does not claim the build is current.
    CannotAssess,
    UpToDate,
    OutOfDate,
}

impl ReleaseStatus {
    pub fn tooltip(self) -> (&'static str, &'static str) {
        match self {
            Self::UpToDate => (
                "UP TO DATE",
                "Click here for the download page, you are seemingly up to date on this build",
            ),
            Self::CannotAssess => (
                "CANNOT ASSESS",
                "open the downloads page for the painter, cannot assess if you are out of date",
            ),
            Self::OutOfDate => (
                "UPDATE AVAILABLE",
                "opens the downloads page for the latest build, your current build is out of date!",
            ),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ReleaseManifest {
    version: String,
    page: String,
}

/// A launch-time background request. The UI starts in `CannotAssess` and
/// remains interactive while this bounded request is in flight.
pub struct ReleaseStatusCheck {
    status: ReleaseStatus,
    receiver: mpsc::Receiver<ReleaseStatus>,
    pending: bool,
}

impl ReleaseStatusCheck {
    pub fn start(current_version: &'static str) -> Self {
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let _ = sender.send(fetch_status(current_version));
        });
        Self {
            status: ReleaseStatus::CannotAssess,
            receiver,
            pending: true,
        }
    }

    pub fn status(&self) -> ReleaseStatus {
        self.status
    }

    pub fn is_pending(&self) -> bool {
        self.pending
    }

    /// Receives a completed assessment. Returns true only when the button's
    /// visible state must change.
    pub fn poll(&mut self) -> bool {
        match self.receiver.try_recv() {
            Ok(next) => {
                self.pending = false;
                if self.status == next {
                    return false;
                }
                self.status = next;
                true
            }
            Err(mpsc::TryRecvError::Empty) => false,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = false;
                false
            }
        }
    }
}

fn fetch_status(current_version: &str) -> ReleaseStatus {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .build();
    let agent = config.new_agent();
    let response = match agent.get(RELEASE_MANIFEST_URL).call() {
        Ok(response) => response,
        Err(_) => return ReleaseStatus::CannotAssess,
    };
    let body = match response.into_body().read_to_string() {
        Ok(body) => body,
        Err(_) => return ReleaseStatus::CannotAssess,
    };
    status_from_manifest_text(current_version, &body)
}

fn status_from_manifest_text(current_version: &str, manifest_text: &str) -> ReleaseStatus {
    let manifest: ReleaseManifest = match serde_json::from_str::<ReleaseManifest>(manifest_text) {
        Ok(manifest) if manifest.page == DOWNLOAD_PAGE_URL => manifest,
        _ => return ReleaseStatus::CannotAssess,
    };
    let current = match Version::parse(current_version) {
        Ok(version) => version,
        Err(_) => return ReleaseStatus::CannotAssess,
    };
    let latest = match Version::parse(&manifest.version) {
        Ok(version) => version,
        Err(_) => return ReleaseStatus::CannotAssess,
    };
    match latest.cmp(&current) {
        std::cmp::Ordering::Greater => ReleaseStatus::OutOfDate,
        std::cmp::Ordering::Equal => ReleaseStatus::UpToDate,
        // A public page behind the shipped app cannot prove that app is
        // current, so it remains an honest unavailable assessment.
        std::cmp::Ordering::Less => ReleaseStatus::CannotAssess,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "https://jartanddesign.com/thaum-painter/";

    fn manifest(version: &str) -> String {
        format!(r#"{{"version":"{version}","page":"{PAGE}"}}"#)
    }

    #[test]
    fn valid_matching_manifest_is_up_to_date() {
        assert_eq!(
            status_from_manifest_text("0.1.2", &manifest("0.1.2")),
            ReleaseStatus::UpToDate
        );
    }

    #[test]
    fn newer_manifest_is_out_of_date() {
        assert_eq!(
            status_from_manifest_text("0.1.2", &manifest("0.1.3")),
            ReleaseStatus::OutOfDate
        );
    }

    #[test]
    fn semver_prereleases_follow_the_same_newer_release_rule() {
        assert_eq!(
            status_from_manifest_text("0.1.2-alpha.1", &manifest("0.1.2")),
            ReleaseStatus::OutOfDate
        );
        assert_eq!(
            status_from_manifest_text("0.1.2", &manifest("0.1.3-rc.1")),
            ReleaseStatus::OutOfDate
        );
    }

    #[test]
    fn stale_or_invalid_manifest_cannot_assess() {
        assert_eq!(
            status_from_manifest_text("0.1.2", &manifest("0.1.1")),
            ReleaseStatus::CannotAssess
        );
        assert_eq!(
            status_from_manifest_text(
                "0.1.2",
                r#"{"version":"latest","page":"https://jartanddesign.com/thaum-painter/"}"#
            ),
            ReleaseStatus::CannotAssess
        );
        assert_eq!(
            status_from_manifest_text(
                "0.1.2",
                r#"{"version":"0.1.2","page":"https://example.com/"}"#
            ),
            ReleaseStatus::CannotAssess
        );
    }

    #[test]
    fn tooltip_copy_is_bound_to_each_honest_state() {
        assert_eq!(
            ReleaseStatus::UpToDate.tooltip().1,
            "Click here for the download page, you are seemingly up to date on this build"
        );
        assert_eq!(
            ReleaseStatus::CannotAssess.tooltip().1,
            "open the downloads page for the painter, cannot assess if you are out of date"
        );
        assert_eq!(
            ReleaseStatus::OutOfDate.tooltip().1,
            "opens the downloads page for the latest build, your current build is out of date!"
        );
    }
}
