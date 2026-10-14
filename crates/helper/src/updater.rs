//! Self-update of the installed helper, built on Velopack.
//!
//! The helper reads its updates only from `https://get.faulk.app/helper/...`, which Vercel
//! redirects to the public releases of `sprkl/faulk-release` (see
//! `docs/architecture/04-distribution-helper.md` and `web/vercel.json`):
//!
//! - `<base>/stable/releases.<os>.json`: the feed of the newest stable release;
//! - `<base>/beta/releases.<os>.json`: the feed of the newest pre-release;
//! - `<base>/packages/<file>.nupkg`: a full or delta package, found in the release of its version.
//!
//! A helper on the stable channel reads the stable feed only, and drops every pre-release it may
//! contain. A helper on the beta channel reads both feeds and takes the newest version.
//!
//! The updater runs on its own thread (`faulk-updater`): one check at start, then one every six
//! hours or when [`Updater::check_now`] is called. An update found is downloaded in the
//! background, then applied either at the next start ([`startup`], Velopack's default) or on
//! request ([`Updater::apply_and_restart`]). Nothing here runs on, waits for or shares state with
//! the audio thread.

use std::fs::{File, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::Duration;

use velopack::bundle::Manifest;
use velopack::sources::{HttpSource, UpdateSource};
use velopack::{
    HttpOptions, NetworkError, UpdateCheck, UpdateInfo, UpdateManager, VelopackApp, VelopackAsset,
    VelopackAssetFeed,
};

/// Where the helper reads its updates. Only this host, or the override below, is ever contacted.
pub const DEFAULT_BASE_URL: &str = "https://get.faulk.app/helper";
/// Environment variable that replaces [`DEFAULT_BASE_URL`], for tests against a local feed.
/// Accepted only for `https://` or for `http://` to a loopback address.
pub const BASE_URL_ENV: &str = "FAULK_UPDATE_URL";
/// Environment variable that forces the channel: `stable` or `beta`.
pub const CHANNEL_ENV: &str = "FAULK_UPDATE_CHANNEL";
/// Velopack package id, as given to `vpk pack --packId`.
pub const PACKAGE_ID: &str = "FaulkHelper";
/// Time between two automatic checks.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

const FEED_TIMEOUT_MS: u64 = 30_000;
const PACKAGE_TIMEOUT_MS: u64 = 10 * 60_000;

/// Runs Velopack's startup logic. Must be the first call of `main`, before the arguments are
/// parsed: the installer and the updater launch the executable with `--veloapp-*` arguments and
/// expect it to exit at once. It also applies an update downloaded during a previous run (and
/// restarts the helper on the new version), but only when no other helper is running: applying
/// replaces the files of the installation and would stop that other helper's audio. Every
/// helper holds a shared [`InstanceLock`] for its whole life to make that check possible, at
/// start here and before a manual apply ([`Updater::apply_and_restart`]). Does nothing when the
/// helper is not installed.
pub fn startup() {
    install_logger();
    let lock = match InstanceLock::take(&instance_lock_path(), LOCK_PATIENCE) {
        Ok(lock) => {
            if !lock.alone() {
                eprintln!(
                    "update: another helper is running, a downloaded update waits for it to stop"
                );
            }
            Some(INSTANCE_LOCK.get_or_init(|| Arc::new(lock)).clone())
        }
        Err(e) => {
            eprintln!(
                "update: cannot take the instance lock ({e}), a downloaded update is not applied"
            );
            None
        }
    };
    let alone = lock.as_ref().is_some_and(|l| l.alone());
    // While alone, the lock stays exclusive: a helper starting now waits instead of starting
    // under an apply. Applying exits the process; otherwise the lock becomes shared again.
    VelopackApp::build().set_auto_apply_on_startup(alone).run();
    if let Some(lock) = lock {
        lock.share();
    }
}

/// How long a starting helper waits for another one to finish deciding whether to apply.
const LOCK_PATIENCE: Duration = Duration::from_secs(10);

static INSTANCE_LOCK: OnceLock<Arc<InstanceLock>> = OnceLock::new();

fn instance_lock_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("faulk")
        .join("helper.lock")
}

