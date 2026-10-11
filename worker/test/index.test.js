import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

import worker, { manifest, parse } from "../src/index.js";

const fixture = readFileSync(new URL("./manifest.plist", import.meta.url), "utf8");
const example = {
  slot: "42",
  port: "14310",
  id: "com.example.app",
  title: `Tom's <Fish> & "Chips" 🐟`,
  version: "2.1+beta",
};

function request(params, method = "GET", path = "/manifest") {
  const url = new URL(`https://warpsign.example.workers.dev${path}`);
  for (const [key, value] of Object.entries(params)) url.searchParams.set(key, value);
  return worker.fetch(new Request(url, { method }));
}

test("matches warpsign-core's manifest byte for byte", () => {
  assert.equal(manifest(parse(new URLSearchParams(example))), fixture);
});

test("serves the manifest", async () => {
  const response = await request(example);
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("Content-Type"), "application/xml");
  assert.equal(response.headers.get("Cache-Control"), "no-store");
  assert.equal(await response.text(), fixture);
});

test("decodes the link warpsign-core makes", async () => {
  // From serve::install_link: itms-services decodes once, the URL once more
  const link =
    "itms-services://?action=download-manifest&url=https%3A%2F%2Fwarpsign.example.workers.dev%2Fmanifest%3Fslot%3D7%26port%3D14310%26id%3Dcom.example.app%26title%3DTom%2527s%2B%253CFish%253E%2B%2526%2B%2522Chips%2522%2B%25F0%259F%2590%259F%26version%3D2.1%252Bbeta";
  const manifestUrl = new URLSearchParams(link.split("?")[1]).get("url");
  const response = await worker.fetch(new Request(manifestUrl));
  assert.equal(response.status, 200);
  const body = await response.text();
  assert.match(body, /<string>Tom&apos;s &lt;Fish&gt; &amp; &quot;Chips&quot; 🐟<\/string>/);
  assert.match(body, /<string>2\.1\+beta<\/string>/);
  assert.match(body, /http:\/\/127\.0\.0\.1:14310\/app7\.ipa/);
});

test("answers HEAD without a body", async () => {
  const response = await request(example, "HEAD");
  assert.equal(response.status, 200);
  assert.equal(await response.text(), "");
});

test("only ever points at loopback", async () => {
  const body = await (await request(example)).text();
  const urls = [...body.matchAll(/<string>(https?:[^<]*)<\/string>/g)].map((m) => m[1]);
  assert.equal(urls.length, 3);
  for (const url of urls) assert.ok(url.startsWith("http://127.0.0.1:14310/"), url);
});

test("rejects bad input", async () => {
  const cases = [
    [{ slot: "x" }, "slot"],
    [{ slot: "-1" }, "slot"],
    [{ slot: "1000000" }, "slot"],
    [{ port: "80" }, "port"],
    [{ port: "65536" }, "port"],
    [{ port: "14310@evil.example" }, "port"],
    [{ id: "" }, "id"],
    [{ id: "com.example/../x" }, "id"],
    [{ id: "a".repeat(256) }, "id"],
    [{ title: "" }, "title"],
    [{ title: "x".repeat(101) }, "title"],
    [{ version: "" }, "version"],
    [{ version: "1.0<x>" }, "version"],
  ];
  for (const [change, field] of cases) {
    const response = await request({ ...example, ...change });
    assert.equal(response.status, 400, JSON.stringify(change));
    assert.match(await response.text(), new RegExp(`^${field}:`));
  }
});

test("answers other paths and methods", async () => {
  assert.equal((await request({}, "GET", "/")).status, 404);
  assert.equal((await request(example, "POST")).status, 405);
});
