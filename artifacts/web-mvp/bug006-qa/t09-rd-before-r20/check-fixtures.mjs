import fs from "node:fs";
const { getDocument } = await import("/Users/qsyj/Code/rust/everything-manual/apps/web/node_modules/pdfjs-dist/legacy/build/pdf.mjs");

const dir = "/Users/qsyj/Code/rust/everything-manual/tests/fixtures/assets/";
const load = (name) => new Uint8Array(fs.readFileSync(dir + name));

async function open(name, options = {}) {
  try {
    const doc = await getDocument({ data: load(name), ...options }).promise;
    const pages = doc.numPages;
    const page = await doc.getPage(1);
    const vp = page.getViewport({ scale: 1 });
    let text = "";
    try {
      const content = await page.getTextContent();
      text = content.items.map((i) => i.str ?? "").join("|");
    } catch (error) {
      text = `(text error: ${error.name})`;
    }
    await doc.loadingTask.destroy();
    return { ok: true, pages, viewport: `${Math.round(vp.width)}x${Math.round(vp.height)} rot=${vp.rotation}`, text };
  } catch (error) {
    return { ok: false, name: error.name, message: String(error.message).slice(0, 80) };
  }
}

console.log("text      ", await open("sample-manual-text.pdf"));
console.log("rotated p2", await (async () => {
  const doc = await getDocument({ data: load("sample-manual-rotated.pdf") }).promise;
  const p2 = await doc.getPage(2);
  const vp = p2.getViewport({ scale: 1 });
  const content = await p2.getTextContent();
  const result = { viewport: `${Math.round(vp.width)}x${Math.round(vp.height)} rot=${vp.rotation}`, text: content.items.map((i) => i.str ?? "").join("|") };
  await doc.loadingTask.destroy();
  return result;
})());
console.log("many-pages", await open("sample-manual-many-pages.pdf"));
console.log("encrypted (no password) ", await open("sample-manual-encrypted.pdf"));
console.log("encrypted (with password)", await open("sample-manual-encrypted.pdf", { password: "fixture-secret" }));
console.log("nonlatin  ", await open("sample-manual-nonlatin.pdf"));