/// The lock file every running helper holds in shared mode. A helper is alone when it can take
/// the lock exclusively: no other helper holds it, whatever its port and whichever started first.
///
/// Every change of mode (take, claim, share) happens under a second, exclusive "gate" lock, so
/// that a helper never drops its shared hold while another one decides it is alone.
pub struct InstanceLock {
    file: File,
    gate: File,
    alone_at_start: bool,
}

/// Holds the gate until dropped.
struct Gate<'a>(&'a File);

impl Drop for Gate<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn open_lock_file(path: &Path) -> std::io::Result<File> {
    std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
}

impl InstanceLock {
    /// Opens the lock at `path` and holds it as long as the returned value lives. When no other
    /// helper holds it, it is held exclusively and [`InstanceLock::alone`] is true; call
    /// [`InstanceLock::share`] once the startup decision is made. Otherwise it is held shared.
    /// A helper that holds it exclusively (deciding, or applying an update and about to exit)
    /// is waited for, at most `patience`; after that this one runs unprotected and not alone.
    pub fn take(path: &Path, patience: Duration) -> std::io::Result<InstanceLock> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let file = open_lock_file(path)?;
        let gate = open_lock_file(&path.with_extension("gate"))?;
        let start = std::time::Instant::now();
        loop {
            {
                gate.lock()?;
                let _gate = Gate(&gate);
                match file.try_lock() {
                    Ok(()) => {
                        drop(_gate);
                        return Ok(InstanceLock {
                            file,
                            gate,
                            alone_at_start: true,
                        });
                    }
                    Err(TryLockError::WouldBlock) => {}
                    Err(TryLockError::Error(e)) => return Err(e),
                }
                match file.try_lock_shared() {
                    Ok(()) => {
                        drop(_gate);
                        return Ok(InstanceLock {
                            file,
                            gate,
                            alone_at_start: false,
                        });
                    }
                    Err(TryLockError::WouldBlock) => {}
                    Err(TryLockError::Error(e)) => return Err(e),
                }
            }
            if start.elapsed() >= patience {
                eprintln!("update: another helper keeps the instance lock, running without it");
                return Ok(InstanceLock {
                    file,
                    gate,
                    alone_at_start: false,
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Whether no other helper held the lock when this one took it.
    pub fn alone(&self) -> bool {
        self.alone_at_start
    }

    /// Whether this process is the only helper now. When it is, the lock stays exclusive, so
    /// that no helper starting meanwhile believes itself alone; [`InstanceLock::share`] gives it
    /// back. When it is not, the shared hold is kept.
    pub fn claim_alone(&self) -> std::io::Result<bool> {
        self.gate.lock()?;
        let _gate = Gate(&self.gate);
        // Unlocking a handle that holds no lock fails on Windows: nothing to undo then.
        let _ = self.file.unlock();
        match self.file.try_lock() {
            Ok(()) => Ok(true),
            Err(e) => {
                // Other helpers hold the lock shared only (none can be exclusive while this one
                // held it shared), so the shared hold comes back at once.
                let _ = self.file.try_lock_shared();
                match e {
                    TryLockError::WouldBlock => Ok(false),
                    TryLockError::Error(e) => Err(e),
                }
            }
        }
    }

    /// Back to a shared hold: after [`InstanceLock::take`] when alone, or after
    /// [`InstanceLock::claim_alone`] (for instance a failed apply).
    pub fn share(&self) {
        if self.gate.lock().is_err() {
            return;
        }
        let _gate = Gate(&self.gate);
        let _ = self.file.unlock();
        let _ = self.file.try_lock_shared();
    }
}

/// Velopack reports through the `log` crate; without a logger a failed apply or delta patch
/// would leave no trace. Velopack's warnings and errors go to stderr, like the rest of the
/// helper. Records of other crates are left out: they are not about updates. The logger is
/// process-wide; if one is already installed, this one is not, and nothing fails.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        metadata.level() <= log::Level::Warn && metadata.target().starts_with("velopack")
    }
    fn log(&self, record: &log::Record) {
        if self.enabled(record.metadata()) {
            eprintln!("update: velopack {}: {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

static LOGGER: StderrLog = StderrLog;

fn install_logger() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Warn);
    }
}

/// Update channel. A pre-release never reaches [`Channel::Stable`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Beta,
}

impl Channel {
    /// The channel of an installed version: a pre-release (`0.3.0-beta.1`) is on beta.
    pub fn for_version(version: &semver::Version) -> Channel {
        if version.pre.is_empty() {
            Channel::Stable
        } else {
            Channel::Beta
        }
    }

    /// Parses `stable` or `beta` (case-insensitive).
    pub fn parse(s: &str) -> Option<Channel> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stable" => Some(Channel::Stable),
            "beta" => Some(Channel::Beta),
            _ => None,
        }
    }

