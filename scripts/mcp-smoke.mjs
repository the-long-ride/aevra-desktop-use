// Starts `aevra-desktop mcp` and speaks MCP over stdio: initialize, tools/list,
// then desktop_status. Usage: node scripts/mcp-smoke.mjs <path-to-binary>
// A throw-away config directory keeps it away from the user's real allowlist.
import { spawn } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const binary = process.argv[2];
if (!binary) {
  console.error("usage: node scripts/mcp-smoke.mjs <path-to-aevra-desktop>");
  process.exit(2);
}

const home = mkdtempSync(join(tmpdir(), "aevra-desktop-smoke-"));
const env = {
  ...process.env,
  APPDATA: home,
  XDG_CONFIG_HOME: home,
  HOME: home,
};
const child = spawn(binary, ["mcp"], { env, stdio: ["pipe", "pipe", "inherit"] });

const pending = new Map();
let buffer = "";
child.stdout.on("data", (chunk) => {
  buffer += chunk;
  for (let nl = buffer.indexOf("\n"); nl >= 0; nl = buffer.indexOf("\n")) {
    const line = buffer.slice(0, nl).trim();
    buffer = buffer.slice(nl + 1);
    if (!line) continue;
    const message = JSON.parse(line);
    pending.get(message.id)?.(message);
  }
});

let nextId = 1;
function request(method, params) {
  const id = nextId++;
  const reply = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error(`${method} timed out`)), 30_000);
    pending.set(id, (message) => {
      clearTimeout(timer);
      resolve(message);
    });
  });
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  return reply;
}

function fail(message) {
  console.error(`smoke test failed: ${message}`);
  child.kill();
  rmSync(home, { recursive: true, force: true });
  process.exit(1);
}

try {
  const init = await request("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "smoke", version: "0" },
  });
  if (init.error) fail(`initialize: ${JSON.stringify(init.error)}`);
  child.stdin.write(
    `${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`,
  );

  const list = await request("tools/list", {});
  const names = (list.result?.tools ?? []).map((tool) => tool.name);
  for (const required of ["desktop_status", "desktop_windows", "desktop_act_many"]) {
    if (!names.includes(required)) fail(`tools/list is missing ${required}`);
  }
  if (names.length !== 17) fail(`expected 17 tools, got ${names.length}`);
  if (names.includes("desktop_request_access")) fail("desktop_request_access must not be exposed");

  const status = await request("tools/call", { name: "desktop_status", arguments: {} });
  if (status.error || status.result?.isError) fail(`desktop_status: ${JSON.stringify(status)}`);
  const body = JSON.parse(status.result.content[0].text);
  if (body.allowedApps !== 0) fail(`a fresh config must allow nothing: ${JSON.stringify(body)}`);

  console.log(`smoke test ok: ${names.length} tools, desktop_status answered`);
} catch (error) {
  fail(String(error));
}
child.kill();
rmSync(home, { recursive: true, force: true });
