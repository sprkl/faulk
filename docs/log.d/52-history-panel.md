2026-10-31
* **Update** (#52, PR-7): the history panel (bottom-panel tab "History") lists the undo history with label, author and time. Entries carry an id and a time (`Context::now_ms`, set by the page before each command through `pw_set_clock`); the page core reads them with `pw_history` (`HistoryView`) and runs a command as an agent with `pw_set_author`. Spec § PR-7 notes the model.