    /// The feed folder of the channel under the base URL.
    pub fn dir(self) -> &'static str {
        match self {
            Channel::Stable => "stable",
            Channel::Beta => "beta",
        }
    }
}

/// The URLs the updater reads, all under one base.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedUrls {
    base: String,
}

impl FeedUrls {
    pub fn new(base: &str) -> FeedUrls {
        FeedUrls {
            base: base.trim_end_matches('/').to_owned(),
        }
    }

    /// Folder of a channel's feed: Velopack appends `releases.<os>.json` to it.
    pub fn feed_dir(&self, channel: Channel) -> String {
        format!("{}/{}", self.base, channel.dir())
    }

    /// Folder of the packages: Velopack appends the package file name to it.
    pub fn packages_dir(&self) -> String {
        format!("{}/packages", self.base)
    }

    /// Full URL of a channel's feed for a Velopack OS channel (`win`, `linux`, `osx`).
    pub fn feed_url(&self, channel: Channel, os_channel: &str) -> String {
        format!("{}/releases.{os_channel}.json", self.feed_dir(channel))
    }

    /// Full URL of a package, or `None` when the file name is not a plain file name.
    pub fn package_url(&self, file_name: &str) -> Option<String> {
        is_safe_file_name(file_name).then(|| format!("{}/{file_name}", self.packages_dir()))
    }
}

/// Picks the base URL: the default, or a valid override. Returns the base and, when an override
/// was refused, the reason to log.
pub fn resolve_base_url(override_url: Option<&str>) -> (String, Option<String>) {
    let Some(raw) = override_url.map(str::trim).filter(|s| !s.is_empty()) else {
        return (DEFAULT_BASE_URL.to_owned(), None);
    };
    // The feed and package paths are appended to the base: a query or a fragment would swallow
    // them.
    let plain = |url: &url::Url| url.query().is_none() && url.fragment().is_none();
    match url::Url::parse(raw) {
        Ok(url) if plain(&url) && url.scheme() == "https" && url.host().is_some() => {
            (raw.to_owned(), None)
        }
        Ok(url) if plain(&url) && url.scheme() == "http" && is_loopback(&url) => {
            (raw.to_owned(), None)
        }
        _ => (
            DEFAULT_BASE_URL.to_owned(),
            Some(format!(
                "{BASE_URL_ENV}={raw} ignored: only https:// or http:// to a loopback address, \
                 without query or fragment"
            )),
        ),
    }
}

fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

/// A package name the updater may download: one plain file name, nothing that walks paths.
pub fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name != "."
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Keeps the assets the channel may install: ours, with a safe file name and a valid version,
/// and on stable never a pre-release. On beta, delta packages are dropped too: a beta install
/// does not follow a straight line of versions (betas, then a stable built from another base),
/// and a delta applies only to the version it was built from, so beta updates are full packages.
pub fn filter_feed(
    channel: Channel,
    package_id: &str,
    feed: VelopackAssetFeed,
) -> VelopackAssetFeed {
    let assets = feed
        .Assets
        .into_iter()
        .filter(|a| a.PackageId.eq_ignore_ascii_case(package_id))
        .filter(|a| is_safe_file_name(&a.FileName))
        .filter(|a| channel == Channel::Stable || a.Type.eq_ignore_ascii_case("Full"))
        .filter(|a| match semver::Version::parse(&a.Version) {
            Ok(v) => channel == Channel::Beta || v.pre.is_empty(),
            Err(_) => false,
        })
        .collect();
    VelopackAssetFeed { Assets: assets }
}

