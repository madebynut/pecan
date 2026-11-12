//! Check GitHub Releases once per launch and point to a newer version.
//!
//! The check is one anonymous GET to the GitHub API. A newer version appears in the reader's
//! status bar and the About window; nothing is downloaded or installed.
use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, ensure};
use futures_lite::AsyncReadExt as _;
use gpui_kit::{
    AnyElement, App, Global, InteractiveElement as _, IntoElement as _, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _,
    component::{ActiveTheme as _, Icon, IconName, h_flex},
    div,
    http_client::{AsyncBody, HttpClient, RequestTimeout, http},
    px,
};
use semver::Version;
use serde::Deserialize;

const RELEASES: &str = "https://api.github.com/repos/madebynut/pecan/releases?per_page=30";
/// Release pages are built from the checked version, never taken from the response.
const RELEASE_PAGES: &str = "https://github.com/madebynut/pecan/releases/tag/";
/// Thirty releases of JSON, including their notes, stay well below this.
const MAX_RESPONSE_BYTES: u64 = 1024 * 1024;
/// The whole request; the shared client already limits connecting to 10 seconds.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
/// Let the first window, its documents and any recovered drafts settle first.
const STARTUP_DELAY: Duration = Duration::from_secs(5);

#[derive(Debug, Deserialize)]
struct Release {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

/// The newest release this launch found, and whether the status bar notice was closed.
#[derive(Debug, Default)]
struct UpdateState {
    available: Option<Version>,
    dismissed: bool,
}
impl Global for UpdateState {}

/// A `v<version>` tag that may be offered: no build metadata and no packaging trial.
fn release_version(release: &Release) -> Option<(Version, bool)> {
    if release.draft || release.tag_name.to_ascii_lowercase().contains("test") {
        return None;
    }
    let version = Version::parse(release.tag_name.strip_prefix('v')?).ok()?;
    if !version.build.is_empty() {
        return None;
    }
    let prerelease = release.prerelease || !version.pre.is_empty();
    Some((version, prerelease))
}

/// The newest release above `current`.
///
/// People on a stable version only hear about stable releases. People on a pre-release
/// also hear about newer pre-releases, and about the final release they lead to.
fn newest(current: &Version, releases: &[Release]) -> Option<Version> {
    releases
        .iter()
        .filter_map(release_version)
        .filter(|(version, prerelease)| {
            version > current && (!prerelease || !current.pre.is_empty())
        })
        .map(|(version, _)| version)
        .max()
}

fn release_url(version: &Version) -> String {
    format!("{RELEASE_PAGES}v{version}")
}

async fn fetch(client: Arc<dyn HttpClient>, url: &str) -> anyhow::Result<Vec<Release>> {
    // The shared client adds only `User-Agent: Pecan/<version>`, which GitHub requires.
    let request = http::Request::builder()
        .uri(url)
        .extension(RequestTimeout(REQUEST_TIMEOUT))
        .body(AsyncBody::empty())?;
    let mut response = client.send(request).await?;
    ensure!(
        response.status().is_success(),
        "GitHub Releases returned HTTP {}",
        response.status()
    );
    let mut body = Vec::new();
    response
        .body_mut()
        .take(MAX_RESPONSE_BYTES + 1)
        .read_to_end(&mut body)
        .await?;
    ensure!(
        body.len() as u64 <= MAX_RESPONSE_BYTES,
        "release list exceeds 1 MiB"
    );
    serde_json::from_slice(&body).context("parsing the release list")
}

fn current() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).expect("Cargo versions are SemVer")
}

fn enabled(cx: &mut App) -> bool {
    crate::preferences::Preferences::shared(cx)
        .read(cx)
        .data
        .check_updates
}

