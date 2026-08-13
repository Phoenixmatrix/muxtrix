#!/usr/bin/env bun
// Local review gallery for the Muxtrix headless capture matrix.
//
//   bun server.mjs [--port 5173]

import { readFile, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const here = import.meta.dir;
const args = process.argv.slice(2);
const port = args.includes("--port") ? Number(args[args.indexOf("--port") + 1]) : 5173;
// Shots and review notes live where build.mjs put them, outside the repo, so a
// review survives `git clean` and the session that produced it.
const out =
  process.env.MUXTRIX_GALLERY_DIR ?? path.join(os.homedir(), ".muxtrix", "capture-gallery");
const notesPath = path.join(out, "notes.json");

const readManifest = async () => JSON.parse(await readFile(path.join(out, "manifest.json"), "utf8"));
const readNotes = async () => {
  try {
    return JSON.parse(await readFile(notesPath, "utf8"));
  } catch {
    return {};
  }
};

const page = async () => {
  const manifest = await readManifest();
  const notes = await readNotes();
  const html = await readFile(path.join(here, "index.html"), "utf8");
  return html.replace(
    "/*__DATA__*/",
    `window.__GALLERY__ = ${JSON.stringify({ ...manifest, notes })};`,
  );
};

const server = Bun.serve({
  port,
  async fetch(request) {
    const url = new URL(request.url);

    if (request.method === "POST" && url.pathname === "/api/notes") {
      const body = await request.json();
      const notes = await readNotes();
      if (body.note === "" && body.verdict == null) delete notes[body.slug];
      else {
        notes[body.slug] = {
          verdict: body.verdict ?? null,
          note: body.note ?? "",
          // Stamped with the frame the verdict was formed against, so a later
          // recapture can show that this one moved and needs another look.
          hash: body.hash ?? null,
          at: new Date().toISOString(),
        };
      }
      await writeFile(notesPath, JSON.stringify(notes, null, 2));
      return Response.json({ ok: true });
    }

    if (url.pathname === "/api/notes") return Response.json(await readNotes());
    if (url.pathname === "/api/manifest") return Response.json(await readManifest());

    if (url.pathname === "/" || url.pathname === "/index.html") {
      return new Response(await page(), {
        headers: { "content-type": "text/html; charset=utf-8" },
      });
    }

    // Static files. Page assets come from the skill directory, shots from the
    // output directory; nothing outside either is reachable.
    const relative = decodeURIComponent(url.pathname).replace(/^\/+/, "");
    const root = relative.startsWith("shots/") ? out : here;
    const requested = path.normalize(path.join(root, relative));
    if (!requested.startsWith(root)) return new Response("no", { status: 403 });
    const file = Bun.file(requested);
    if (await file.exists()) return new Response(file);
    return new Response("not found", { status: 404 });
  },
});

console.log(`Muxtrix capture gallery → http://localhost:${server.port}`);
console.log(`reading ${out}`);
