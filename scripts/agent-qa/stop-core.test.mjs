import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  RECORD_LIMIT,
  addRecord,
  blockReason,
  exemptAgentMarker,
  isAgentWorktreePath,
  canDefer,
  missingEvidence,
  requiredEvidence,
  stopDecision,
  verifiedCommand,
} from "./stop-core.mjs";
import { wroteDuringCall } from "./session.mjs";

const verified = { status: "verified", fingerprint: "f1" };
const reviewed = { status: "reviewed", fingerprint: "f1", branch: "feat/6-x" };


describe("stopDecision", () => {
  it("allows a turn when the code has no change", () => {
    assert.deepEqual(stopDecision({ fingerprint: null, writer: true, baseline: "f0" }), { allow: true });
  });

  it("allows a turn with a verification and a review of the current state", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f1", writer: true, baseline: "f0", verifications: [verified], reviews: [reviewed] }), {
      allow: true,
    });
  });

  const cases = [
    ["without any record", {}, ["verification", "review"]],
    ["with the verification only", { verifications: [verified] }, ["review"]],
    ["with the review only", { reviews: [reviewed] }, ["verification"]],
    [
      "with records of an earlier state",
      { verifications: [{ ...verified, fingerprint: "f0" }], reviews: [{ ...reviewed, fingerprint: "f0", branch: "feat/7-y" }] },
      ["verification", "review"],
    ],
    ["with a record of the wrong status", { verifications: [{ ...verified, status: "deferred" }], reviews: [reviewed] }, ["verification"]],
  ];
  for (const [label, records, missing] of cases) {
    it(`blocks ${label}`, () => {
      assert.deepEqual(stopDecision({ fingerprint: "f1", writer: true, baseline: "f0", branch: "develop", ...records }), {
        allow: false,
        missing,
      });
    });
  }

  it("lets exactly one stop through with a deferral, then blocks again", () => {
    const deferral = { status: "deferred", fingerprint: "f1", reason: "question" };
    assert.deepEqual(stopDecision({ fingerprint: "f1", writer: true, baseline: "f0", deferral }), { allow: true, consumeDeferral: true });
    assert.equal(
      stopDecision({ fingerprint: "f1", writer: true, baseline: "f0", deferral: { ...deferral, consumedAt: "2026-10-13T12:00:00Z" } }).allow,
      false,
    );
  });

  it("refuses a deferral of an earlier state", () => {
    const deferral = { status: "deferred", fingerprint: "f0" };
    assert.equal(stopDecision({ fingerprint: "f1", writer: true, baseline: "f0", deferral }).allow, false);
  });
});

describe("stopDecision: one delivery review per task branch", () => {
  const review = { status: "reviewed", fingerprint: "f1", branch: "feat/6-x" };
  const laterVerification = { ...verified, fingerprint: "f2" };

  it("keeps the review for a later state of the same task branch once it is verified", () => {
    assert.deepEqual(
      stopDecision({ fingerprint: "f2", writer: true, baseline: "f0", branch: "feat/6-x", verifications: [laterVerification], reviews: [review] }),
      { allow: true },
    );
  });

  it("still requires the verification of the later state", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f2", writer: true, baseline: "f0", branch: "feat/6-x", verifications: [verified], reviews: [review] }), {
      allow: false,
      missing: ["verification"],
    });
  });

  it("requires a review on a branch that has none (another branch's review does not count)", () => {
    assert.deepEqual(
      stopDecision({ fingerprint: "f2", writer: true, baseline: "f0", branch: "feat/7-y", verifications: [laterVerification], reviews: [review] }),
      { allow: false, missing: ["review"] },
    );
  });

  for (const branch of ["develop", "main", "HEAD", "wip", "feat/no-issue", undefined]) {
    it(`never carries a review over on ${branch}`, () => {
      const decision = stopDecision({
        fingerprint: "f2",
        writer: true, baseline: "f0",
        branch,
        verifications: [laterVerification],
        reviews: [{ ...review, branch }],
      });
      assert.equal(decision.allow, false);
    });
  }
});

