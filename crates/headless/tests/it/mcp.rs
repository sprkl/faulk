//! `faulk mcp <folder> --stdio` driven as an MCP client drives it: newline-delimited JSON-RPC on
//! the process's stdin and stdout. An agent builds a song with the tools, exports it, and the
//! folder holds the song once the session ends.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use headless::host::Host;
use serde_json::{Value, json};

use crate::song::{clip_command, notes_command, scratch, track_commands};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Client {
    fn start(dir: &str) -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_faulk"))
            .args(["mcp", dir, "--stdio"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("the server starts");
        let stdin = child.stdin.take().expect("stdin");
        let stdout = BufReader::new(child.stdout.take().expect("stdout"));
        let mut client = Client {
            child,
            stdin,
            stdout,
            next: 1,
        };
        let init = client.request(
            "initialize",
            json!({
                "protocolVersion": "2025-06-18",
                "capabilities": {},
                "clientInfo": { "name": "it-agent", "version": "0" }
            }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "faulk", "{init}");
        client.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        client
    }

    fn send(&mut self, message: &Value) {
        writeln!(self.stdin, "{message}").expect("sent");
        self.stdin.flush().expect("flushed");
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let mut line = String::new();
            assert!(
                self.stdout.read_line(&mut line).expect("read") > 0,
                "the server closed"
            );
            let message: Value = serde_json::from_str(&line).expect("JSON-RPC");
            if message["id"] == json!(id) {
                return message;
            }
        }
    }

    /// Calls the tool of `command` (a command's JSON, with its `type`).
    fn call(&mut self, command: &Value) -> Value {
        let mut arguments = command.as_object().expect("an object").clone();
        let name = arguments
            .remove("type")
            .and_then(|t| t.as_str().map(|t| t.replace('.', "_")))
            .expect("a type");
        let answer = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        answer["result"].clone()
    }

    fn close(mut self) {
        drop(self.stdin);
        let status = self.child.wait().expect("the server ends");
        assert!(status.success(), "{status:?}");
    }
}

#[test]
fn an_agent_builds_and_exports_a_song_through_the_tools() {
    let root = scratch("mcp");
    let dir = root.to_str().expect("utf-8");
    Host::create(&root, Some("Agent song")).expect("made");
    let mut client = Client::start(dir);

    let tools = client.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .expect("tools")
        .iter()
        .map(|t| t["name"].as_str().expect("a name"))
        .collect();
    assert!(names.contains(&"track_create") && names.contains(&"project_export_audio"));
    assert!(!names.contains(&"project_open") && !names.contains(&"transport_play"));

    for (index, create) in track_commands().into_iter().enumerate() {
        let made = client.call(&create);
        assert_ne!(made["isError"], true, "{made}");
        let track =
            serde_json::from_value(made["structuredContent"]["track"].clone()).expect("a track");
        let clip = client.call(&clip_command(&track, 2));
        let notes = client.call(&notes_command(
            clip["structuredContent"]["clip"].clone(),
            2,
            index == 1,
        ));
        assert_ne!(notes["isError"], true, "{notes}");
    }
    // Saved after each call: the folder already holds the tracks.
    assert_eq!(
        Host::open(&root)
            .expect("opens")
            .view()
            .project
            .tracks
            .len(),
        2
    );

    let analysis = client.call(&json!({ "type": "project.analyze_render" }));
    assert_eq!(
        analysis["structuredContent"]["silenced_frames"], 0,
        "{analysis}"
    );
    let export = client.call(&json!({ "type": "project.export_audio" }));
    assert_ne!(export["isError"], true, "{export}");
    assert!(
        export.to_string().contains("exports/Agent song.wav"),
        "{export}"
    );

    let refused = client.call(&json!({ "type": "track.delete", "track": "trk_0000000000000000" }));
    assert_eq!(refused["isError"], true, "{refused}");
    let unknown = client.request(
        "tools/call",
        json!({ "name": "transport_play", "arguments": {} }),
    );
    assert!(unknown.get("error").is_some(), "{unknown}");
    client.close();

    let host = Host::open(&root).expect("opens");
    assert_eq!(host.view().project.tracks.len(), 2);
    let wav = std::fs::read(root.join("exports/Agent song.wav")).expect("the export");
    assert!(wav.len() > 44 + 2 * 48_000 * 4);
    let history = serde_json::to_value(host.history()).expect("serializes");
    // The history is not kept in the folder: a reopened project starts a new one.
    assert!(history.is_object());
}
