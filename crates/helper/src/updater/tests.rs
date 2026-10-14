use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn asset(version: &str, kind: &str) -> VelopackAsset {
    VelopackAsset {
        PackageId: PACKAGE_ID.into(),
        Version: version.into(),
        Type: kind.into(),
        FileName: format!("{PACKAGE_ID}-{version}-{}.nupkg", kind.to_ascii_lowercase()),
        ..Default::default()
    }
}

fn feed(assets: Vec<VelopackAsset>) -> VelopackAssetFeed {
    VelopackAssetFeed { Assets: assets }
}

fn versions(feed: &VelopackAssetFeed) -> Vec<String> {
    feed.Assets
        .iter()
        .map(|a| format!("{} {}", a.Version, a.Type))
        .collect()
}

fn not_found() -> velopack::Error {
    ureq::Error::StatusCode(404).into()
}

fn offline() -> velopack::Error {
    ureq::Error::Io(std::io::Error::new(
        std::io::ErrorKind::ConnectionRefused,
        "refused",
    ))
    .into()
}

fn manifest() -> Manifest {
    Manifest::default()
}

#[derive(Clone)]
enum Reply {
    Feed(Vec<VelopackAsset>),
    NotFound,
    Offline,
}

/// An in-memory feed: one folder of the update site.
struct FakeFeed {
    reply: Reply,
    feed_calls: AtomicUsize,
    downloads: Mutex<Vec<String>>,
}

#[derive(Clone)]
struct Fake(Arc<FakeFeed>);

impl Fake {
    fn new(reply: Reply) -> Fake {
        Fake(Arc::new(FakeFeed {
            reply,
            feed_calls: AtomicUsize::new(0),
            downloads: Mutex::new(Vec::new()),
        }))
    }
    fn calls(&self) -> usize {
        self.0.feed_calls.load(Ordering::SeqCst)
    }
}

impl UpdateSource for Fake {
    fn get_release_feed(
        &self,
        _channel: &str,
        _app: &Manifest,
        _staged: &str,
    ) -> Result<VelopackAssetFeed, velopack::Error> {
        self.0.feed_calls.fetch_add(1, Ordering::SeqCst);
        match &self.0.reply {
            Reply::Feed(a) => Ok(feed(a.clone())),
            Reply::NotFound => Err(not_found()),
            Reply::Offline => Err(offline()),
        }
    }

    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        _local: &Path,
        _progress: Option<Sender<i16>>,
    ) -> Result<(), velopack::Error> {
        self.0
            .downloads
            .lock()
            .unwrap()
            .push(asset.FileName.clone());
        match &self.0.reply {
            Reply::Offline => Err(offline()),
            _ => Ok(()),
        }
    }
}

fn source(channel: Channel, stable: Reply, beta: Reply) -> (ChannelSource<Fake>, Fake, Fake, Fake) {
    let (s, b, p) = (
        Fake::new(stable),
        Fake::new(beta),
        Fake::new(Reply::Feed(vec![])),
    );
    (
        ChannelSource::new(channel, PACKAGE_ID, s.clone(), b.clone(), p.clone()),
        s,
        b,
        p,
    )
}

// ---------------------------------------------------------------------------
// Channels, versions, URLs
// ---------------------------------------------------------------------------

#[test]
fn channel_follows_the_installed_version() {
    let v = |s| semver::Version::parse(s).unwrap();
    assert_eq!(Channel::for_version(&v("0.1.0")), Channel::Stable);
    assert_eq!(Channel::for_version(&v("1.2.3+build.5")), Channel::Stable);
    assert_eq!(Channel::for_version(&v("0.2.0-beta.1")), Channel::Beta);
    assert_eq!(Channel::for_version(&v("0.2.0-rc-2")), Channel::Beta);
}

#[test]
fn channel_parses_its_names_only() {
    assert_eq!(Channel::parse("stable"), Some(Channel::Stable));
    assert_eq!(Channel::parse(" BETA "), Some(Channel::Beta));
    assert_eq!(Channel::parse("nightly"), None);
    assert_eq!(Channel::parse(""), None);
}

#[test]
fn urls_are_built_under_one_base() {
    let urls = FeedUrls::new("https://get.faulk.app/helper/");
    assert_eq!(
        urls.feed_url(Channel::Stable, "win"),
        "https://get.faulk.app/helper/stable/releases.win.json"
    );
    assert_eq!(
        urls.feed_url(Channel::Beta, "linux"),
        "https://get.faulk.app/helper/beta/releases.linux.json"
    );
    assert_eq!(urls.packages_dir(), "https://get.faulk.app/helper/packages");
    assert_eq!(
        urls.package_url("FaulkHelper-0.2.0-full.nupkg").as_deref(),
        Some("https://get.faulk.app/helper/packages/FaulkHelper-0.2.0-full.nupkg")
    );
    assert_eq!(urls.package_url("../secrets"), None);
    assert_eq!(urls.package_url("a/b.nupkg"), None);
}

#[test]
fn the_default_base_is_get_faulk_app() {
    assert_eq!(DEFAULT_BASE_URL, "https://get.faulk.app/helper");
    let (config, warnings) = Config::from_values(None, None);
    assert_eq!(config.base_url, DEFAULT_BASE_URL);
    assert_eq!(config.channel, None);
    assert_eq!(config.interval, Duration::from_secs(6 * 3600));
    assert!(warnings.is_empty());
}