/// Union of two feeds, without duplicate file names.
pub fn merge_feeds(a: VelopackAssetFeed, b: VelopackAssetFeed) -> VelopackAssetFeed {
    let mut assets = a.Assets;
    for asset in b.Assets {
        if !assets
            .iter()
            .any(|x| x.FileName.eq_ignore_ascii_case(&asset.FileName))
        {
            assets.push(asset);
        }
    }
    VelopackAssetFeed { Assets: assets }
}

/// Why a check or a download failed, in the terms a tray menu can show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    /// No connection, DNS failure, timeout, or a server error.
    Network(String),
    /// The feed does not exist (no release published yet, or the redirect is missing).
    FeedNotFound,
    /// The feed or a package could not be read.
    InvalidFeed(String),
    /// A downloaded package does not match the feed (checksum or size).
    Corrupt(String),
    /// Anything else (disk, lock held by another update, Velopack internals).
    Other(String),
}

impl std::fmt::Display for UpdateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UpdateError::Network(m) => write!(f, "network error: {m}"),
            UpdateError::FeedNotFound => write!(f, "no update feed published yet"),
            UpdateError::InvalidFeed(m) => write!(f, "invalid update feed: {m}"),
            UpdateError::Corrupt(m) => write!(f, "corrupt download: {m}"),
            UpdateError::Other(m) => write!(f, "{m}"),
        }
    }
}

/// Maps a Velopack error to an [`UpdateError`].
pub fn classify(error: &velopack::Error) -> UpdateError {
    use velopack::Error as E;
    match error {
        E::Network(n) => match n.as_ref() {
            NetworkError::Http(ureq::Error::StatusCode(404)) => UpdateError::FeedNotFound,
            other => UpdateError::Network(other.to_string()),
        },
        E::Json(e) => UpdateError::InvalidFeed(e.to_string()),
        E::Semver(e) => UpdateError::InvalidFeed(e.to_string()),
        E::InvalidPackage(m) => UpdateError::InvalidFeed(m.clone()),
        E::ChecksumInvalid(..) | E::SizeInvalid(..) => UpdateError::Corrupt(error.to_string()),
        other => UpdateError::Other(other.to_string()),
    }
}

fn is_not_found(error: &velopack::Error) -> bool {
    classify(error) == UpdateError::FeedNotFound
}

/// A Velopack [`UpdateSource`] that applies Faulk's channels on top of three plain sources: the
/// stable feed, the beta feed and the packages.
pub struct ChannelSource<S> {
    channel: Channel,
    package_id: String,
    stable: S,
    beta: S,
    packages: S,
}

impl<S: UpdateSource> ChannelSource<S> {
    pub fn new(channel: Channel, package_id: &str, stable: S, beta: S, packages: S) -> Self {
        ChannelSource {
            channel,
            package_id: package_id.to_owned(),
            stable,
            beta,
            packages,
        }
    }
}

impl ChannelSource<HttpSource> {
    /// The real source, reading over HTTPS under `urls`.
    pub fn http(channel: Channel, urls: &FeedUrls) -> Self {
        let options = |ms| HttpOptions {
            Headers: Vec::new(),
            TimeoutMilliseconds: ms,
        };
        ChannelSource::new(
            channel,
            PACKAGE_ID,
            HttpSource::new_with_options(urls.feed_dir(Channel::Stable), options(FEED_TIMEOUT_MS)),
            HttpSource::new_with_options(urls.feed_dir(Channel::Beta), options(FEED_TIMEOUT_MS)),
            HttpSource::new_with_options(urls.packages_dir(), options(PACKAGE_TIMEOUT_MS)),
        )
    }
}

