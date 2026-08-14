# Muxtrix capture gallery

183 real rendered Muxtrix frames, captured headlessly through the e2e harness
(Xvfb + llvmpipe, no visible window), plus a local review UI.

Shots, `manifest.json` and your review notes live in
`~/.muxtrix/capture-gallery` (override with `MUXTRIX_GALLERY_DIR`), outside the
repo — a review survives `git clean` and the session that produced it.

## Review them

```bash
cd scripts/capture-gallery
bun server.mjs            # → http://localhost:5173
```

- `/` filter by group, search, or verdict; card size selector top-right.
- Click any frame for the full-size view, the state it was captured in, and
  the question that frame is meant to answer.
- `1` marks a frame right, `2` flags it, and the note box records why. Verdicts
  persist to `notes.json` — nothing is stored in a browser.
- The gallery opens on **Needs review**: frames never looked at, plus frames
  whose pixels changed since the verdict was recorded. Everything already
  marked OK or flagged against the current image stays hidden until you switch
  the filter, so a second pass only shows what actually needs one.
- `←`/`→` walk the current filter; `z` toggles 1:1 zoom; `/` focuses search.

## Recapture

```bash
bun build.mjs                       # whole matrix, 5–6 parallel jobs
bun build.mjs --only theme --jobs 4 # one group or slug substring
bun build.mjs --jobs 2              # slower machines / under load
```

`build.mjs` writes a settings profile per case into `profiles/`, runs
`../capture-one.sh` for each, and rebuilds `manifest.json` over the whole
matrix — so `--only` re-runs a slice without dropping the rest.

Each capture boots the real binary and drives the full interaction scenario
(~6 s). A case failing means the run failed, not that the picture is ugly:
check `shots/logs/<slug>.log`.

## What is in the matrix

`matrix.mjs` is the single source of truth. Three axes:

- **capture state** — `MUXTRIX_E2E_CAPTURE`, one branch of
  `Scenario::stage_capture` in `crates/muxtrix-app/src/e2e.rs`
- **viewport** — 720×480 (minimum supported) through 1920×1080
- **settings profile** — `MUXTRIX_E2E_SETTINGS`, seeded before boot; this is
  how themes, fonts, weights, type sizes, appearance and fleet view are pinned

Adding a state means adding a branch to `stage_capture` and a row to
`matrix.mjs`. Adding a variation of an existing state only needs the row.
