#!/usr/bin/env bun
// Runs the capture matrix through the headless e2e harness and writes
// shots/ plus manifest.json for the gallery server.
//
//   bun build.mjs [--only <substring>] [--jobs N]
//
// Output goes to $MUXTRIX_GALLERY_DIR (default ~/.muxtrix/capture-gallery).

import { mkdir, writeFile, rm, stat, readFile, readdir } from "node:fs/promises";
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import os from "node:os";
import path from "node:path";
import cases from "./matrix.mjs";

const here = import.meta.dir;
const repo = execFileSync("git", ["rev-parse", "--show-toplevel"], {
  cwd: here,
  encoding: "utf8",
}).trim();

// Shots, manifest and review notes live outside the repo and outside any
// session scratchpad, so a review survives both `git clean` and the end of the
// session that produced it.
const out =
  process.env.MUXTRIX_GALLERY_DIR ?? path.join(os.homedir(), ".muxtrix", "capture-gallery");
const shots = path.join(out, "shots");
const profiles = path.join(out, "profiles");

// The harness test binary carries a content hash in its name, so it is found
// rather than hardcoded.
const debugDeps = path.join(repo, "target/debug/deps");
const testBinaries = await Promise.all(
  (await readdir(debugDeps))
    .filter((name) => /^headless_e2e-[0-9a-f]+$/.test(name))
    .map(async (name) => ({
      name,
      modified: (await stat(path.join(debugDeps, name))).mtimeMs,
    })),
);
const testBin = path.join(
  debugDeps,
  testBinaries.sort((a, b) => a.modified - b.modified).at(-1)?.name ?? "headless_e2e-missing",
);

const args = process.argv.slice(2);
const only = args.includes("--only") ? args[args.indexOf("--only") + 1] : null;
const jobs = args.includes("--jobs") ? Number(args[args.indexOf("--jobs") + 1]) : 6;
const retries = 1;

const DEFAULT_SETTINGS = {
  appearance: "system",
  show_status_bar: false,
  ui_font: "system-sans",
  ui_font_weight: "normal",
  ui_font_size: 16.0,
  fleet_view: "tabs",
  terminal_theme: "muxtrix-dark",
  terminal_font: "system-monospace",
  terminal_font_weight: "normal",
  terminal_font_size: 14.0,
  terminal_line_height: 1.15,
  windows_shell_backend: "native",
  wsl_distribution: "",
  codex_command: "codex",
  claude_command: "claude",
  pi_command: "omp",
};

await mkdir(out, { recursive: true });
await mkdir(shots, { recursive: true });
await mkdir(profiles, { recursive: true });
await mkdir(path.join(shots, "logs"), { recursive: true });

const selected = only ? cases.filter((c) => c.slug.includes(only) || c.group.includes(only)) : cases;
if (selected.length === 0) {
  console.error("no cases matched");
  process.exit(2);
}

// Write one settings profile per case that overrides defaults.
for (const entry of selected) {
  if (!entry.settings) continue;
  entry.profilePath = path.join(profiles, `${entry.slug}.json`);
  await writeFile(entry.profilePath, JSON.stringify({ ...DEFAULT_SETTINGS, ...entry.settings }, null, 2));
}

const runOne = (entry) =>
  new Promise((resolve) => {
    const child = spawn(
      path.join(here, "capture-one.sh"),
      [entry.slug, entry.capture, entry.viewport, entry.profilePath ?? ""],
      {
        cwd: repo,
        env: { ...process.env, OUT_DIR: shots, TEST_BIN: testBin },
        stdio: ["ignore", "pipe", "pipe"],
      },
    );
    let err = "";
    child.stderr.on("data", (chunk) => (err += chunk));
    child.on("close", (code) => resolve({ code, err }));
  });

let done = 0;
const results = [];
const queue = [...selected];

const worker = async () => {
  for (;;) {
    const entry = queue.shift();
    if (!entry) return;
    let outcome = null;
    for (let attempt = 0; attempt <= retries; attempt += 1) {
      outcome = await runOne(entry);
      if (outcome.code === 0) break;
    }
    done += 1;
    const ok = outcome.code === 0;
    results.push({ ...entry, ok });
    console.log(
      `${ok ? "ok  " : "FAIL"} [${String(done).padStart(3)}/${selected.length}] ${entry.slug}`,
    );
    if (!ok) console.log(`     ${outcome.err.trim().split("\n").slice(-3).join(" | ")}`);
  }
};

const started = Date.now();
await Promise.all(Array.from({ length: jobs }, worker));

// The manifest always describes the whole matrix, not just this run's slice,
// so re-running one case with --only cannot drop the rest of the gallery.
const manifest = [];
for (const entry of cases) {
  const result = results.find((r) => r.slug === entry.slug);
  const file = path.join(shots, `${entry.slug}.png`);
  let bytes = 0;
  let hash = null;
  try {
    bytes = (await stat(file)).size;
    // The hash is what lets the reviewer tell "I have not looked at this" from
    // "I looked, but the frame has changed since".
    hash = createHash("sha1").update(await readFile(file)).digest("hex").slice(0, 16);
  } catch {
    /* missing */
  }
  manifest.push({
    slug: entry.slug,
    title: entry.title,
    group: entry.group,
    capture: entry.capture === "-" ? "workspace (default)" : entry.capture,
    viewport: entry.viewport,
    settings: entry.settings ?? null,
    check: entry.check,
    // A case this run did not touch keeps whatever it captured last time.
    ok: bytes > 0 && (result ? result.ok : true),
    bytes,
    hash,
    image: `shots/${entry.slug}.png`,
  });
}

await writeFile(
  path.join(out, "manifest.json"),
  JSON.stringify(
    {
      generated: new Date().toISOString(),
      seconds: Math.round((Date.now() - started) / 1000),
      total: manifest.length,
      failed: manifest.filter((m) => !m.ok).length,
      cases: manifest,
    },
    null,
    2,
  ),
);

// Clean up per-case logs for successful runs so failures stay easy to find.
for (const entry of manifest.filter((m) => m.ok)) {
  await rm(path.join(shots, "logs", `${entry.slug}.log`), { force: true });
}

console.log(
  `\n${manifest.filter((m) => m.ok).length}/${manifest.length} captured in ${Math.round((Date.now() - started) / 1000)}s`,
);
if (manifest.some((m) => !m.ok)) {
  console.log("failed:", manifest.filter((m) => !m.ok).map((m) => m.slug).join(", "));
}

console.log(`output: ${out}`);
