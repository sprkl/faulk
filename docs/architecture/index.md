# Architecture

* [Architecture technique](01-architecture.md) - One Rust engine compiled to WebAssembly for the browser and to native code for the helper; every action is a typed command exposed to the UI, to MCP and to ACP agents; the helper is the local bridge for audio, plugins and agents.
