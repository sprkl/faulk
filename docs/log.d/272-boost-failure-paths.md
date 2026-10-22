2026-10-22
* **Update** (#272): three defects on Boost's failure paths. A failed connection no longer pushes a snapshot into the Web engine that kept playing (it cut a held note); a failure after the helper accepted the session now closes that session, and keeps the pairing token; a late `audio.stream` error or close from a helper session that is no longer current is ignored.
