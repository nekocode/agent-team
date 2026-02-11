// ============================================================
// Postinstall: Verify Platform Package
// ============================================================

const PLATFORMS = {
  "darwin-arm64": "@nekocode/agent-team-darwin-arm64",
  "darwin-x64": "@nekocode/agent-team-darwin-x64",
  "linux-x64": "@nekocode/agent-team-linux-x64",
  "win32-x64": "@nekocode/agent-team-win32-x64",
};

const key = `${process.platform}-${process.arch}`;
const pkg = PLATFORMS[key];

if (!pkg) {
  console.warn(`[agent-team] Warning: Unsupported platform ${key}`);
  console.warn(`[agent-team] Supported: ${Object.keys(PLATFORMS).join(", ")}`);
  process.exit(0);
}

try {
  require.resolve(`${pkg}/package.json`);
} catch {
  console.warn(`[agent-team] Warning: Platform package ${pkg} not installed`);
  console.warn(`[agent-team] This may happen if npm failed to install optional dependencies`);
  process.exit(0);
}
