# Architecture

* [Architecture technique](01-architecture.md) - One Rust engine compiled to WebAssembly for the browser and to native code for the helper; every action is a typed command exposed to the UI, to MCP and to ACP agents; the helper is the local bridge for audio, plugins and agents.
* [Modèle de projet et format sur disque](02-modele-projet.md) - Proposal for the Faulk project model and its on-disk format: a folder grouped by track with canonical text for human-scale data and content-addressed blobs for machine-scale data, a semantic three-way merge shared by git, the browser and the helper, in-app versions and variants, and the strategy for very large projects.
* [Stockage local du projet et mémoire audio](03-stockage-local.md) - Proposal for keeping a Faulk project and its audio on disk (user folder or OPFS) rather than in browser memory: current browser limits, disk streaming into the AudioWorklet, the Boost-mode impact, atomic writes, failure modes and an MVP proposal.