describe("stopDecision: the records of every branch of a checkout (#492)", () => {
  const a = { verified: { status: "verified", fingerprint: "fa", branch: "feat/6-x" }, reviewed: { status: "reviewed", fingerprint: "fa", branch: "feat/6-x" } };
  const b = { verified: { status: "verified", fingerprint: "fb", branch: "feat/7-y" }, reviewed: { status: "reviewed", fingerprint: "fb", branch: "feat/7-y" } };
  const records = { verifications: [a.verified, b.verified], reviews: [a.reviewed, b.reviewed] };

  it("finds the records of the judged state among those of other branches, in any order", () => {
    for (const [fingerprint, branch] of [["fa", "feat/6-x"], ["fb", "feat/7-y"]]) {
      assert.deepEqual(stopDecision({ fingerprint, writer: true, branch, ...records }), { allow: true });
      assert.deepEqual(
        stopDecision({
          fingerprint,
          writer: true,
          branch,
          verifications: [...records.verifications].reverse(),
          reviews: [...records.reviews].reverse(),
        }),
        { allow: true },
      );
    }
  });

  it("carries a branch's review over to its later state while another branch's records are newer", () => {
    const later = { status: "verified", fingerprint: "fa2", branch: "feat/6-x" };
    assert.deepEqual(
      stopDecision({ fingerprint: "fa2", writer: true, branch: "feat/6-x", verifications: [a.verified, later, b.verified], reviews: records.reviews }),
      { allow: true },
    );
  });

  it("does not take another branch's records for this state", () => {
    assert.deepEqual(stopDecision({ fingerprint: "fc", writer: true, branch: "feat/8-z", ...records }), {
      allow: false,
      missing: ["verification", "review"],
    });
  });

  it("says when a review of this branch exists but is not carried over (a branch named outside the convention)", () => {
    for (const branch of ["fix/po-ui-only", "develop"]) {
      const decision = stopDecision({
        fingerprint: "f2",
        writer: true,
        branch,
        verifications: [{ status: "verified", fingerprint: "f2" }],
        reviews: [{ status: "reviewed", fingerprint: "f1", branch }],
      });
      assert.deepEqual(decision, { allow: false, missing: ["review"], reviewNotCarried: true });
      const reason = blockReason({ root: "/r", fingerprint: "f2", files: ["web/a.ts"], ...decision, branch });
      assert.match(reason, new RegExp(`review of an earlier state of ${branch} exists, but it is not carried over`));
      assert.doesNotMatch(reason, /has no delivery review yet/);
    }
    // Another branch's review, or none: the usual message.
    const other = stopDecision({ fingerprint: "f2", writer: true, branch: "fix/po-ui-only", reviews: [{ status: "reviewed", fingerprint: "f1", branch: "wip" }] });
    assert.equal(other.reviewNotCarried, undefined);
    assert.match(blockReason({ root: "/r", fingerprint: "f2", files: ["web/a.ts"], ...other }), /has no delivery review yet/);
  });
});

describe("addRecord", () => {
  const entry = (fingerprint, branch = "feat/6-x", status = "verified") => ({ status, fingerprint, branch });

  it("appends, and moves a state recorded again to the end", () => {
    assert.deepEqual(addRecord([], entry("f1")), [entry("f1")]);
    assert.deepEqual(addRecord(undefined, entry("f1")), [entry("f1")]);
    const again = { ...entry("f1"), at: "later" };
    assert.deepEqual(addRecord([entry("f1"), entry("f2")], again), [entry("f2"), again]);
  });

  it("keeps the same state of two branches apart", () => {
    assert.deepEqual(addRecord([entry("f1", "feat/6-x")], entry("f1", "feat/7-y")), [entry("f1", "feat/6-x"), entry("f1", "feat/7-y")]);
  });

  it("keeps the last `limit` entries, oldest dropped", () => {
    let list = [];
    for (let i = 0; i < RECORD_LIMIT + 5; i++) {
      list = addRecord(list, entry(`f${i}`));
    }
    assert.equal(list.length, RECORD_LIMIT);
    assert.equal(list[0].fingerprint, "f5");
    assert.equal(list.at(-1).fingerprint, `f${RECORD_LIMIT + 4}`);
    assert.deepEqual(addRecord([entry("a"), entry("b"), entry("c")], entry("d"), 2), [entry("c"), entry("d")]);
  });
});