impl<S: UpdateSource> UpdateSource for ChannelSource<S> {
    fn get_release_feed(
        &self,
        os_channel: &str,
        app: &Manifest,
        staged_user_id: &str,
    ) -> Result<VelopackAssetFeed, velopack::Error> {
        let stable = self
            .stable
            .get_release_feed(os_channel, app, staged_user_id);
        let feed = match self.channel {
            Channel::Stable => stable?,
            Channel::Beta => {
                let beta = self.beta.get_release_feed(os_channel, app, staged_user_id);
                match (stable, beta) {
                    (Ok(s), Ok(b)) => merge_feeds(s, b),
                    (Ok(s), Err(e)) => {
                        // No pre-release published yet is the normal case, not worth a line.
                        if !is_not_found(&e) {
                            eprintln!(
                                "update: beta feed unavailable ({}), using stable",
                                classify(&e)
                            );
                        }
                        s
                    }
                    (Err(e), Ok(b)) => {
                        if !is_not_found(&e) {
                            eprintln!(
                                "update: stable feed unavailable ({}), using beta",
                                classify(&e)
                            );
                        }
                        b
                    }
                    (Err(e), Err(_)) => return Err(e),
                }
            }
        };
        Ok(filter_feed(self.channel, &self.package_id, feed))
    }

    fn download_release_entry(
        &self,
        asset: &VelopackAsset,
        local_file: &Path,
        progress: Option<Sender<i16>>,
    ) -> Result<(), velopack::Error> {
        if !is_safe_file_name(&asset.FileName) {
            return Err(velopack::Error::InvalidPackage(format!(
                "unsafe package name {:?}",
                asset.FileName
            )));
        }
        self.packages
            .download_release_entry(asset, local_file, progress)
    }
}

/// What the updater is doing, for a tray menu or a log line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateStatus {
    /// Updates are off: the helper is not installed (run from `cargo`), or it has no feed.
    Disabled {
        reason: String,
    },
    /// Started, first check not done yet.
    Idle,
    Checking,
    /// The installed version is the newest of its channel.
    UpToDate {
        version: String,
    },
    Downloading {
        version: String,
        percent: u8,
    },
    /// Downloaded; applied at the next start or by [`Updater::apply_and_restart`].
    Ready {
        version: String,
    },
    /// The last check or download failed; the next check retries.
    Failed {
        error: UpdateError,
    },
}

/// The operations the updater needs from Velopack. [`UpdateManager`] is the real one; tests use
/// a fake.
pub trait Backend: Send + Sync + 'static {
    /// The installed version.
    fn current_version(&self) -> String;
    /// `Ok(None)` when there is nothing newer.
    fn check(&self) -> Result<Option<UpdateInfo>, velopack::Error>;
    fn download(&self, update: &UpdateInfo, progress: Sender<i16>) -> Result<(), velopack::Error>;
    /// A package downloaded and not applied yet.
    fn pending(&self) -> Option<VelopackAsset>;
    /// Starts Velopack's updater and exits the process; returns only on failure.
    fn apply_and_restart(
        &self,
        asset: &VelopackAsset,
        args: &[String],
    ) -> Result<(), velopack::Error>;
}

impl Backend for UpdateManager {
    fn current_version(&self) -> String {
        self.get_current_version_as_string()
    }

    fn check(&self) -> Result<Option<UpdateInfo>, velopack::Error> {
        Ok(match self.check_for_updates()? {
            UpdateCheck::UpdateAvailable(info) => Some(*info),
            UpdateCheck::NoUpdateAvailable | UpdateCheck::RemoteIsEmpty => None,
        })
    }

    fn download(&self, update: &UpdateInfo, progress: Sender<i16>) -> Result<(), velopack::Error> {
        self.download_updates(update, Some(progress))
    }

    fn pending(&self) -> Option<VelopackAsset> {
        self.get_update_pending_restart()
    }

    fn apply_and_restart(
        &self,
        asset: &VelopackAsset,
        args: &[String],
    ) -> Result<(), velopack::Error> {
        self.apply_updates_and_restart_with_args(asset, args)
    }
}

