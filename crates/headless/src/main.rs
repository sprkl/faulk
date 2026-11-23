//! `faulk`: Faulk without a browser (spike #820).
//!
//! ```text
//! faulk new    <folder> [--name <name>]
//! faulk render <folder> [-o <file.wav>] [--region song|loop] [--rate <hz>] [--format f32|s24|s16]
//! faulk run    <folder> <commands.jsonl | -> [--save] [--keep-going] [--author <name>]
//! faulk mcp    <folder> --stdio
//! ```
//!
//! Exit codes: 0 done, 1 refused or failed, 2 usage.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use headless::host::{Answer, ExportFailure, Host, HostError};
use serde_json::{Map, Value, json};

const USAGE: &str = "usage:
  faulk new    <folder> [--name <name>]
  faulk render <folder> [-o <file.wav>] [--region song|loop] [--rate <hz>] [--format f32|s24|s16]
  faulk run    <folder> <commands.jsonl | -> [--save] [--keep-going] [--author <name>]
  faulk mcp    <folder> --stdio";

/// A command line split into its positional arguments and its options.
struct Args {
    positional: Vec<String>,
    options: Vec<(String, Option<String>)>,
}

impl Args {
    /// `flags` take no value; any other `-x`/`--x` takes the next argument.
    fn parse(raw: impl Iterator<Item = String>, flags: &[&str]) -> Result<Args, String> {
        let mut raw = raw.peekable();
        let mut args = Args {
            positional: Vec::new(),
            options: Vec::new(),
        };
        while let Some(arg) = raw.next() {
            if arg.starts_with('-') && arg != "-" {
                if flags.contains(&arg.as_str()) {
                    args.options.push((arg, None));
                } else {
                    let value = raw.next().ok_or_else(|| format!("{arg} takes a value"))?;
                    args.options.push((arg, Some(value)));
                }
            } else {
                args.positional.push(arg);
            }
        }
        Ok(args)
    }

    fn value(&self, names: &[&str]) -> Option<&str> {
        self.options
            .iter()
            .rev()
            .find(|(name, _)| names.contains(&name.as_str()))
            .and_then(|(_, value)| value.as_deref())
    }

    fn flag(&self, name: &str) -> bool {
        self.options.iter().any(|(n, _)| n == name)
    }

    fn only(&self, allowed: &[&str], positional: usize) -> Result<(), String> {
        if let Some((name, _)) = self
            .options
            .iter()
            .find(|(n, _)| !allowed.contains(&n.as_str()))
        {
            return Err(format!("unknown option {name}"));
        }
        if self.positional.len() != positional {
            return Err(format!(
                "{positional} argument(s) expected, {} given",
                self.positional.len()
            ));
        }
        Ok(())
    }
}

fn main() -> ExitCode {
    let mut raw = std::env::args().skip(1);
    let Some(verb) = raw.next() else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let result = match verb.as_str() {
        "new" => new(raw),
        "render" => render(raw),
        "run" => run(raw),
        "mcp" => mcp(raw),
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        other => Err(Failure::Usage(format!("unknown command {other:?}"))),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("faulk: {message}\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Failed(message)) => {
            eprintln!("faulk: {message}");
            ExitCode::from(1)
        }
    }
}

enum Failure {
    Usage(String),
    Failed(String),
}

impl From<HostError> for Failure {
    fn from(e: HostError) -> Self {
        Failure::Failed(e.to_string())
    }
}

fn usage(message: String) -> Failure {
    Failure::Usage(message)
}

fn new(raw: impl Iterator<Item = String>) -> Result<(), Failure> {
    let args = Args::parse(raw, &[]).map_err(usage)?;
    args.only(&["--name"], 1).map_err(usage)?;
    let root = PathBuf::from(&args.positional[0]);
    let host = Host::create(&root, args.value(&["--name"]))?;
    print_json(&json!({
        "folder": root.display().to_string(),
        "name": host.view().project.name,
    }));
    Ok(())
}