#[test]
fn base_override_accepts_https_and_loopback_http_only() {
    for ok in [
        "https://updates.example.org/helper",
        "http://127.0.0.1:8000",
        "http://localhost:8000/x",
        "http://[::1]:9000",
    ] {
        assert_eq!(resolve_base_url(Some(ok)), (ok.to_owned(), None), "{ok}");
    }
    for refused in [
        "http://evil.example.org",
        "http://192.168.1.10:8000",
        "ftp://127.0.0.1",
        "file:///C:/feed",
        "not a url",
        "https://updates.example.org/helper?token=x",
        "https://updates.example.org/helper#stable",
        "http://127.0.0.1:8000/?x=1",
    ] {
        let (base, warning) = resolve_base_url(Some(refused));
        assert_eq!(base, DEFAULT_BASE_URL, "{refused}");
        assert!(warning.unwrap().contains(BASE_URL_ENV), "{refused}");
    }
    assert_eq!(
        resolve_base_url(Some("  ")),
        (DEFAULT_BASE_URL.into(), None)
    );
}

#[test]
fn config_reports_refused_channel() {
    let (config, warnings) = Config::from_values(Some("http://127.0.0.1:1"), Some("beta"));
    assert_eq!(config.base_url, "http://127.0.0.1:1");
    assert_eq!(config.channel, Some(Channel::Beta));
    assert!(warnings.is_empty());
    let (config, warnings) = Config::from_values(None, Some("nightly"));
    assert_eq!(config.channel, None);
    assert_eq!(warnings.len(), 1);
}

#[test]
fn safe_file_names() {
    for ok in [
        "FaulkHelper-0.2.0-full.nupkg",
        "FaulkHelper-0.2.0-beta.1-delta.nupkg",
        "FaulkHelper-1.0.0+meta-full.nupkg",
    ] {
        assert!(is_safe_file_name(ok), "{ok}");
    }
    for bad in [
        "",
        ".",
        "..",
        "../x.nupkg",
        "a/b.nupkg",
        "a\\b.nupkg",
        "C:x.nupkg",
        "x?y=1",
        &"a".repeat(201),
    ] {
        assert!(!is_safe_file_name(bad), "{bad}");
    }
}

// ---------------------------------------------------------------------------
// Feed filtering
// ---------------------------------------------------------------------------

#[test]
fn stable_never_keeps_a_pre_release() {
    let f = feed(vec![
        asset("0.1.1", "Full"),
        asset("0.2.0-beta.1", "Full"),
        asset("0.2.0-beta.1", "Delta"),
        asset("0.3.0-rc.1", "Full"),
    ]);
    assert_eq!(
        versions(&filter_feed(Channel::Stable, PACKAGE_ID, f.clone())),
        ["0.1.1 Full"]
    );
    // Beta keeps pre-releases but only full packages (see `filter_feed`).
    assert_eq!(
        versions(&filter_feed(Channel::Beta, PACKAGE_ID, f)),
        ["0.1.1 Full", "0.2.0-beta.1 Full", "0.3.0-rc.1 Full"]
    );
}

#[test]
fn stable_keeps_its_delta_packages() {
    let f = feed(vec![asset("0.1.1", "Full"), asset("0.1.1", "Delta")]);
    assert_eq!(
        versions(&filter_feed(Channel::Stable, PACKAGE_ID, f)),
        ["0.1.1 Full", "0.1.1 Delta"]
    );
}

#[test]
fn filter_drops_foreign_unsafe_and_unparsable_assets() {
    let mut foreign = asset("9.0.0", "Full");
    foreign.PackageId = "Other".into();
    let mut traversal = asset("9.0.1", "Full");
    traversal.FileName = "../../evil.nupkg".into();
    let mut garbage = asset("1.0.0", "Full");
    garbage.Version = "latest".into();
    let f = feed(vec![foreign, traversal, garbage, asset("0.1.1", "Full")]);
    for channel in [Channel::Stable, Channel::Beta] {
        assert_eq!(
            versions(&filter_feed(channel, PACKAGE_ID, f.clone())),
            ["0.1.1 Full"]
        );
    }
}

#[test]
fn merge_keeps_each_file_once() {
    let merged = merge_feeds(
        feed(vec![asset("0.1.1", "Full"), asset("0.1.1", "Delta")]),
        feed(vec![asset("0.1.1", "Full"), asset("0.2.0-beta.1", "Full")]),
    );
    assert_eq!(
        versions(&merged),
        ["0.1.1 Full", "0.1.1 Delta", "0.2.0-beta.1 Full"]
    );
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[test]
fn errors_are_classified() {
    assert_eq!(classify(&not_found()), UpdateError::FeedNotFound);
    assert!(matches!(classify(&offline()), UpdateError::Network(_)));
    assert!(matches!(
        classify(&ureq::Error::StatusCode(503).into()),
        UpdateError::Network(_)
    ));
    let json: velopack::Error = serde_json::from_str::<serde_json::Value>("<html>")
        .unwrap_err()
        .into();
    assert!(matches!(classify(&json), UpdateError::InvalidFeed(_)));
    let checksum = velopack::Error::ChecksumInvalid("p".into(), "a".into(), "b".into());
    assert!(matches!(classify(&checksum), UpdateError::Corrupt(_)));
    assert!(matches!(
        classify(&velopack::Error::NotInstalled("x".into())),
        UpdateError::Other(_)
    ));
}

const CRLF: &str = "\r\n";

/// A one-shot HTTP server on loopback that answers every request with `status`.
fn serve_status(status: u16) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming().take(4) {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X{CRLF}Content-Length: 0{CRLF}Connection: close{CRLF}{CRLF}"
            );
        }
    });
    format!("http://{addr}")
}