describe("stopDecision: a session is judged only on what it changed", () => {
  it("allows a session none of whose tool calls changed the code, whatever the state", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f1", writer: false, baseline: "f0" }), { allow: true });
    assert.deepEqual(stopDecision({ fingerprint: "f1", writer: false }), { allow: true });
  });

  it("allows a writer that undid its own changes", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f1", writer: true, baseline: "f1" }), { allow: true });
  });

  it("judges a writer with an unknown or missing baseline", () => {
    assert.equal(stopDecision({ fingerprint: "f1", writer: true, baseline: "unknown" }).allow, false);
    assert.equal(stopDecision({ fingerprint: "f1", writer: true }).allow, false);
  });
});

describe("wroteDuringCall", () => {
  it("compares the fingerprint after the call with the one before it", () => {
    assert.equal(wroteDuringCall({ before: "f1", baseline: "f0", after: "f1" }), false);
    assert.equal(wroteDuringCall({ before: "f1", after: "f2" }), true);
    assert.equal(wroteDuringCall({ before: null, after: "f2" }), true);
    assert.equal(wroteDuringCall({ before: "f1", after: null }), true);
  });

  it("falls back on the baseline, then counts any change as the session's", () => {
    assert.equal(wroteDuringCall({ baseline: "f2", after: "f2" }), false);
    assert.equal(wroteDuringCall({ baseline: "f1", after: "f2" }), true);
    assert.equal(wroteDuringCall({ after: "f2" }), true);
    assert.equal(wroteDuringCall({ after: null }), false);
    assert.equal(wroteDuringCall({ before: "unknown", after: "f2" }), true);
  });

  it("never marks a writer when the fingerprint after cannot be computed", () => {
    assert.equal(wroteDuringCall({ before: "f1", after: "unknown" }), false);
  });
});

describe("canDefer", () => {
  const consumed = { fingerprint: "f1", status: "deferred", consumedAt: "2026-10-13T12:00:00.000Z" };

  it("allows a first deferral, and one never consumed", () => {
    assert.equal(canDefer(null), true);
    assert.equal(canDefer({ fingerprint: "f1", status: "deferred" }), true);
  });

  it("refuses another one until the user writes again, whatever the code", () => {
    assert.equal(canDefer(consumed), false);
    assert.equal(canDefer(consumed, "2026-10-13T11:00:00.000Z"), false);
  });

  it("allows a new deferral after the user wrote again", () => {
    assert.equal(canDefer(consumed, "2026-10-13T12:05:00.000Z"), true);
  });
});

describe("requiredEvidence", () => {
  const cases = [
    [["crates/engine/src/lib.rs"], ["rust", "render"]],
    [["crates/engine-wasm/src/lib.rs"], ["rust", "render", "latency"]],
    [["crates/commands/src/lib.rs"], ["rust", "commands"]],
    [["crates/helper/src/main.rs"], ["rust", "commands", "latency"]],
    [["crates/protocol/src/lib.rs"], ["rust", "latency"]],
    [["Cargo.lock"], ["rust"]],
    [["modules/reverb/reverb.dsp"], ["render"]],
    [["modules/osc/src/lib.rs"], ["rust", "render"]],
    [["web/src/App.ts"], ["web", "browser", "exploration"]],
    [["web/src/audio/worklet.ts"], ["web", "browser", "exploration", "latency"]],
    [["site/src/index.html"], ["web", "browser", "exploration"]],
    [["scripts/agent-qa/record.mjs"], ["scripts"]],
    [[".claude/settings.json"], ["scripts"]],
    [["crates\\engine\\src\\lib.rs"], ["rust", "render"]],
    [[], []],
  ];
  for (const [files, flags] of cases) {
    it(`asks ${flags.join(", ") || "nothing"} for ${files.join(", ") || "no file"}`, () => {
      assert.deepEqual(requiredEvidence(files), flags);
    });
  }

  it("reports missing or blank evidence", () => {
    assert.deepEqual(missingEvidence(["crates/engine/src/lib.rs"], { rust: "ok", render: "  " }), ["render"]);
    assert.deepEqual(missingEvidence(["crates/engine/src/lib.rs"], { rust: "ok", render: "ok" }), []);
  });
});

