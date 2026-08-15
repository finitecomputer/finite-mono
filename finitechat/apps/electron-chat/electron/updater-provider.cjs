// Resolves electron-updater's autoUpdater lazily so that merely requiring
// this module never constructs platform updater state. In development (and in
// tests) the dependency resolves from node_modules; the packaged app has no
// node_modules, so scripts/package-macos-alpha.mjs writes a single-file
// esbuild bundle of electron-updater to electron/vendor/electron-updater.cjs
// next to this module.
let cached = null;

function getAutoUpdater() {
  if (cached === null) {
    try {
      ({ autoUpdater: cached } = require("electron-updater"));
    } catch {
      ({ autoUpdater: cached } = require("./vendor/electron-updater.cjs"));
    }
  }
  return cached;
}

module.exports = { getAutoUpdater };