/// Start the launch check, unless the user turned it off. Failures are only logged.
pub(crate) fn check(cx: &mut App) {
    if !enabled(cx) {
        return;
    }
    let client = cx.http_client();
    cx.spawn(async move |cx| {
        cx.background_executor().timer(STARTUP_DELAY).await;
        let current = current();
        match fetch(client, RELEASES).await {
            Ok(releases) => match newest(&current, &releases) {
                Some(version) => cx.update(|cx| found(version, cx)),
                None => tracing::debug!(version = %current, "Pecan is up to date"),
            },
            Err(error) => tracing::debug!(error = %format!("{error:#}"), "update check failed"),
        }
    })
    .detach();
}

fn found(version: Version, cx: &mut App) {
    tracing::info!(%version, "a newer Pecan release is available");
    cx.default_global::<UpdateState>().available = Some(version);
    // Reader rows and the status bar are cached views; redraw every window once.
    cx.refresh_windows();
}

/// The newer version, while checking is on. The About window shows it even after the
/// status bar notice was closed.
pub(crate) fn available(cx: &mut App) -> Option<Version> {
    let version = cx.try_global::<UpdateState>()?.available.clone()?;
    enabled(cx).then_some(version)
}

pub(crate) fn open(version: &Version, cx: &mut App) {
    cx.open_url(&release_url(version));
}

/// Hide the status bar notice until the next launch.
fn dismiss(cx: &mut App) {
    cx.default_global::<UpdateState>().dismissed = true;
    cx.refresh_windows();
}

