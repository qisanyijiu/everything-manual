/** Teardown for affected suites, which must remain bound to C's exact copied inputs. */
import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
export default function verifyCFreeze() {
  const root = process.env.EM_PC05C_QA_WEB_ROOT, expected = process.env.EM_PC05C_QA_WEB_MANIFEST_SHA256;
  const binary = process.env.EM_PC05C_QA_BINARY, binarySha = process.env.EM_PC05C_QA_BINARY_SHA256;
  if (!root || !expected || !binary || !binarySha) throw new Error("All four C freeze inputs required");
  const sha = (file: string) => createHash("sha256").update(fs.readFileSync(file)).digest("hex");
  const manifestFile = path.join(root, "source-hashes.json");
  if (sha(manifestFile) !== expected || sha(binary) !== binarySha) throw new Error("C copied input fingerprint changed");
  const manifest = JSON.parse(fs.readFileSync(manifestFile, "utf8")) as Record<string, string>;
  for (const [file, hash] of Object.entries(manifest)) if (sha(path.join(root, file)) !== hash) throw new Error("C frozen frontend changed: " + file);
}