describe("messages", () => {
  it("names the exact command with the flags the changed files require", () => {
    const command = verifiedCommand(["crates/engine/src/lib.rs"]);
    assert.match(command, /^node scripts\/agent-qa\/record\.mjs verified --rust "<.+>" --render "<.+>"$/);
  });

  it("gives a command that can be pasted: no double quote inside an argument", () => {
    const command = verifiedCommand(["crates/helper/src/a.rs", "web/src/worklet.ts", "modules/x.dsp", "scripts/a.mjs"]);
    const args = [...command.matchAll(/ --(\w+) "<[^"]+>"/g)].map((m) => m[1]);
    assert.deepEqual(args, ["rust", "render", "commands", "web", "browser", "exploration", "scripts", "latency"]);
    assert.equal(command.replace(/ --\w+ "<[^"]+>"/g, ""), "node scripts/agent-qa/record.mjs verified");
  });

  it("tells the agent what to run for each missing record and how to defer", () => {
    const reason = blockReason({ root: "/repo", fingerprint: "f1", files: ["web/src/a.ts"], missing: ["verification", "review"] });
    assert.match(reason, /\/repo/);
    assert.match(reason, /record\.mjs verified --web/);
    assert.match(reason, /--browser/);
    assert.match(reason, /brief\.mjs <issue> delivery/);
    assert.match(reason, /run\.mjs \.scratch\/judge-dredd\/<issue>-delivery\.md/);
    assert.match(reason, /record\.mjs reviewed --brief/);
    assert.match(reason, /record\.mjs deferred --reason/);
  });
});

describe("subagent worktrees (#392)", () => {
  const yes = { marker: { agent: "a1" }, running: true, agentWorktree: true, sameRepository: true, leadRoot: false };

  it("recognizes the folder Claude Code gives an isolated subagent, and nothing else", () => {
    for (const ok of ["C:/p/.claude/worktrees/agent-a958391d8b68fbdfa", "/p/.claude/worktrees/agent-x1/", "C:\\p\\.claude\\worktrees\\agent-x1"]) {
      assert.equal(isAgentWorktreePath(ok), true, ok);
    }
    for (const no of [
      "/p",
      "/p/.claude/worktrees/feat-x",
      "/p/.claude/worktrees/agent-x/sub",
      "/p/worktrees/agent-x",
      "/p/.claude/agent-x",
      "/p/.claude/worktrees/agent-",
      "/p/.claude/worktrees/agentx",
    ]) {
      assert.equal(isAgentWorktreePath(no), false, no);
    }
  });

  it("exempts a marker only when every condition holds", () => {
    assert.equal(exemptAgentMarker(yes), true);
    assert.equal(exemptAgentMarker({ ...yes, marker: {} }), false, "the lead's own marker");
    assert.equal(exemptAgentMarker({ ...yes, marker: undefined }), false);
    for (const key of ["running", "agentWorktree", "sameRepository"]) {
      assert.equal(exemptAgentMarker({ ...yes, [key]: false }), false, key);
    }
    assert.equal(exemptAgentMarker({ ...yes, leadRoot: true }), false, "a checkout the lead works in");
  });
});