/// Updater settings, read from the environment by [`Config::from_env`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub base_url: String,
    /// `None`: the channel follows the installed version.
    pub channel: Option<Channel>,
    pub interval: Duration,
}

impl Config {
    /// Builds the settings from the two optional variables; returns the lines to log for values
    /// that were refused.
    pub fn from_values(base_url: Option<&str>, channel: Option<&str>) -> (Config, Vec<String>) {
        let mut warnings = Vec::new();
        let (base_url, refused) = resolve_base_url(base_url);
        warnings.extend(refused);
        let channel = channel
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .and_then(|raw| {
                let parsed = Channel::parse(raw);
                if parsed.is_none() {
                    warnings.push(format!(
                        "{CHANNEL_ENV}={raw} ignored: expected stable or beta"
                    ));
                }
                parsed
            });
        (
            Config {
                base_url,
                channel,
                interval: CHECK_INTERVAL,
            },
            warnings,
        )
    }

    /// Reads [`BASE_URL_ENV`] and [`CHANNEL_ENV`], logging refused values.
    pub fn from_env() -> Config {
        let url = std::env::var(BASE_URL_ENV).ok();
        let channel = std::env::var(CHANNEL_ENV).ok();
        let (config, warnings) = Config::from_values(url.as_deref(), channel.as_deref());
        for w in warnings {
            eprintln!("update: {w}");
        }
        config
    }
}

enum Command {
    CheckNow,
    Shutdown,
}

/// Handle on the update thread. Cheap to query from any thread except the audio one, which never
/// needs it.
pub struct Updater {
    status: Arc<Mutex<UpdateStatus>>,
    backend: Option<Arc<dyn Backend>>,
    commands: Option<SyncSender<Command>>,
    thread: Option<JoinHandle<()>>,
    restart_args: Vec<String>,
    instance: InstanceCheck,
    stopping: Arc<AtomicBool>,
    /// Held by the update thread during a check or a download, and by an apply.
    busy: Arc<Mutex<()>>,
}

/// How [`Updater::apply_and_restart`] makes sure no other helper is running.
enum InstanceCheck {
    /// Through the instance lock taken by [`startup`].
    Lock(Arc<InstanceLock>),
    /// The lock could not be taken: a manual apply is refused, as the apply at start is.
    Unavailable(String),
    /// No check: an updater started with [`Updater::start_with`] on a test backend.
    None,
}

impl Updater {
    /// Starts the updater of the installed helper. When the helper is not installed (run from
    /// `cargo`), returns a disabled updater that never touches the network.
    pub fn start(config: Config) -> Updater {
        // Probe the installation without any network source, to read the installed version.
        let probe = match UpdateManager::new(velopack::sources::NoneSource {}, None, None) {
            Ok(m) => m,
            Err(e) => return Updater::disabled(format!("not installed: {e}")),
        };
        let channel = config
            .channel
            .unwrap_or_else(|| Channel::for_version(&probe.get_current_version()));
        let urls = FeedUrls::new(&config.base_url);
        let source = ChannelSource::http(channel, &urls);
        match UpdateManager::new(source, None, None) {
            Ok(manager) => {
                eprintln!(
                    "update: {} {} on the {} channel, feed {}",
                    PACKAGE_ID,
                    manager.get_current_version_as_string(),
                    channel.dir(),
                    urls.feed_dir(channel)
                );
                let mut updater = Updater::start_with(manager, config.interval);
                updater.instance = match INSTANCE_LOCK.get() {
                    Some(lock) => InstanceCheck::Lock(lock.clone()),
                    None => match InstanceLock::take(&instance_lock_path(), LOCK_PATIENCE) {
                        Ok(lock) => {
                            lock.share();
                            InstanceCheck::Lock(
                                INSTANCE_LOCK.get_or_init(|| Arc::new(lock)).clone(),
                            )
                        }
                        Err(e) => InstanceCheck::Unavailable(format!(
                            "cannot take the instance lock ({e})"
                        )),
                    },
                };
                updater
            }
            Err(e) => Updater::disabled(format!("not installed: {e}")),
        }
    }

