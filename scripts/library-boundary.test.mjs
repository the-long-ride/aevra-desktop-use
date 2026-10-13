import assert from "node:assert/strict";
import test from "node:test";
import { validateLibraryBoundary } from "./check-pins.mjs";
test("one canonical library owns platform dependencies; consumers cannot mirror the native graph", () => {
  const core =
    '[dependencies]\naevra-desktop-helper = { path = "../../vendor/aevra/helper" }\nserde="1"';
  assert.deepEqual(validateLibraryBoundary(core), []);
  assert.ok(
    validateLibraryBoundary(
      core.replace("../../vendor/aevra/helper", "../../some-copy"),
    ).length,
  );
  assert.ok(
    validateLibraryBoundary(
      core + "\n[target.'cfg(windows)'.dependencies]\nwindows=\"0.58\"",
    ).length,
  );
});