#[test]
fn a_real_404_is_a_missing_feed() {
    let base = serve_status(404);
    let err = HttpSource::new(format!("{base}/stable"))
        .get_release_feed("win", &manifest(), "")
        .unwrap_err();
    assert_eq!(classify(&err), UpdateError::FeedNotFound);
}

#[test]
fn a_package_404_is_not_a_missing_feed() {
    let base = serve_status(404);
    let source = ChannelSource::http(Channel::Stable, &FeedUrls::new(&base));
    let dir = std::env::temp_dir().join(format!("faulk-pkg-404-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = source
        .download_release_entry(&asset("0.2.0", "Full"), &dir.join("p.nupkg"), None)
        .unwrap_err();
    let error = classify(&err);
    assert!(
        matches!(&error, UpdateError::PackageNotFound(name) if name.ends_with(".nupkg")),
        "{error:?}"
    );
    assert!(error.to_string().contains("not found"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_real_server_error_or_closed_port_is_a_network_error() {
    let base = serve_status(500);
    let err = HttpSource::new(format!("{base}/stable"))
        .get_release_feed("win", &manifest(), "")
        .unwrap_err();
    assert!(matches!(classify(&err), UpdateError::Network(_)), "{err}");
    // A port nobody listens on.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let err = HttpSource::new(format!("http://127.0.0.1:{port}/stable"))
        .get_release_feed("win", &manifest(), "")
        .unwrap_err();
    assert!(matches!(classify(&err), UpdateError::Network(_)), "{err}");
}

// ---------------------------------------------------------------------------
// Instance lock
// ---------------------------------------------------------------------------

fn lock_path(name: &str) -> PathBuf {
    std::env::temp_dir()
        .join(format!("faulk-lock-test-{}-{name}", std::process::id()))
        .join("helper.lock")
}

// Each `InstanceLock` opens its own handle, which the OS treats as another process would be.

const QUICK: Duration = Duration::from_millis(200);

/// A started helper: the lock taken, then shared once the startup decision is made.
fn started(path: &Path) -> InstanceLock {
    let lock = InstanceLock::take(path, QUICK).unwrap();
    lock.share();
    lock
}

#[test]
fn only_the_first_helper_is_alone() {
    let path = lock_path("first");
    let a = InstanceLock::take(&path, QUICK).unwrap();
    assert!(a.alone());
    a.share();
    let b = InstanceLock::take(&path, QUICK).unwrap();
    assert!(!b.alone());
    drop(a);
    drop(b);
    assert!(InstanceLock::take(&path, QUICK).unwrap().alone());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn two_helpers_starting_together_are_never_both_alone() {
    let path = lock_path("together");
    for _ in 0..20 {
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let spawn = |path: PathBuf, barrier: Arc<std::sync::Barrier>| {
            std::thread::spawn(move || {
                barrier.wait();
                let lock = InstanceLock::take(&path, Duration::from_secs(5)).unwrap();
                let alone = lock.alone();
                // The startup decision takes a moment, then the lock is shared.
                std::thread::sleep(Duration::from_millis(5));
                lock.share();
                (lock, alone)
            })
        };
        let x = spawn(path.clone(), barrier.clone());
        let y = spawn(path.clone(), barrier);
        let (_x, x_alone) = x.join().unwrap();
        let (_y, y_alone) = y.join().unwrap();
        assert!(!(x_alone && y_alone));
        assert!(x_alone || y_alone);
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn a_helper_starting_while_another_decides_waits_for_it() {
    let path = lock_path("wait");
    let a = Arc::new(InstanceLock::take(&path, QUICK).unwrap());
    assert!(a.alone());
    // A keeps the lock exclusive (deciding whether to apply), then shares it.
    let sharer = {
        let a = a.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            a.share();
        })
    };
    let start = Instant::now();
    let b = InstanceLock::take(&path, Duration::from_secs(5)).unwrap();
    assert!(start.elapsed() >= Duration::from_millis(50));
    assert!(!b.alone());
    sharer.join().unwrap();
    // B now holds the lock: A is not alone any more.
    assert!(!a.claim_alone().unwrap());
    // Out of patience, taking fails: a helper never runs as if it held the lock.
    let a2 = InstanceLock::take(&lock_path("wait2"), QUICK).unwrap();
    let late = InstanceLock::take(&lock_path("wait2"), QUICK);
    assert_eq!(
        late.err().map(|e| e.kind()),
        Some(std::io::ErrorKind::TimedOut)
    );
    drop(a2);
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
    let _ = std::fs::remove_dir_all(lock_path("wait2").parent().unwrap());
}

#[test]
fn a_helper_started_while_another_runs_is_not_alone_after_the_first_quits() {
    // A starts, B starts next to it, A quits: C must still see B.
    let path = lock_path("order");
    let a = started(&path);
    let b = started(&path);
    drop(a);
    let c = InstanceLock::take(&path, QUICK).unwrap();
    assert!(!c.alone());
    assert!(!c.claim_alone().unwrap());
    drop(b);
    assert!(c.claim_alone().unwrap());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn claiming_keeps_others_out_until_shared_again() {
    let path = lock_path("claim");
    let a = started(&path);
    assert!(a.claim_alone().unwrap());
    // A helper starting while A applies waits for it.
    assert!(InstanceLock::take(&path, QUICK).is_err());
    a.share();
    let b = started(&path);
    // A's failed claim keeps its shared hold: B is still not alone.
    assert!(!a.claim_alone().unwrap());
    assert!(!b.claim_alone().unwrap());
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn the_logger_keeps_velopack_warnings_only() {
    let meta =
        |target: &'static str, level| log::Metadata::builder().target(target).level(level).build();
    let logger = JournalLog;
    use log::Log;
    assert!(logger.enabled(&meta("velopack::manager", log::Level::Warn)));
    assert!(logger.enabled(&meta("velopack", log::Level::Error)));
    assert!(!logger.enabled(&meta("velopack::manager", log::Level::Info)));
    assert!(!logger.enabled(&meta("tungstenite::protocol", log::Level::Warn)));
    assert!(!logger.enabled(&meta("midir", log::Level::Error)));
}

// ---------------------------------------------------------------------------
// ChannelSource
// ---------------------------------------------------------------------------

#[test]
fn stable_source_reads_the_stable_feed_only() {
    let (src, stable, beta, _) = source(
        Channel::Stable,
        Reply::Feed(vec![asset("0.1.1", "Full"), asset("0.2.0-beta.1", "Full")]),
        Reply::Feed(vec![asset("0.3.0-beta.1", "Full")]),
    );
    let f = src.get_release_feed("win", &manifest(), "").unwrap();
    assert_eq!(versions(&f), ["0.1.1 Full"]);
    assert_eq!((stable.calls(), beta.calls()), (1, 0));
}

#[test]
fn stable_source_fails_when_its_feed_fails() {
    for reply in [Reply::NotFound, Reply::Offline] {
        let (src, ..) = source(Channel::Stable, reply, Reply::Feed(vec![]));
        assert!(src.get_release_feed("win", &manifest(), "").is_err());
    }
}

#[test]
fn beta_source_merges_both_feeds() {
    let (src, stable, beta, _) = source(
        Channel::Beta,
        Reply::Feed(vec![asset("0.2.0", "Full")]),
        Reply::Feed(vec![asset("0.2.0-beta.3", "Full")]),
    );
    let f = src.get_release_feed("win", &manifest(), "").unwrap();
    assert_eq!(versions(&f), ["0.2.0 Full", "0.2.0-beta.3 Full"]);
    assert_eq!((stable.calls(), beta.calls()), (1, 1));
}

#[test]
fn beta_source_survives_one_missing_feed() {
    let (src, ..) = source(
        Channel::Beta,
        Reply::Feed(vec![asset("0.1.1", "Full")]),
        Reply::NotFound,
    );
    assert_eq!(
        versions(&src.get_release_feed("win", &manifest(), "").unwrap()),
        ["0.1.1 Full"]
    );
    let (src, ..) = source(
        Channel::Beta,
        Reply::Offline,
        Reply::Feed(vec![asset("0.2.0-beta.1", "Full")]),
    );
    assert_eq!(
        versions(&src.get_release_feed("win", &manifest(), "").unwrap()),
        ["0.2.0-beta.1 Full"]
    );
}

#[test]
fn beta_source_fails_when_both_feeds_fail() {
    let (src, ..) = source(Channel::Beta, Reply::Offline, Reply::NotFound);
    let err = src.get_release_feed("win", &manifest(), "").unwrap_err();
    assert!(matches!(classify(&err), UpdateError::Network(_)));
}

#[test]
fn downloads_go_to_the_packages_source_and_refuse_unsafe_names() {
    let (src, stable, beta, packages) =
        source(Channel::Stable, Reply::Feed(vec![]), Reply::Feed(vec![]));
    let dir = std::env::temp_dir();
    src.download_release_entry(&asset("0.1.1", "Full"), &dir.join("x"), None)
        .unwrap();
    let mut evil = asset("0.1.2", "Full");
    evil.FileName = "../evil.nupkg".into();
    let err = src
        .download_release_entry(&evil, &dir.join("y"), None)
        .unwrap_err();
    assert!(matches!(classify(&err), UpdateError::InvalidFeed(_)));
    assert_eq!(
        *packages.0.downloads.lock().unwrap(),
        ["FaulkHelper-0.1.1-full.nupkg"]
    );
    assert!(stable.0.downloads.lock().unwrap().is_empty());
    assert!(beta.0.downloads.lock().unwrap().is_empty());
}

// ---------------------------------------------------------------------------
// The real Velopack UpdateManager on a fake installation and a fake feed
// ---------------------------------------------------------------------------

/// A fake installed helper in a temporary folder: manifest, Update.exe stub, packages folder.
struct FakeInstall {
    root: std::path::PathBuf,
}

impl FakeInstall {
    fn new(version: &str) -> FakeInstall {
        static N: AtomicUsize = AtomicUsize::new(0);
        let root = std::env::temp_dir().join(format!(
            "faulk-updater-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let current = root.join("current");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::create_dir_all(root.join("packages")).unwrap();
        std::fs::write(root.join("Update.exe"), b"stub").unwrap();
        std::fs::write(
            current.join("sq.version"),
            format!(
                r#"<?xml version="1.0" encoding="utf-8"?>
<package><metadata><id>{PACKAGE_ID}</id><version>{version}</version><channel>win</channel><mainExe>faulk-helper.exe</mainExe></metadata></package>"#
            ),
        )
        .unwrap();
        FakeInstall { root }
    }

    fn locator(&self) -> velopack::locator::VelopackLocatorConfig {
        velopack::locator::VelopackLocatorConfig {
            RootAppDir: self.root.clone(),
            UpdateExePath: self.root.join("Update.exe"),
            PackagesDir: self.root.join("packages"),
            ManifestPath: self.root.join("current").join("sq.version"),
            CurrentBinaryDir: self.root.join("current"),
            IsPortable: false,
        }
    }

    fn check(&self, channel: Channel, stable: Reply, beta: Reply) -> Option<String> {
        let (src, ..) = source(channel, stable, beta);
        let manager = UpdateManager::new(src, None, Some(self.locator())).unwrap();
        manager
            .check()
            .unwrap()
            .map(|u| u.TargetFullRelease.Version)
    }
}

impl Drop for FakeInstall {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[test]
fn velopack_offers_the_newest_stable_to_a_stable_install() {
    let install = FakeInstall::new("0.1.0");
    let stable = Reply::Feed(vec![
        asset("0.1.0", "Full"),
        asset("0.1.1", "Full"),
        // A pre-release that reached the stable feed by mistake.
        asset("0.2.0-beta.1", "Full"),
    ]);
    let beta = Reply::Feed(vec![asset("0.3.0-beta.1", "Full")]);
    assert_eq!(
        install.check(Channel::Stable, stable.clone(), beta.clone()),
        Some("0.1.1".into())
    );
    // The same install switched to beta sees the newest pre-release.
    assert_eq!(
        install.check(Channel::Beta, stable, beta),
        Some("0.3.0-beta.1".into())
    );
}

#[test]
fn velopack_offers_nothing_newer_than_a_pre_release_to_stable() {
    let install = FakeInstall::new("0.1.1");
    let only_pre = Reply::Feed(vec![asset("0.1.1", "Full"), asset("0.2.0-beta.1", "Full")]);
    assert_eq!(
        install.check(Channel::Stable, only_pre, Reply::Feed(vec![])),
        None
    );
    // Empty feed: nothing either.
    assert_eq!(
        install.check(Channel::Stable, Reply::Feed(vec![]), Reply::Feed(vec![])),
        None
    );
}

#[test]
fn a_beta_install_gets_a_newer_stable() {
    let install = FakeInstall::new("0.2.0-beta.2");
    assert_eq!(
        install.check(
            Channel::Beta,
            Reply::Feed(vec![asset("0.2.0", "Full")]),
            Reply::Feed(vec![asset("0.2.0-beta.2", "Full")]),
        ),
        Some("0.2.0".into())
    );
}

#[test]
fn velopack_check_reports_a_missing_feed() {
    let install = FakeInstall::new("0.1.0");
    let (src, ..) = source(Channel::Stable, Reply::NotFound, Reply::NotFound);
    let manager = UpdateManager::new(src, None, Some(install.locator())).unwrap();
    let err = Backend::check(&manager).err().unwrap();
    assert_eq!(classify(&err), UpdateError::FeedNotFound);
}

// ---------------------------------------------------------------------------
// The update thread, on a fake backend
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FakeBackend {
    /// `None`: up to date. `Some(v)`: version `v` available.
    available: Mutex<Option<String>>,
    check_fails: Mutex<bool>,
    download_fails: Mutex<bool>,
    /// Holds the download until the test sends on it.
    gate: Mutex<Option<Receiver<()>>>,
    pending: Mutex<Option<String>>,
    checks: AtomicUsize,
    applied: Mutex<Vec<(String, Vec<String>)>>,
}

impl Backend for Arc<FakeBackend> {
    fn current_version(&self) -> String {
        "0.1.0".into()
    }

    fn check(&self) -> Result<Option<UpdateInfo>, velopack::Error> {
        self.checks.fetch_add(1, Ordering::SeqCst);
        if *self.check_fails.lock().unwrap() {
            return Err(offline());
        }
        Ok(self.available.lock().unwrap().clone().map(|v| UpdateInfo {
            TargetFullRelease: asset(&v, "Full"),
            ..Default::default()
        }))
    }

    fn download(&self, update: &UpdateInfo, progress: Sender<i16>) -> Result<(), velopack::Error> {
        let _ = progress.send(-5);
        let _ = progress.send(40);
        if let Some(gate) = self.gate.lock().unwrap().take() {
            let _ = gate.recv();
        }
        let _ = progress.send(150);
        if *self.download_fails.lock().unwrap() {
            return Err(offline());
        }
        *self.pending.lock().unwrap() = Some(update.TargetFullRelease.Version.clone());
        Ok(())
    }

    fn pending(&self) -> Option<VelopackAsset> {
        self.pending
            .lock()
            .unwrap()
            .clone()
            .map(|v| asset(&v, "Full"))
    }

    fn apply_and_restart(
        &self,
        asset: &VelopackAsset,
        args: &[String],
    ) -> Result<(), velopack::Error> {
        self.applied
            .lock()
            .unwrap()
            .push((asset.Version.clone(), args.to_vec()));
        Ok(())
    }
}

fn wait_for(updater: &Updater, what: impl Fn(&UpdateStatus) -> bool) -> UpdateStatus {
    let start = Instant::now();
    loop {
        let s = updater.status();
        if what(&s) {
            return s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(10),
            "status stuck at {s:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

const NEVER: Duration = Duration::from_secs(3600);

#[test]
fn check_at_start_then_download_then_ready() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let updater = Updater::start_with(backend.clone(), NEVER);
    let s = wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    assert_eq!(
        s,
        UpdateStatus::Ready {
            version: "0.2.0".into()
        }
    );
    assert_eq!(backend.checks.load(Ordering::SeqCst), 1);
    updater.shutdown();
}

#[test]
fn up_to_date_reports_the_installed_version() {
    let backend = Arc::new(FakeBackend::default());
    let updater = Updater::start_with(backend, NEVER);
    let s = wait_for(&updater, |s| matches!(s, UpdateStatus::UpToDate { .. }));
    assert_eq!(
        s,
        UpdateStatus::UpToDate {
            version: "0.1.0".into()
        }
    );
    updater.shutdown();
}

#[test]
fn download_progress_is_clamped_and_shown() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    let updater = Updater::start_with(backend, NEVER);
    let s = wait_for(&updater, |s| {
        matches!(s, UpdateStatus::Downloading { percent: 40, .. })
    });
    assert_eq!(
        s,
        UpdateStatus::Downloading {
            version: "0.2.0".into(),
            percent: 40
        }
    );
    open.send(()).unwrap();
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    updater.shutdown();
}

#[test]
fn a_network_failure_is_reported_and_the_next_check_retries() {
    let backend = Arc::new(FakeBackend::default());
    *backend.check_fails.lock().unwrap() = true;
    let updater = Updater::start_with(backend.clone(), NEVER);
    let s = wait_for(&updater, |s| matches!(s, UpdateStatus::Failed { .. }));
    assert!(matches!(
        s,
        UpdateStatus::Failed {
            error: UpdateError::Network(_)
        }
    ));
    // The network comes back; "Check for updates" retries.
    *backend.check_fails.lock().unwrap() = false;
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    assert!(updater.check_now());
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    assert_eq!(backend.checks.load(Ordering::SeqCst), 2);
    updater.shutdown();
}

#[test]
fn a_failed_download_is_reported_without_pending_package() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    *backend.download_fails.lock().unwrap() = true;
    let updater = Updater::start_with(backend, NEVER);
    let s = wait_for(&updater, |s| matches!(s, UpdateStatus::Failed { .. }));
    assert!(matches!(
        s,
        UpdateStatus::Failed {
            error: UpdateError::Network(_)
        }
    ));
    updater.shutdown();
}

#[test]
fn a_ready_update_survives_a_later_failure() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    // Offline at the next check: the downloaded 0.2.0 is still applicable.
    *backend.check_fails.lock().unwrap() = true;
    updater.check_now();
    wait_for(&updater, |_| backend.checks.load(Ordering::SeqCst) == 2);
    let s = wait_for(&updater, |s| !matches!(s, UpdateStatus::Checking));
    assert_eq!(
        s,
        UpdateStatus::Ready {
            version: "0.2.0".into()
        }
    );
    // A newer version whose download fails: still the 0.2.0 on disk.
    *backend.check_fails.lock().unwrap() = false;
    *backend.available.lock().unwrap() = Some("0.3.0".into());
    *backend.download_fails.lock().unwrap() = true;
    updater.check_now();
    wait_for(&updater, |_| backend.checks.load(Ordering::SeqCst) == 3);
    let s = wait_for(&updater, |s| {
        !matches!(s, UpdateStatus::Checking | UpdateStatus::Downloading { .. })
    });
    assert_eq!(
        s,
        UpdateStatus::Ready {
            version: "0.2.0".into()
        }
    );
    updater.shutdown();
}

#[test]
fn the_same_ready_version_is_not_downloaded_twice() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    // A second download would fail; it must not happen.
    *backend.download_fails.lock().unwrap() = true;
    updater.check_now();
    wait_for(&updater, |_| backend.checks.load(Ordering::SeqCst) == 2);
    let s = wait_for(&updater, |s| !matches!(s, UpdateStatus::Checking));
    assert_eq!(
        s,
        UpdateStatus::Ready {
            version: "0.2.0".into()
        }
    );
    updater.shutdown();
}

#[test]
fn checks_repeat_on_the_interval() {
    let backend = Arc::new(FakeBackend::default());
    let updater = Updater::start_with(backend.clone(), Duration::from_millis(10));
    let start = Instant::now();
    while backend.checks.load(Ordering::SeqCst) < 3 {
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(5));
    }
    updater.shutdown();
}

#[test]
fn repeated_check_requests_coalesce_and_never_block() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::Downloading { .. }));
    let start = Instant::now();
    for _ in 0..100 {
        assert!(updater.check_now());
    }
    assert!(start.elapsed() < Duration::from_secs(1));
    open.send(()).unwrap();
    wait_for(&updater, |_| backend.checks.load(Ordering::SeqCst) == 2);
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    // One queued request at most: exactly one more check.
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(backend.checks.load(Ordering::SeqCst), 2);
    updater.shutdown();
}

#[test]
fn shutdown_stops_the_thread() {
    let backend = Arc::new(FakeBackend::default());
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::UpToDate { .. }));
    updater.shutdown();
    // Dropping a running updater does not block either.
    let updater = Updater::start_with(backend, NEVER);
    let start = Instant::now();
    drop(updater);
    assert!(start.elapsed() < Duration::from_secs(1));
}

#[test]
fn apply_needs_a_downloaded_update_and_keeps_the_arguments() {
    let backend = Arc::new(FakeBackend::default());
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::UpToDate { .. }));
    assert!(updater.apply_and_restart().is_err());
    assert!(backend.applied.lock().unwrap().is_empty());

    *backend.available.lock().unwrap() = Some("0.2.0".into());
    updater.check_now();
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    updater.apply_and_restart().unwrap();
    let applied = backend.applied.lock().unwrap().clone();
    let args: Vec<String> = std::env::args().skip(1).collect();
    assert_eq!(applied, [("0.2.0".to_owned(), args)]);
    updater.shutdown();
}

#[test]
fn apply_is_refused_while_another_helper_runs() {
    let path = lock_path("apply");
    let mine = started(&path);
    let other = started(&path);
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let updater = Updater::start_with(backend.clone(), NEVER).with_instance_lock(Arc::new(mine));
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    assert!(updater.apply_and_restart().is_err());
    assert!(backend.applied.lock().unwrap().is_empty());
    // The other helper is still counted after the refusal.
    let probe = InstanceLock::take(&path, QUICK).unwrap();
    assert!(!probe.alone());
    drop(probe);
    drop(other);
    updater.apply_and_restart().unwrap();
    assert_eq!(backend.applied.lock().unwrap().len(), 1);
    // The apply returned (a failed apply would too): the hold is shared again.
    let probe = InstanceLock::take(&path, QUICK).unwrap();
    assert!(!probe.alone());
    updater.shutdown();
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn apply_is_refused_while_a_download_runs() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    // A newer version appears: its download holds the packages folder.
    *backend.available.lock().unwrap() = Some("0.3.0".into());
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    updater.check_now();
    wait_for(&updater, |s| matches!(s, UpdateStatus::Downloading { .. }));
    assert!(updater.apply_and_restart().is_err());
    assert!(backend.applied.lock().unwrap().is_empty());
    open.send(()).unwrap();
    wait_for(
        &updater,
        |s| matches!(s, UpdateStatus::Ready { version } if version == "0.3.0"),
    );
    updater.apply_and_restart().unwrap();
    assert_eq!(backend.applied.lock().unwrap()[0].0, "0.3.0");
    updater.shutdown();
}

#[test]
fn a_ready_update_whose_package_vanished_is_downloaded_again() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    // The package is cleaned up; the same version is still offered.
    *backend.pending.lock().unwrap() = None;
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    updater.check_now();
    wait_for(&updater, |s| matches!(s, UpdateStatus::Downloading { .. }));
    open.send(()).unwrap();
    wait_for(&updater, |s| matches!(s, UpdateStatus::Ready { .. }));
    assert!(backend.pending().is_some());
    updater.apply_and_restart().unwrap();
    updater.shutdown();
}

