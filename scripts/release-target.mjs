// Resolve only an existing versioned tag; every downstream job uses this SHA.
import { appendFileSync, existsSync, readFileSync } from "node:fs";
import { execFileSync } from "node:child_process";

try {
  const tag = process.env.RELEASE_TAG || process.argv[2] || "";
  if (!/^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(tag))
    throw new Error("Release requires an existing vMAJOR.MINOR.PATCH tag.");
  const nodePackage = existsSync("package.json");
  const version = nodePackage
    ? JSON.parse(readFileSync("package.json", "utf8")).version
    : readFileSync("Cargo.toml", "utf8").match(
        /^\[workspace\.package\][\s\S]*?^version\s*=\s*"([^"]+)"/m,
      )?.[1];
  const requested = nodePackage ? tag.slice(1) : tag.slice(1).split(/[+-]/)[0];
  if (!version || version !== requested)
    throw new Error(`Tag ${tag} does not match source version ${version}.`);
  const git = (...args) =>
    execFileSync("git", args, {
      encoding: "utf8",
      stdio: ["pipe", "pipe", "pipe"],
    }).trim();
  const sha = git("rev-parse", "--verify", `refs/tags/${tag}^{commit}`);
  if (git("rev-parse", "HEAD") !== sha)
    throw new Error("Checkout does not match the release tag commit.");
  if (process.env.GITHUB_OUTPUT)
    appendFileSync(
      process.env.GITHUB_OUTPUT,
      `release-tag=${tag}\nrelease-sha=${sha}\n`,
    );
  console.log(JSON.stringify({ tag, sha, version }));
} catch (error) {
  console.error(error.message);
  process.exitCode = 1;
}