fn render(raw: impl Iterator<Item = String>) -> Result<(), Failure> {
    let args = Args::parse(raw, &[]).map_err(usage)?;
    args.only(&["-o", "--out", "--region", "--rate", "--format"], 1)
        .map_err(usage)?;
    let mut request = Map::new();
    if let Some(region) = args.value(&["--region"]) {
        if !matches!(region, "song" | "loop") {
            return Err(usage(format!("--region is song or loop, not {region:?}")));
        }
        request.insert("region".into(), json!(region));
    }
    if let Some(rate) = args.value(&["--rate"]) {
        let rate: u32 = rate
            .parse()
            .map_err(|_| usage(format!("--rate takes a number of hertz, not {rate:?}")))?;
        request.insert("sample_rate".into(), json!(rate));
    }
    if let Some(format) = args.value(&["--format"]) {
        let depth = match format {
            "f32" => "float32",
            "s24" => "pcm24",
            "s16" => "pcm16",
            other => return Err(usage(format!("--format is f32, s24 or s16, not {other:?}"))),
        };
        request.insert("bit_depth".into(), json!(depth));
    }
    let mut host = Host::open(Path::new(&args.positional[0]))?;
    let out = args.value(&["-o", "--out"]).map(PathBuf::from);
    match host.export(&request, out.as_deref()) {
        Ok(exported) => {
            if exported.peak_db.is_none() {
                eprintln!("faulk: warning: the render is silent");
            }
            if !exported.missing.is_empty() {
                eprintln!(
                    "faulk: warning: audio files missing from the folder, their clips are silent: {}",
                    exported.missing.join(", ")
                );
            }
            print_json(&exported.to_json());
            Ok(())
        }
        Err(ExportFailure::Refused(error)) => Err(Failure::Failed(format!(
            "the project cannot be exported as asked: {error}"
        ))),
        Err(ExportFailure::Failed(message)) => Err(Failure::Failed(message)),
    }
}

fn run(raw: impl Iterator<Item = String>) -> Result<(), Failure> {
    let args = Args::parse(raw, &["--save", "--keep-going"]).map_err(usage)?;
    args.only(&["--save", "--keep-going", "--author"], 2)
        .map_err(usage)?;
    let author = format!("agent:{}", args.value(&["--author"]).unwrap_or("cli"));
    let mut host = Host::open(Path::new(&args.positional[0]))?;
    let input: Box<dyn BufRead> = match args.positional[1].as_str() {
        "-" => Box::new(std::io::stdin().lock()),
        path => Box::new(std::io::BufReader::new(
            std::fs::File::open(path).map_err(|e| Failure::Failed(format!("{path}: {e}")))?,
        )),
    };
    let keep_going = args.flag("--keep-going");
    let mut failed = 0usize;
    for (index, line) in input.lines().enumerate() {
        let line = line.map_err(|e| Failure::Failed(format!("line {}: {e}", index + 1)))?;
        let text = line.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let answer = match serde_json::from_str::<Value>(text) {
            Ok(command) => host.call(&author, &command),
            Err(e) => Answer::Invalid {
                message: format!("not JSON: {e}"),
            },
        };
        let mut shown = answer.to_json();
        shown["line"] = json!(index + 1);
        print_json(&shown);
        if !answer.applied() {
            failed += 1;
            if !keep_going {
                break;
            }
        }
    }
    if args.flag("--save") {
        let wrote = host.save()?;
        eprintln!("faulk: {}", if wrote { "saved" } else { "nothing to save" });
    }
    if failed > 0 {
        return Err(Failure::Failed(format!("{failed} command(s) not applied")));
    }
    Ok(())
}

fn mcp(raw: impl Iterator<Item = String>) -> Result<(), Failure> {
    let args = Args::parse(raw, &["--stdio"]).map_err(usage)?;
    args.only(&["--stdio"], 1).map_err(usage)?;
    if !args.flag("--stdio") {
        return Err(usage("only --stdio is served by this prototype".into()));
    }
    let host = Host::open(Path::new(&args.positional[0]))?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Failure::Failed(format!("no async runtime: {e}")))?;
    runtime
        .block_on(headless::mcp::serve_stdio(host))
        .map_err(Failure::Failed)
}

fn print_json(value: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{value}");
}