#[test]
fn shutdown_skips_a_queued_check() {
    let backend = Arc::new(FakeBackend::default());
    *backend.available.lock().unwrap() = Some("0.2.0".into());
    let (open, gate) = mpsc::channel();
    *backend.gate.lock().unwrap() = Some(gate);
    let updater = Updater::start_with(backend.clone(), NEVER);
    wait_for(&updater, |s| matches!(s, UpdateStatus::Downloading { .. }));
    assert!(updater.check_now());
    let opener = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(50));
        let _ = open.send(());
    });
    updater.shutdown();
    opener.join().unwrap();
    assert_eq!(backend.checks.load(Ordering::SeqCst), 1);
}

#[test]
fn a_disabled_updater_does_nothing() {
    let updater = Updater::disabled("not installed".into());
    assert_eq!(
        updater.status(),
        UpdateStatus::Disabled {
            reason: "not installed".into()
        }
    );
    assert!(!updater.check_now());
    assert!(updater.apply_and_restart().is_err());
    updater.shutdown();
}

#[test]
fn start_outside_an_installation_is_disabled_and_offline() {
    // `cargo test` runs from target/, never from a Velopack installation.
    let updater = Updater::start(Config::from_values(Some("http://127.0.0.1:9"), None).0);
    assert!(matches!(updater.status(), UpdateStatus::Disabled { .. }));
    assert!(!updater.check_now());
}

