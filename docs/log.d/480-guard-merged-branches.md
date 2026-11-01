2026-11-02
* **Update**: the end-of-turn guard no longer warns about a branch merged into `develop` then deleted: the session marker keeps the branch's last known commit and is dropped silently when that commit is in `origin/develop`; other deleted branches keep one warning per distinct text (#480). `agents/verification.md` > Limits follows.