    /// An updater that does nothing, with the reason shown in its status.
    pub fn disabled(reason: String) -> Updater {
        Updater {
            status: Arc::new(Mutex::new(UpdateStatus::Disabled { reason })),
            backend: None,
            commands: None,
            thread: None,
            restart_args: Vec::new(),
            instance: InstanceCheck::None,
            stopping: Arc::new(AtomicBool::new(true)),
            busy: Arc::new(Mutex::new(())),
        }
    }

    /// Starts the update thread on any backend: one check now, then every `interval`.
    pub fn start_with<B: Backend>(backend: B, interval: Duration) -> Updater {
        let backend: Arc<dyn Backend> = Arc::new(backend);
        let status = Arc::new(Mutex::new(UpdateStatus::Idle));
        // Capacity 1: clicks on "Check for updates" during a check coalesce into one more check.
        let (tx, rx) = mpsc::sync_channel(1);
        let stopping = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(Mutex::new(()));
        let thread = {
            let backend = backend.clone();
            let status = status.clone();
            let stopping = stopping.clone();
            let busy = busy.clone();
            std::thread::Builder::new()
                .name("faulk-updater".into())
                .spawn(move || run_loop(backend.as_ref(), &status, &rx, interval, &stopping, &busy))
        };
        match thread {
            Ok(thread) => Updater {
                status,
                backend: Some(backend),
                commands: Some(tx),
                thread: Some(thread),
                restart_args: std::env::args().skip(1).collect(),
                instance: InstanceCheck::None,
                stopping,
                busy,
            },
            Err(e) => Updater::disabled(format!("cannot start the update thread: {e}")),
        }
    }

    /// The current status.
    pub fn status(&self) -> UpdateStatus {
        lock(&self.status).clone()
    }

    /// Asks for a check now, without waiting for it. Returns `false` when updates are disabled.
    pub fn check_now(&self) -> bool {
        match &self.commands {
            Some(tx) => match tx.try_send(Command::CheckNow) {
                Ok(()) | Err(TrySendError::Full(_)) => true,
                Err(TrySendError::Disconnected(_)) => false,
            },
            None => false,
        }
    }

    /// Makes [`Updater::apply_and_restart`] check `lock` first. [`Updater::start`] does it with
    /// the lock taken by [`startup`]; this is for an updater started with
    /// [`Updater::start_with`].
    pub fn with_instance_lock(mut self, lock: Arc<InstanceLock>) -> Updater {
        self.instance = InstanceCheck::Lock(lock);
        self
    }

    /// Applies the downloaded update and restarts the helper on it, with the same arguments.
    /// Refused while another helper is running: applying replaces the installation's files
    /// under it. Refused too while the update thread checks or downloads: Velopack would apply
    /// from a packages folder being written. On success the process exits and this never
    /// returns.
    pub fn apply_and_restart(&self) -> Result<(), UpdateError> {
        let backend = self
            .backend
            .as_ref()
            .ok_or_else(|| UpdateError::Other("updates are disabled".into()))?;
        let _busy = match self.busy.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                return Err(UpdateError::Other(
                    "an update check or download is running: try again when it ends".into(),
                ));
            }
        };
        let asset = backend
            .pending()
            .ok_or_else(|| UpdateError::Other("no downloaded update to apply".into()))?;
        let lock = match &self.instance {
            InstanceCheck::Lock(lock) => match lock.claim_alone() {
                Ok(true) => Some(lock),
                Ok(false) => {
                    return Err(UpdateError::Other(
                        "another helper is running: close it, then restart to update".into(),
                    ));
                }
                Err(e) => {
                    return Err(UpdateError::Other(format!(
                        "cannot check for another helper ({e})"
                    )));
                }
            },
            InstanceCheck::Unavailable(reason) => return Err(UpdateError::Other(reason.clone())),
            InstanceCheck::None => None,
        };
        let result = backend
            .apply_and_restart(&asset, &self.restart_args)
            .map_err(|e| classify(&e));
        if let Some(lock) = lock {
            // Still running: the apply failed (or a test backend returned).
            lock.share();
        }
        result
    }

    /// Stops the thread and waits for it (at most the end of a running check or download; a
    /// check queued meanwhile is skipped).
    pub fn shutdown(mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(tx) = self.commands.take() {
            // A full queue means a check is already queued: the loop sees `stopping` before
            // running it, and dropping `tx` wakes a loop that waits.
            let _ = tx.try_send(Command::Shutdown);
        }
    }
}