// ---------------------------------------------------------------------------
// The redirects of get.faulk.app (web/vercel.json) match what the updater asks for
// ---------------------------------------------------------------------------

struct Rule {
    source_text: String,
    source: regex::Regex,
    names: Vec<String>,
    destination: String,
}

/// Translates the subset of path-to-regexp used in `web/vercel.json`: literal text and
/// `:name(regex)` parameters.
fn compile(source: &str) -> (regex::Regex, Vec<String>) {
    let mut pattern = String::from("^");
    let mut names = Vec::new();
    let mut rest = source;
    while let Some(i) = rest.find(':') {
        pattern.push_str(&literal(&rest[..i]));
        let after = &rest[i + 1..];
        let open = after.find('(').expect("parameter without a pattern");
        names.push(after[..open].to_owned());
        let mut depth = 0;
        let mut end = 0;
        for (j, c) in after[open..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = open + j;
                        break;
                    }
                }
                _ => {}
            }
        }
        pattern.push_str(&format!("({})", &after[open + 1..end]));
        rest = &after[end + 1..];
    }
    pattern.push_str(&literal(rest));
    pattern.push('$');
    (regex::Regex::new(&pattern).unwrap(), names)
}

/// Literal text of a source, where a backslash escapes the next character.
fn literal(text: &str) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        let c = if c == '\\' { chars.next().unwrap() } else { c };
        out.push_str(&regex::escape(&c.to_string()));
    }
    out
}

