const assert = require('node:assert/strict');
require('browser-env')();
try {
  require('../packages/web/src/generated/ocs-settings.js');
  assert(globalThis.OCS && globalThis.EUS, 'the bundled CommonJS branch must expose both APIs');
  assert.equal(globalThis.OCS.$elements, globalThis.EUS.$elements, 'shared custom element registry');
  assert(globalThis.OCS.definedProjects().length > 0, 'settings projects are available');
  console.log('PASS: bundled settings expose OCS and EUS with one element registry');
} finally { window.close(); }
