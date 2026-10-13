//! Prints every schema the web app needs (commands, protocol messages, parameter ids).

fn main() {
    let bundle = protocol::schema_bundle();
    println!(
        "{}",
        serde_json::to_string_pretty(&bundle).expect("a JSON value always serializes")
    );
}