fn rules() -> Vec<Rule> {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/vercel.json");
    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    json["redirects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| {
            assert_eq!(
                r["has"],
                serde_json::json!([{ "type": "host", "value": "get.faulk.app" }]),
                "every redirect is limited to get.faulk.app"
            );
            assert_eq!(r["permanent"], false, "redirects stay temporary");
            let (source, names) = compile(r["source"].as_str().unwrap());
            Rule {
                source_text: r["source"].as_str().unwrap().to_owned(),
                source,
                names,
                destination: r["destination"].as_str().unwrap().to_owned(),
            }
        })
        .collect()
}

/// Where get.faulk.app sends a URL: the first matching rule, as Vercel applies them.
fn redirect(url: &str) -> String {
    redirect_by(url, true).unwrap_or_else(|| panic!("no redirect for {url}"))
}

/// The source of the catch-all rule, which matches every path.
const CATCH_ALL: &str = "/:path(.*)";

/// Like [`redirect`], optionally without the catch-all: a URL the page tells users to type must
/// have a rule of its own, not land on the installer only because everything does.
fn redirect_by(url: &str, with_catch_all: bool) -> Option<String> {
    let path = url.strip_prefix("https://get.faulk.app").unwrap();
    for rule in rules() {
        if !with_catch_all && rule.source_text == CATCH_ALL {
            continue;
        }
        if let Some(caps) = rule.source.captures(path) {
            let mut dest = rule.destination.clone();
            // Longest names first, so `:file` never eats part of `:filename`.
            let mut named: Vec<_> = rule.names.iter().enumerate().collect();
            named.sort_by_key(|(_, n)| std::cmp::Reverse(n.len()));
            for (i, name) in named {
                dest = dest.replace(&format!(":{name}"), &caps[i + 1]);
            }
            return Some(dest);
        }
    }
    None
}

