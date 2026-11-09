// Downloads the official Engine.IO and Socket.IO protocol test suites at the commits pinned in suites.json.
//
//   node scripts/fetch-suites.mjs [--config <file>] [--out <dir>]
//
// Nothing is vendored: git fetches the pinned commit (git verifies object ids, so the content is the pinned one) into a
// temporary directory, the `dir` of the suite is copied to suites/<name>/ and the hard-coded server URL is replaced by
// `process.env.SUITE_URL`. No shell is involved: git runs with argument arrays.
import { execFileSync } from "node:child_process";
import { cpSync, mkdtempSync, readFileSync, rmSync, writeFileSync, mkdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const LITERAL = '"http://localhost:3000"';
const REPLACEMENT = "process.env.SUITE_URL";

function parseArgs(argv) {
  const options = { config: join(ROOT, "suites.json"), out: join(ROOT, "suites") };
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--config") options.config = resolve(argv[++i]);
    else if (argv[i] === "--out") options.out = resolve(argv[++i]);
    else throw new Error(`unknown argument ${argv[i]}`);
  }
  return options;
}

function git(cwd, ...args) {
  return execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim();
}

function fetchSuite(name, suite, outDir) {
  if (typeof suite.commit !== "string" || !/^[0-9a-f]{40}$/.test(suite.commit)) {
    throw new Error(`suite ${name}: "commit" must be 40 lowercase hex characters, got ${JSON.stringify(suite.commit)}`);
  }
  if (typeof suite.repo !== "string" || !suite.repo.startsWith("https://")) {
    throw new Error(`suite ${name}: "repo" must be an https URL`);
  }
  if (!Array.isArray(suite.files) || suite.files.length === 0) {
    throw new Error(`suite ${name}: "files" must list the files that hold ${LITERAL}`);
  }

  const work = mkdtempSync(join(tmpdir(), `socket-io-suite-${name}-`));
  try {
    try {
      git(work, "init", "--quiet");
      git(work, "fetch", "--quiet", "--depth", "1", suite.repo, suite.commit);
      git(work, "checkout", "--quiet", "--detach", "FETCH_HEAD");
    } catch (error) {
      throw new Error(`suite ${name}: cannot fetch commit ${suite.commit} from ${suite.repo}: ${String(error.stderr ?? error.message).trim()}`);
    }
    const head = git(work, "rev-parse", "HEAD");
    if (head !== suite.commit) {
      throw new Error(`suite ${name}: checked out ${head} instead of the pinned ${suite.commit}`);
    }

    const target = join(outDir, name);
    rmSync(target, { recursive: true, force: true });
    mkdirSync(dirname(target), { recursive: true });
    cpSync(join(work, suite.dir), target, { recursive: true });
    rmSync(join(target, "node_modules"), { recursive: true, force: true });

    for (const file of suite.files) {
      const path = join(target, file);
      let text;
      try {
        text = readFileSync(path, "utf8");
      } catch {
        rmSync(target, { recursive: true, force: true });
        throw new Error(`suite ${name}: listed file ${file} does not exist at commit ${suite.commit}`);
      }
      const count = text.split(LITERAL).length - 1;
      if (count !== 1) {
        rmSync(target, { recursive: true, force: true });
        throw new Error(`suite ${name}: file ${file} must contain ${LITERAL} exactly once, found ${count} (did upstream change?)`);
      }
      writeFileSync(path, text.replace(LITERAL, REPLACEMENT));
    }
    console.log(`${name}: ${suite.commit.slice(0, 12)} -> ${target}`);
  } finally {
    rmSync(work, { recursive: true, force: true });
  }
}

try {
  const options = parseArgs(process.argv.slice(2));
  const suites = JSON.parse(readFileSync(options.config, "utf8"));
  for (const [name, suite] of Object.entries(suites)) fetchSuite(name, suite, options.out);
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