/// "新版本 X 可用" with a close button, for the reader's status bar.
pub(crate) fn status_notice(cx: &mut App) -> Option<AnyElement> {
    if cx
        .try_global::<UpdateState>()
        .is_some_and(|state| state.dismissed)
    {
        return None;
    }
    let version = available(cx)?;
    let label = format!("新版本 {version} 可用");
    Some(
        h_flex()
            .flex_shrink_0()
            .pl_3()
            .gap_1()
            .items_center()
            .child(
                div()
                    .id("update-notice")
                    .debug_selector(|| "update-notice".into())
                    .cursor_pointer()
                    .text_color(cx.theme().foreground)
                    .hover(|style| style.underline())
                    .child(label)
                    .on_click(move |_, _, cx| open(&version, cx)),
            )
            .child(
                div()
                    .id("update-dismiss")
                    .debug_selector(|| "update-dismiss".into())
                    .cursor_pointer()
                    .rounded(px(3.))
                    .p(px(2.))
                    .hover(|style| style.bg(cx.theme().muted))
                    .child(Icon::new(IconName::Close).size(px(10.)))
                    .on_click(|_, _, cx| {
                        cx.stop_propagation();
                        dismiss(cx);
                    }),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> Release {
        Release {
            tag_name: tag.into(),
            draft: false,
            prerelease,
        }
    }

    fn version(text: &str) -> Version {
        Version::parse(text).unwrap()
    }

    #[test]
    fn stable_users_hear_about_stable_releases_and_pre_release_users_about_both() {
        let releases = [
            release("v1.0.0-beta.1", true),
            release("v1.0.0-beta.2", true),
            release("v1.0.0", false),
            release("v1.1.0-beta.1", true),
        ];
        let pick = |current: &str| newest(&version(current), &releases);
        assert_eq!(pick("1.0.0-beta.1"), Some(version("1.1.0-beta.1")));
        assert_eq!(pick("0.9.0"), Some(version("1.0.0")));
        assert_eq!(
            pick("1.0.0"),
            None,
            "stable users are not offered pre-releases"
        );
        assert_eq!(pick("1.1.0-beta.1"), None);
        let to_final = [release("v1.0.0", false)];
        assert_eq!(
            newest(&version("1.0.0-beta.1"), &to_final),
            Some(version("1.0.0")),
            "pre-release users are offered the final release"
        );
        // GitHub's pre-release flag counts even without a SemVer suffix.
        assert_eq!(newest(&version("1.0.0"), &[release("v1.1.0", true)]), None);
    }

    #[test]
    fn drafts_trials_build_metadata_and_other_tags_are_never_offered() {
        let mut draft = release("v9.0.0", false);
        draft.draft = true;
        for ignored in [
            draft,
            release("v9.0.0-test.1", false),
            release("v9.0.0-Test", false),
            release("build/test-9", false),
            release("v9.0.0+20261112", false),
            release("9.0.0", false),
            release("v9.0", false),
            release("release-9.0.0", false),
        ] {
            let tag = ignored.tag_name.clone();
            assert_eq!(newest(&version("1.0.0-beta.1"), &[ignored]), None, "{tag}");
        }
    }

    #[test]
    fn release_pages_are_built_from_the_checked_version() {
        assert_eq!(
            release_url(&version("1.2.0-beta.3")),
            "https://github.com/madebynut/pecan/releases/tag/v1.2.0-beta.3"
        );
    }

    #[test]
    fn the_request_is_anonymous_and_the_response_is_bounded() {
        use std::{
            io::{Read as _, Write as _},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/releases", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let mut requests = Vec::new();
            for body in [
                r#"[{"tag_name":"v9.0.0","draft":false,"prerelease":false,"body":"notes","html_url":"https://example.com"}]"#.to_owned(),
                "x".repeat(MAX_RESPONSE_BYTES as usize + 1),
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0; 4096];
                let count = stream.read(&mut request).unwrap();
                requests.push(String::from_utf8_lossy(&request[..count]).to_lowercase());
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}"
                )
                .unwrap();
            }
            requests
        });
        let client: Arc<dyn HttpClient> = crate::network::client();
        let releases = futures_lite::future::block_on(fetch(client.clone(), &url)).unwrap();
        assert_eq!(newest(&version("1.0.0"), &releases), Some(version("9.0.0")));
        assert!(futures_lite::future::block_on(fetch(client, &url)).is_err());
        let requests = server.join().unwrap();
        assert!(requests[0].starts_with("get /releases "));
        assert!(
            requests[0].contains("user-agent: pecan/"),
            "GitHub requires a User-Agent"
        );
        for header in ["authorization", "cookie", "x-github-api-version"] {
            assert!(!requests[0].contains(header), "{header}: {}", requests[0]);
        }
    }

    #[gpui_kit::test]
    fn the_status_bar_offers_the_release_until_dismissed_and_about_keeps_it(
        cx: &mut gpui_kit::TestAppContext,
    ) {
        use gpui_kit::{AppContext as _, VisualTestContext};
        let (handle, _) = cx.update(|cx| {
            gpui_kit::init(cx);
            gpui_kit::open_window(Default::default(), cx, |window, cx| {
                cx.new(|cx| {
                    crate::app::Reader::new(
                        None,
                        pecan_config::startup::Launch::Returning,
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(handle, cx);
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(visual.debug_bounds("update-notice").is_none());

        visual.update(|_, cx| found(version("9.0.0"), cx));
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        let notice = visual
            .debug_bounds("update-notice")
            .expect("the status bar names the new version");
        let status = visual.debug_bounds("reader-status").unwrap();
        assert!(status.contains(&notice.center()));
        assert!(
            notice.center().x > status.center().x,
            "the notice sits on the right"
        );

        let close = visual.debug_bounds("update-dismiss").unwrap();
        visual.simulate_click(close.center(), Default::default());
        visual.run_until_parked();
        visual.update(|window, cx| window.draw(cx).clear(cx));
        assert!(
            visual.debug_bounds("update-notice").is_none(),
            "× hides the notice"
        );
        visual.update(|_, cx| {
            assert_eq!(
                available(cx),
                Some(version("9.0.0")),
                "About still offers it"
            );
            crate::preferences::Preferences::shared(cx).update(cx, |prefs, cx| {
                prefs.change(|data| data.check_updates = false, cx)
            });
            assert_eq!(available(cx), None, "turning checks off hides every notice");
        });
    }
}