/// The Blob store get.faulk.app redirects to, read from `web/vercel.json` (the one place that
/// names it), with the checks that it is one public Blob origin shared by every rule.
fn blob() -> String {
    let origins: std::collections::BTreeSet<String> = rules()
        .iter()
        .map(|r| {
            let url = url::Url::parse(&r.destination).unwrap();
            url.origin().ascii_serialization()
        })
        .collect();
    assert_eq!(origins.len(), 1, "one origin for every rule: {origins:?}");
    let origin = origins.into_iter().next().unwrap();
    let host = origin.strip_prefix("https://").expect("https");
    let store = host
        .strip_suffix(".public.blob.vercel-storage.com")
        .expect("a public Vercel Blob store");
    assert!(
        !store.is_empty() && store.chars().all(|c| c.is_ascii_alphanumeric()),
        "{host}"
    );
    origin
}

/// Where the publish script (`scripts/helper-release/publish-core.mjs`) writes each file:
/// `helper/<os>/packages/<file>` and `helper/<os>/<channel>/<file>`.
fn installer() -> String {
    format!("{}/helper/win/stable/{PACKAGE_ID}-win-Setup.exe", blob())
}

#[test]
fn feeds_go_to_their_channel_folder_in_the_store() {
    let urls = FeedUrls::new(DEFAULT_BASE_URL);
    for os in ["win", "linux", "osx"] {
        for channel in [Channel::Stable, Channel::Beta] {
            assert_eq!(
                redirect(&urls.feed_url(channel, os)),
                format!(
                    "{}/helper/{os}/{}/releases.{os}.json",
                    blob(),
                    channel.dir()
                )
            );
        }
    }
}

