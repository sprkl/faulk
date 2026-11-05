2026-11-05
* **Update** (#560): `specs/03-enregistrement-midi.md` § 8 and § 9: the engine moves a take's notes back by its shift (`SetRecording`), where it knows the loop wraps, so a note heard just before the loop's start on the first pass stays before it; `record.commit_take` folds an explicit `shift_ms` modulo the loop; protocol 38.
