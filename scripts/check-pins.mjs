#!/usr/bin/env node
// Native platform dependencies are owned once by the canonical helper library.
import { readFileSync, existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
const root = join(dirname(fileURLToPath(import.meta.url)), "..");
export function validateLibraryBoundary(coreText) {
  const problems = [];
  if (
    !/^aevra-desktop-helper\s*=\s*\{\s*path\s*=\s*"\.\.\/\.\.\/vendor\/aevra\/helper"\s*\}/m.test(
      coreText,
    )
  )
    problems.push(
      "desktop-core must depend directly on the canonical helper library",
    );
  if (/^\[target\./m.test(coreText))
    problems.push(
      "native platform dependencies belong in the canonical helper crate",
    );
  return problems;
}
if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const problems = validateLibraryBoundary(
    readFileSync(join(root, "crates/desktop-core/Cargo.toml"), "utf8"),
  );
  if (!existsSync(join(root, "vendor/aevra/helper/src/lib.rs")))
    problems.push("prepare the canonical helper library overlay first");
  if (problems.length) {
    console.error(problems.join("\n"));
    process.exit(1);
  }
  console.log("canonical library boundary OK");
}