#[test]
fn packages_go_to_the_packages_folder_in_the_store() {
    let urls = FeedUrls::new(DEFAULT_BASE_URL);
    for (version, kind) in [
        ("0.2.0", "full"),
        ("0.2.0", "delta"),
        ("0.3.0-beta.1", "full"),
        ("0.3.0-rc-2", "delta"),
        ("10.20.30", "full"),
    ] {
        let file = format!("{PACKAGE_ID}-{version}-{kind}.nupkg");
        assert_eq!(
            redirect(&urls.package_url(&file).unwrap()),
            format!("{}/helper/win/packages/{file}", blob())
        );
    }
}

#[test]
fn the_download_link_and_everything_else_go_to_the_windows_installer() {
    for other in [
        "https://get.faulk.app/helper/windows",
        "https://get.faulk.app/",
        "https://get.faulk.app/helper",
        "https://get.faulk.app/helper/stable/RELEASES",
        "https://get.faulk.app/helper/packages/Other-1.0.0-full.nupkg",
    ] {
        assert_eq!(redirect(other), installer(), "{other}");
    }
}

/// The page tells an outdated helper's user where to download it again: that address must
/// be a real route to the installer.
#[test]
fn the_download_address_in_the_page_texts_is_a_real_route() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/src/lib/i18n/app.ts");
    let text = std::fs::read_to_string(path).unwrap();
    let found: Vec<&str> = text
        .match_indices("get.faulk.app")
        .map(|(i, _)| {
            let rest = &text[i..];
            let end = rest
                .find(|c: char| c.is_whitespace() || "'\",)".contains(c))
                .unwrap_or(rest.len());
            // A sentence may end right after the address.
            rest[..end].trim_end_matches('.')
        })
        .collect();
    assert!(found.len() >= 2, "the address is cited in both languages");
    for address in found {
        assert_eq!(
            redirect_by(&format!("https://{address}"), false),
            Some(installer()),
            "{address} must have its own rule, not only the catch-all"
        );
    }
    // The check has teeth: a mistyped address reaches the installer only through the catch-all.
    let typo = "https://get.faulk.app/helper/window";
    assert_eq!(redirect(typo), installer());
    assert_eq!(redirect_by(typo, false), None);
    assert!(
        rules().iter().any(|r| r.source_text == CATCH_ALL),
        "the catch-all is the rule this test sets aside"
    );
}