impl Drop for Updater {
    fn drop(&mut self) {
        // Never blocks: the thread ends after its current step.
        self.stop();
    }
}

fn lock(status: &Mutex<UpdateStatus>) -> std::sync::MutexGuard<'_, UpdateStatus> {
    status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set(status: &Mutex<UpdateStatus>, next: UpdateStatus) {
    *lock(status) = next;
}

fn run_loop(
    backend: &dyn Backend,
    status: &Mutex<UpdateStatus>,
    commands: &Receiver<Command>,
    interval: Duration,
    stopping: &AtomicBool,
    busy: &Mutex<()>,
) {
    loop {
        if stopping.load(Ordering::Acquire) {
            return;
        }
        {
            let _busy = busy.lock().unwrap_or_else(|p| p.into_inner());
            run_check(backend, status);
        }
        match commands.recv_timeout(interval) {
            Ok(Command::CheckNow) | Err(RecvTimeoutError::Timeout) => continue,
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// One check, and the download it leads to. Public for tests; the thread calls it.
pub fn run_check(backend: &dyn Backend, status: &Mutex<UpdateStatus>) {
    let ready_before = match &*lock(status) {
        UpdateStatus::Ready { version } => Some(version.clone()),
        _ => None,
    };
    set(status, UpdateStatus::Checking);
    let update = match backend.check() {
        Ok(Some(update)) => update,
        Ok(None) => {
            let next = match backend.pending() {
                Some(asset) => UpdateStatus::Ready {
                    version: asset.Version,
                },
                None => UpdateStatus::UpToDate {
                    version: backend.current_version(),
                },
            };
            set(status, next);
            return;
        }
        Err(e) => {
            let error = classify(&e);
            eprintln!("update: check failed: {error}");
            set(status, after_failure(backend, error));
            return;
        }
    };
    let version = update.TargetFullRelease.Version.clone();
    // Already downloaded: skip, but only while the package is still there (it may have been
    // cleaned up, or consumed by a failed apply).
    let still_pending = backend
        .pending()
        .is_some_and(|asset| asset.Version == version);
    if ready_before.as_deref() == Some(version.as_str()) && still_pending {
        set(status, UpdateStatus::Ready { version });
        return;
    }
    eprintln!("update: {version} available, downloading");
    set(
        status,
        UpdateStatus::Downloading {
            version: version.clone(),
            percent: 0,
        },
    );
    let (progress_tx, progress_rx) = mpsc::channel::<i16>();
    let result = std::thread::scope(|scope| {
        scope.spawn(|| {
            // Ends when the download drops its sender.
            for p in progress_rx {
                let percent = p.clamp(0, 100) as u8;
                let mut s = lock(status);
                if let UpdateStatus::Downloading {
                    percent: current, ..
                } = &mut *s
                {
                    *current = percent;
                }
            }
        });
        backend.download(&update, progress_tx)
    });
    match result {
        Ok(()) => {
            eprintln!("update: {version} downloaded, applied at the next start");
            set(status, UpdateStatus::Ready { version });
        }
        Err(e) => {
            let error = classify(&e);
            eprintln!("update: download of {version} failed: {error}");
            set(status, after_failure(backend, error));
        }
    }
}

/// After a failure, a package already on disk stays applicable.
fn after_failure(backend: &dyn Backend, error: UpdateError) -> UpdateStatus {
    match backend.pending() {
        Some(asset) => UpdateStatus::Ready {
            version: asset.Version,
        },
        None => UpdateStatus::Failed { error },
    }
}

#[cfg(test)]
mod tests;
