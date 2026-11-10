//! Faulk without a browser (spike #820): one host that owns a project folder and runs the
//! page's own project core (`project_wasm::Core`) natively.
//!
//! - [`folder`]: the folder on disk, read into the core's folder frame, written back from its
//!   save frame, and its audio files read for a render.
//! - [`render`]: the offline render of an export job (`engine::offline`, the export's engine).
//! - [`policy`]: which commands the headless host runs, serves itself, or refuses.
//! - [`host`]: the host: open, create, run a command as an author, save, export.
//! - [`mcp`]: the catalog's commands as MCP tools over stdio, run by the host.
//!
//! The page owns the project in Web and Boost modes; without a page, this host does (provisional
//! answers on #821 for this prototype, to be validated by the founder; the core rule of
//! `CLAUDE.md` is unchanged until then). It runs no product logic of its own: every command goes
//! through the same core as the page's, and the tools are generated from the command catalog.

pub mod folder;
pub mod host;
pub mod mcp;
pub mod policy;
pub mod render;
