2026-11-06
* **Update**: the decoded size of an audio import is capped at `project::limits::MAX_DECODED_BYTES` (16 GiB of `f32`), refused as `too_large` from the header or by the block that would pass it (#780); `architecture/03-stockage-local.md` § 5.9 says why and what was measured.
