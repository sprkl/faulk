//! Which commands the headless host runs, and how (provisional answers on #821 for this prototype, not yet the
//! founder's decision).
//!
//! The host owns the folder it was started on, has no page, no live engine and no browser
//! storage. So:
//! - the commands of kind `edit`, `setting` and `query` run in the project core as they do in the
//!   page, except those listed below;
//! - `project.save`, `project.export_audio` and `project.analyze_render` are served by the host
//!   (the page serves them through its storage and its export Worker);
//! - `history.begin_group` and `history.end_group` run in the core (they are `live` because they
//!   change no project, not because they need a page);
//! - every other `live` command needs a page, a live engine, an audio or MIDI device or the
//!   browser's storage: refused with [`UNSUPPORTED`];
//! - `project.new` and `project.open` are refused: the host works on the folder it was started on
//!   (`faulk new` creates one); `project.list_stored` (the browser's projects) and `mixer.levels`
//!   (live meters) have nothing to answer with.

use commands::Kind;

/// The error code of a command the headless host does not run.
pub const UNSUPPORTED: &str = "headless.unsupported";

/// How the host runs a command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Route {
    /// Through the project core, as the page does.
    Core,
    /// `project.save`: the core, then the host writes the folder.
    Save,
    /// `project.export_audio`: the core checks it, then the host renders and writes the file.
    Export,
    /// `project.analyze_render`: the host renders and stages the measures, then the core answers.
    Analyze,
    /// Not run: why.
    Refused(String),
}

/// The live commands the core runs by itself.
const LIVE_IN_CORE: [&str; 2] = ["history.begin_group", "history.end_group"];

/// The route of the command named `name`, of kind `kind`.
pub fn route(name: &str, kind: Kind) -> Route {
    match name {
        "project.save" => Route::Save,
        "project.export_audio" => Route::Export,
        "project.analyze_render" => Route::Analyze,
        "project.new" | "project.open" => Route::Refused(format!(
            "{name}: the headless host works on the folder it was started on (`faulk new <folder>` creates a project)"
        )),
        "project.list_stored" => Route::Refused(format!(
            "{name}: the headless host keeps no projects of a browser"
        )),
        "mixer.levels" => Route::Refused(format!(
            "{name}: the headless host plays nothing, so it has no live meters (use project.analyze_render)"
        )),
        _ if LIVE_IN_CORE.contains(&name) => Route::Core,
        _ if kind == Kind::Live => Route::Refused(format!(
            "{name} needs a page, a live engine or the browser's storage: the headless host does not run it"
        )),
        _ => Route::Core,
    }
}

/// The route of a command of the catalog, by name; `None` for a name that is no command.
pub fn route_of(name: &str) -> Option<Route> {
    kind_of(name).map(|kind| route(name, kind))
}

/// The kind of the command named `name`, from the catalog.
pub fn kind_of(name: &str) -> Option<Kind> {
    use std::collections::HashMap;
    use std::sync::OnceLock;
    static KINDS: OnceLock<HashMap<String, Kind>> = OnceLock::new();
    KINDS
        .get_or_init(|| {
            commands::catalog()
                .into_iter()
                .map(|spec| (spec.name, spec.kind))
                .collect()
        })
        .get(name)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_live_command_is_refused_but_the_served_and_the_groups() {
        for spec in commands::catalog() {
            let route = route(&spec.name, spec.kind);
            let served = matches!(route, Route::Save | Route::Export | Route::Analyze);
            if spec.kind == Kind::Live && !served && !LIVE_IN_CORE.contains(&spec.name.as_str()) {
                assert!(matches!(route, Route::Refused(_)), "{}", spec.name);
            }
            if spec.destructive {
                assert!(matches!(route, Route::Refused(_)), "{}", spec.name);
            }
        }
    }

    #[test]
    fn the_served_and_core_live_commands_exist_in_the_catalog() {
        for name in [
            "project.save",
            "project.export_audio",
            "project.analyze_render",
            "project.new",
            "project.open",
            "project.list_stored",
            "mixer.levels",
        ]
        .into_iter()
        .chain(LIVE_IN_CORE)
        {
            assert!(kind_of(name).is_some(), "{name} is no command");
        }
    }
}
