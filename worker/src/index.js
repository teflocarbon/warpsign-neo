// WarpSign's install manifests, for web installs on iOS 27 and later.
//
// iOS only installs over the air from a manifest on trusted HTTPS, and
// since iOS 27 the manifest's bundle-identifier must match the app. So
// this answers each install with a manifest naming the app, while the app
// itself still comes from WarpSign's server on the phone's loopback. No
// storage and no logging: the request is answered and forgotten.
//
// GET /manifest?slot=42&port=14310&id=com.example.app&title=Example&version=1.0

const BUNDLE_ID = /^[A-Za-z0-9.-]{1,255}$/;
const VERSION = /^[A-Za-z0-9.+-]{1,64}$/;

export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (request.method !== "GET" && request.method !== "HEAD") {
      return text(405, "Method not allowed");
    }
    if (url.pathname !== "/manifest") {
      return text(404, "WarpSign install manifests: GET /manifest");
    }
    const result = parse(url.searchParams);
    if (result.error) {
      return text(400, result.error);
    }
    return new Response(request.method === "HEAD" ? null : manifest(result), {
      headers: {
        "Content-Type": "application/xml",
        "Cache-Control": "no-store",
      },
    });
  },
};

/// The install's details, checked; or an error
export function parse(params) {
  const slot = integer(params.get("slot"), 0, 999_999);
  const port = integer(params.get("port"), 1024, 65_535);
  const id = params.get("id") ?? "";
  const title = params.get("title") ?? "";
  const version = params.get("version") ?? "";
  if (slot === null) return { error: "slot: expected a number" };
  if (port === null) return { error: "port: expected 1024 to 65535" };
  if (!BUNDLE_ID.test(id)) return { error: "id: expected a bundle ID" };
  if (title.length < 1 || title.length > 100) return { error: "title: expected 1 to 100 characters" };
  if (!VERSION.test(version)) return { error: "version: expected a version" };
  return { slot, port, id, title, version };
}

function integer(value, min, max) {
  if (value === null || !/^\d{1,6}$/.test(value)) return null;
  const number = Number(value);
  return number >= min && number <= max ? number : null;
}

/// The manifest, byte for byte what warpsign-core's serve::manifest makes
/// (both are checked against test/manifest.plist). Assets always point at
/// loopback, so this can't be used to offer apps from anywhere else.
export function manifest({ slot, port, id, title, version }) {
  const base = `http://127.0.0.1:${port}/app${slot}`;
  const asset = (kind, url) => {
    const shine = kind === "software-package" ? "" : "\t\t\t\t\t<key>needs-shine</key>\n\t\t\t\t\t<true/>\n";
    return `\t\t\t\t<dict>\n\t\t\t\t\t<key>kind</key>\n\t\t\t\t\t<string>${kind}</string>\n${shine}\t\t\t\t\t<key>url</key>\n\t\t\t\t\t<string>${escape(url)}</string>\n\t\t\t\t</dict>\n`;
  };
  return (
    `<?xml version="1.0" encoding="UTF-8"?>\n` +
    `<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n` +
    `<plist version="1.0">\n<dict>\n\t<key>items</key>\n\t<array>\n\t\t<dict>\n\t\t\t<key>assets</key>\n\t\t\t<array>\n` +
    asset("software-package", `${base}.ipa`) +
    asset("display-image", `${base}.png`) +
    asset("full-size-image", `${base}.png`) +
    `\t\t\t</array>\n\t\t\t<key>metadata</key>\n\t\t\t<dict>\n` +
    `\t\t\t\t<key>bundle-identifier</key>\n\t\t\t\t<string>${escape(id)}</string>\n` +
    `\t\t\t\t<key>bundle-version</key>\n\t\t\t\t<string>${escape(version)}</string>\n` +
    `\t\t\t\t<key>kind</key>\n\t\t\t\t<string>software</string>\n` +
    `\t\t\t\t<key>title</key>\n\t\t\t\t<string>${escape(title)}</string>\n` +
    `\t\t\t</dict>\n\t\t</dict>\n\t</array>\n</dict>\n</plist>`
  );
}

// As the plist crate escapes, so the two stay byte for byte the same
function escape(text) {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/'/g, "&apos;")
    .replace(/"/g, "&quot;");
}

function text(status, body) {
  return new Response(body + "\n", {
    status,
    headers: { "Content-Type": "text/plain", "Cache-Control": "no-store" },
  });
}
