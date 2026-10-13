import assert from "node:assert/strict";
import { describe, it } from "node:test";
import {
  blockReason,
  canDefer,
  missingEvidence,
  requiredEvidence,
  stopDecision,
  verifiedCommand,
} from "./stop-core.mjs";

const verified = { status: "verified", fingerprint: "f1" };
const reviewed = { status: "reviewed", fingerprint: "f1", branch: "feat/6-x" };
const seen = { fingerprint: "f0" };

describe("stopDecision", () => {
  it("allows a turn when the code has no change", () => {
    assert.deepEqual(stopDecision({ fingerprint: null, session: seen }), { allow: true });
  });

  it("allows a turn with a verification and a review of the current state", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f1", session: seen, verification: verified, review: reviewed }), {
      allow: true,
    });
  });

  const cases = [
    ["without any record", {}, ["verification", "review"]],
    ["with the verification only", { verification: verified }, ["review"]],
    ["with the review only", { review: reviewed }, ["verification"]],
    [
      "with records of an earlier state",
      { verification: { ...verified, fingerprint: "f0" }, review: { ...reviewed, fingerprint: "f0", branch: "develop" } },
      ["verification", "review"],
    ],
    ["with a record of the wrong status", { verification: { ...verified, status: "deferred" }, review: reviewed }, ["verification"]],
  ];
  for (const [label, records, missing] of cases) {
    it(`blocks ${label}`, () => {
      assert.deepEqual(stopDecision({ fingerprint: "f1", session: seen, branch: "develop", ...records }), {
        allow: false,
        missing,
      });
    });
  }

  it("lets exactly one stop through with a deferral, then blocks again", () => {
    const deferral = { status: "deferred", fingerprint: "f1", reason: "question" };
    assert.deepEqual(stopDecision({ fingerprint: "f1", session: seen, deferral }), { allow: true, consumeDeferral: true });
    assert.equal(
      stopDecision({ fingerprint: "f1", session: seen, deferral: { ...deferral, consumedAt: "2026-10-13T12:00:00Z" } }).allow,
      false,
    );
  });

  it("refuses a deferral of an earlier state", () => {
    const deferral = { status: "deferred", fingerprint: "f0" };
    assert.equal(stopDecision({ fingerprint: "f1", session: seen, deferral }).allow, false);
  });
});

describe("stopDecision: one delivery review per task branch", () => {
  const review = { status: "reviewed", fingerprint: "f1", branch: "feat/6-x" };
  const laterVerification = { ...verified, fingerprint: "f2" };

  it("keeps the review for a later state of the same task branch once it is verified", () => {
    assert.deepEqual(
      stopDecision({ fingerprint: "f2", session: seen, branch: "feat/6-x", verification: laterVerification, review }),
      { allow: true },
    );
  });

  it("still requires the verification of the later state", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f2", session: seen, branch: "feat/6-x", verification: verified, review }), {
      allow: false,
      missing: ["verification"],
    });
  });

  it("requires a review on a branch that has none (another branch's review does not count)", () => {
    assert.deepEqual(
      stopDecision({ fingerprint: "f2", session: seen, branch: "feat/7-y", verification: laterVerification, review }),
      { allow: false, missing: ["review"] },
    );
  });

  for (const branch of ["develop", "main", "HEAD", undefined]) {
    it(`never carries a review over on ${branch}`, () => {
      const decision = stopDecision({
        fingerprint: "f2",
        session: seen,
        branch,
        verification: laterVerification,
        review: { ...review, branch },
      });
      assert.equal(decision.allow, false);
    });
  }
});

describe("stopDecision: a session is judged only on what it changed", () => {
  it("allows a session that left the code as the hooks first found it, even unverified", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f1", session: { fingerprint: "f1" } }), { allow: true });
  });

  it("allows a session the hooks never saw, and asks to record its baseline", () => {
    assert.deepEqual(stopDecision({ fingerprint: "f1", session: null }), { allow: true, recordBaseline: true });
  });

  it("judges a session that changed the code", () => {
    assert.equal(stopDecision({ fingerprint: "f2", session: { fingerprint: "f1" } }).allow, false);
  });

  it("judges a session with an unknown or unreadable baseline, and one without a session id", () => {
    assert.equal(stopDecision({ fingerprint: "f1", session: { fingerprint: "unknown" } }).allow, false);
    assert.equal(stopDecision({ fingerprint: "f1", session: {} }).allow, false);
    assert.equal(stopDecision({ fingerprint: "f1", session: undefined }).allow, false);
  });
});

describe("canDefer", () => {
  const consumed = { fingerprint: "f1", status: "deferred", consumedAt: "2026-10-13T12:00:00.000Z" };

  it("allows a first deferral", () => {
    assert.equal(canDefer(null, "f1"), true);
  });

  it("refuses a second deferral of the same state", () => {
    assert.equal(canDefer(consumed, "f1"), false);
    assert.equal(canDefer(consumed, "f1", "2026-10-13T11:00:00.000Z"), false);
  });

  it("allows a new deferral after the user wrote again", () => {
    assert.equal(canDefer(consumed, "f1", "2026-10-13T12:05:00.000Z"), true);
  });

  it("allows a deferral after a change of the code", () => {
    assert.equal(canDefer(consumed, "f2"), true);
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
