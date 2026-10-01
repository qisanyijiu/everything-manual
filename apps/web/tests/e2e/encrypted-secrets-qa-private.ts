/** ES-01 QA helpers. Operate only on explicitly supplied, isolated fixture data.
 * Never attach command output, scanned bytes, canary values or secret-bearing objects
 * to a Playwright assertion, reporter, trace or artifact.
 */
import { spawnSync } from "node:child_process";
import { randomBytes } from "node:crypto";
import fs from "node:fs";
import path from "node:path";

export function newQaSecrets() {
  return {
    master: randomBytes(32).toString("hex"),
    tripo: `qa-${randomBytes(24).toString("hex")}`,
    manualAi: `qa-${randomBytes(24).toString("hex")}`,
  };
}

/** Deliberately exclude the host's EM_* and provider secrets. */
export function privateQaEnvironment(values: Record<string, string> = {}): NodeJS.ProcessEnv {
  return { PATH: process.env.PATH, TMPDIR: process.env.TMPDIR, ...values };
}

/** Unlike execFileSync, an unsuccessful command does not throw captured stderr. */
export function runPrivateCommand(
  executable: string,
  args: string[],
  cwd: string,
  values: Record<string, string> = {},
  input?: Buffer,
) {
  const result = spawnSync(executable, args, {
    cwd,
    env: privateQaEnvironment(values),
    input,
    stdio: ["pipe", "pipe", "pipe"],
    timeout: 30_000,
    maxBuffer: 16 * 1024 * 1024,
  });
  return {
    status: result.status,
    signal: result.signal,
    executionFailed: result.error !== undefined,
    // Caller may inspect only in memory; reports use status/boolean/count fields.
    stdout: result.stdout ?? Buffer.alloc(0),
    stderr: result.stderr ?? Buffer.alloc(0),
  };
}

function decodeJsonEscapes(text: string): string {
  return text.replace(/\\u([0-9a-f]{4})|\\(["\\/bfnrt])/giu, (match, unicode: string | undefined) => {
    if (unicode !== undefined) return String.fromCharCode(Number.parseInt(unicode, 16));
    try { return JSON.parse(`"${match}"`) as string; } catch { return match; }
  });
}

/** Detect direct and JSON-escaped reflections without printing matching context.
 * Three passes also cover JSON diagnostic strings embedded in another JSON value.
 * This is intentionally not a claim to detect arbitrary secret encodings.
 */
export function secretPresence(bytes: Buffer | string, canaries: readonly string[]) {
  let text = typeof bytes === "string" ? bytes : bytes.toString("utf8");
  const contains = (value: string) => canaries.some((key) => key.length > 0 && value.includes(key));
  const direct = contains(text);
  let jsonEscaped = false;
  for (let layer = 0; layer < 3; layer += 1) {
    const decoded = decodeJsonEscapes(text);
    if (decoded === text) break;
    jsonEscaped ||= contains(decoded);
    text = decoded;
  }
  return { direct, jsonEscaped, leaked: direct || jsonEscaped };
}

/** Only call with QA-owned temp/artifact roots. Symlinks are counted, never followed.
 * Scan intentional legacy/plaintext input fixtures separately with explicit expected
 * presence; they must not be silently excluded from an application-output scan.
 */
export function scanPrivateQaTree(root: string, canaries: readonly string[]) {
  const result = { filesScanned: 0, directMatches: 0, jsonEscapedMatches: 0, symlinksSkipped: 0 };
  function visit(current: string) {
    const metadata = fs.lstatSync(current);
    if (metadata.isSymbolicLink()) { result.symlinksSkipped += 1; return; }
    if (metadata.isDirectory()) {
      for (const entry of fs.readdirSync(current)) visit(path.join(current, entry));
    } else if (metadata.isFile()) {
      const found = secretPresence(fs.readFileSync(current), canaries);
      result.filesScanned += 1;
      result.directMatches += Number(found.direct);
      result.jsonEscapedMatches += Number(found.jsonEscaped);
    }
  }
  visit(root);
  return result;
}
