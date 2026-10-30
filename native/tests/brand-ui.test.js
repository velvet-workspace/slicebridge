'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const base=path.resolve(__dirname,'..');
const read=(f)=>fs.readFileSync(path.join(base,f),'utf8');

test('approved SliceBridge name and Mint/Forest identity are embedded',()=>{
 const page=read('frontend/dist/index.html');
 const manifest=JSON.parse(read('wails.json').replace(/^\uFEFF/,''));
 assert.equal(manifest.name,'SliceBridge');
 assert.equal(manifest.outputfilename,'SliceBridge');
 assert.equal(manifest.info.productName,'SliceBridge');
 const css=read('frontend/dist/app.css');
 const icon=fs.readFileSync(path.join(base,'frontend/dist/brand-icon.png'));
 assert.match(page,/<title>SliceBridge<\/title>/);
 assert.match(page,/class="brand-mark" src="\.\/brand-icon\.png"/);
 assert.match(page,/<strong>Slice<span class="brand-accent">Bridge<\/span><\/strong>/);
 for(const hex of ['DFF6EE','A9E5D0','5BBFA8','2F7F73','193F38']){
  assert.ok(css.toUpperCase().includes('#'+hex),'missing approved shade '+hex);
 }
 assert.equal(icon.subarray(1,4).toString(),'PNG');
 assert.ok(icon.length>10000,'approved image-derived icon must not be the old SVG');
 assert.doesNotMatch(page,/class="brand-mark"[^>]*>3<\/div>/);
});

test('version and check button live in About, not the busy footer',()=>{
 const page=read('frontend/dist/index.html');
 const footerStart=page.indexOf('<footer class="actionbar">');
 const footer=page.slice(footerStart,page.indexOf('</footer>',footerStart));
 const about=page.slice(page.indexOf('<dialog id="aboutDialog"'));
 assert.match(about,/id="installedVersion"/);
 assert.match(about,/id="checkUpdatesBtn"/);
 assert.match(about,/id="updateNotice"/);
 assert.doesNotMatch(footer,/id="installedVersion"|id="checkUpdatesBtn"/);
 assert.match(page,/id="aboutBtn"/);
 assert.equal((page.match(/id="installedVersion"/g)||[]).length,1);
 assert.equal((page.match(/id="checkUpdatesBtn"/g)||[]).length,1);
 assert.equal((page.match(/id="settingsBtn"/g)||[]).length,1);
});

test('results clear is manual and automatic, while JSON export is removed',()=>{
 const page=read('frontend/dist/index.html');
 const app=read('frontend/dist/app.js');
 assert.match(page,/id="clearResultsBtn"/);
 assert.doesNotMatch(page,/id="exportReportBtn"|id="reportToolbar"/);
 assert.doesNotMatch(app,/ExportLastReport|exportReportBtn|reportToolbar/);
 assert.match(app,/function clearResults\(/);
 assert.match(app,/if\(next.length===files.length\)return;/);
 assert.match(app,/files=next;\s*memoryStatus=null;\s*clearResults\(\);/);
 assert.match(app,/el\('clearResultsBtn'\)\.addEventListener/);
 for(const lang of ['clearResults:\'נקה תוצאות\'','clearResults:\'Clear results\'']){
  assert.ok(app.includes(lang),'missing translation '+lang);
 }
});

test('MakerWorld connection labels are localized, not translation keys',()=>{
 const page=read('frontend/dist/index.html');
 const app=read('frontend/dist/app.js');
 const integrations=read('frontend/dist/integrations.js');
 assert.match(page,/data-i18n="makerworldSetup">MakerWorld Setup<\/span>/);
 assert.match(app,/makerworldSetup:'חיבור ל־MakerWorld'/);
 assert.match(app,/makerworldSetup:'MakerWorld Setup'/);
 assert.match(integrations,/integrationTitle:'חיבור ל־MakerWorld'/);
 assert.match(integrations,/integrationTitle:'MakerWorld Setup'/);
 assert.doesNotMatch(page,/browserIntegration|Browser integration/);
 assert.doesNotMatch(integrations,/browserIntegration|Browser integration/);
});

// Existing native CI invokes this suite explicitly; include the opt-in
// browser installation flow regressions without changing protected workflows.
require('./extension-install-ui.test.js');

// The native CI invokes this suite explicitly; keep the source queue geometry
// regressions in that fixed list without changing a protected GitHub workflow.
require('./source-queue-ui.test.js');

// Keep the dynamically scrollable report operable from the keyboard.
test('conversion results scroll region can receive keyboard focus with a visible indicator',()=>{
 const page=read('frontend/dist/index.html');
 const css=read('frontend/dist/app.css');
 assert.match(page,/id="resultList" role="region" tabindex="0" aria-label="תוצאות המרה" data-i18n-aria-label="resultsTitle"/);
 assert.match(css,/\.results-scroll:focus-visible\{outline:2px solid var\(--brand\);outline-offset:-2px\}/);
});
