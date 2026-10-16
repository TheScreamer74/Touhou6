// Interactive Firefox driver for Touhou6 #7 journeys (synthetic profile only).
// Usage: node driver.mjs <outdir> [--no-community]
// Control: curl localhost:9377/<cmd>?...  (key, down, up, wait, shot, eval, log, reload, file)
import { firefox } from "playwright";
import http from "node:http";
import fs from "node:fs";

const OUT = process.argv[2];
const NO_COMMUNITY = process.argv.includes("--no-community");
fs.mkdirSync(OUT, { recursive: true });
const ctx = await firefox.launchPersistentContext(OUT + "/profile", {
  headless: true, viewport: { width: 1280, height: 960 }, acceptDownloads: true,
});
if (NO_COMMUNITY) await ctx.route("**/platform/community.js*", r => r.fulfill({ status: 404, body: "" }));
// Wrap the page's submit hook so every payload the engine emits is logged with a timestamp.
await ctx.addInitScript(() => {
  window.__t7Results = [];
  let inner = null;
  Object.defineProperty(window, "__th06SubmitScore", {
    configurable: true,
    get() { return p => { window.__t7Results.push({ t: Date.now(), payload: p }); return inner && inner(p); }; },
    set(f) { inner = f; },
  });
});
const page = ctx.pages()[0] || await ctx.newPage();
const errors = [];
page.on("pageerror", e => errors.push(String(e)));
page.on("console", m => { if (m.type() === "error") errors.push(m.text()); });
const wait = ms => page.waitForTimeout(ms);
const load = async () => {
  await page.goto("https://games.blackoutburst.com/th06/", { waitUntil: "load" });
  await page.waitForSelector("body.playing", { timeout: 300000 });
  await page.locator("canvas").first().click({ position: { x: 5, y: 5 } }).catch(() => {});
};
await load();
let shotN = 0;
const handlers = {
  async key(q) { const n = +(q.get("n") || 1), gap = +(q.get("gap") || 120);
    for (let i = 0; i < n; i++) { await page.keyboard.press(q.get("k"), { delay: +(q.get("hold") || 40) }); await wait(gap); } return "ok"; },
  async down(q) { for (const k of q.get("k").split(",")) await page.keyboard.down(k); return "ok"; },
  async up(q) { for (const k of q.get("k").split(",")) await page.keyboard.up(k); return "ok"; },
  async wait(q) { await wait(+q.get("ms")); return "ok"; },
  async shot(q) { const p = `${OUT}/${String(++shotN).padStart(3, "0")}-${q.get("name") || "s"}.png`;
    await page.locator("canvas").first().screenshot({ path: p }); return p; },
  async eval(q, body) { return JSON.stringify(await page.evaluate(body)); },
  async log() { return JSON.stringify({ results: await page.evaluate(() => window.__t7Results), errors }); },
  async reload() { await load(); return "ok"; },
  async file(q) { await page.setInputFiles("#" + q.get("id"), q.get("path")); return "ok"; },
  async click(q) { await page.click(q.get("sel")); return "ok"; },
  async download(q) { const [d] = await Promise.all([page.waitForEvent("download"), page.click(q.get("sel"))]);
    const p = `${OUT}/${d.suggestedFilename()}`; await d.saveAs(p); return p; },
  async quit() { setTimeout(async () => { await ctx.close(); process.exit(0); }, 100); return "bye"; },
};
http.createServer(async (req, res) => {
  const u = new URL(req.url, "http://x");
  let body = ""; for await (const c of req) body += c;
  try { res.end(String(await handlers[u.pathname.slice(1)](u.searchParams, body)) + "\n"); }
  catch (e) { res.statusCode = 500; res.end(String(e) + "\n"); }
}).listen(9377, () => console.log("driver ready"));
