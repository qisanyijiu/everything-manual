import fs from "node:fs";
import http from "node:http";
import path from "node:path";

const cmapsDir = "/Users/qsyj/Code/rust/everything-manual/apps/web/node_modules/pdfjs-dist/cmaps";
const fontsDir = "/Users/qsyj/Code/rust/everything-manual/apps/web/node_modules/pdfjs-dist/standard_fonts";
const served = [];
const server = http.createServer((req, res) => {
  const url = new URL(req.url, "http://127.0.0.1");
  const base = url.pathname.startsWith("/fonts/") ? fontsDir : cmapsDir;
  const rel = decodeURIComponent(url.pathname.replace(/^\/(cmaps|fonts)\//, ""));
  const file = path.join(base, rel);
  served.push(url.pathname);
  if (!fs.existsSync(file)) { res.statusCode = 404; res.end(); return; }
  res.end(fs.readFileSync(file));
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const port = server.address().port;

const { getDocument } = await import("/Users/qsyj/Code/rust/everything-manual/apps/web/node_modules/pdfjs-dist/legacy/build/pdf.mjs");
const data = new Uint8Array(fs.readFileSync("/Users/qsyj/Code/rust/everything-manual/tests/fixtures/assets/sample-manual-nonlatin.pdf"));
const doc = await getDocument({
  data,
  cMapUrl: `http://127.0.0.1:${port}/cmaps/`,
  cMapPacked: true,
  standardFontDataUrl: `http://127.0.0.1:${port}/fonts/`,
}).promise;
const page = await doc.getPage(1);
const content = await page.getTextContent();
console.log("numPages:", doc.numPages);
console.log("text:", JSON.stringify(content.items.map((i) => i.str ?? "").join("|")));
console.log("fetched:", served);
await doc.loadingTask.destroy();
server.close();
