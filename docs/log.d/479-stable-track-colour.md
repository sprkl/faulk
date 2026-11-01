2026-11-02
* **Update** (#479): a track keeps its colour. `track.create` and `track.duplicate` store the next free hue of the 16 DS-0.1 hues; opening a project colours, in memory and without history, each track that has none with the hue of its position (`project::colors`, the same way clips are named). `track.set_color(null)` still clears a colour: the track then follows its position again.
