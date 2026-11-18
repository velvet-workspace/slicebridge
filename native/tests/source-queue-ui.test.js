'use strict';

const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const root=path.resolve(__dirname,'..');
const read=p=>fs.readFileSync(path.join(root,p),'utf8');

test('source-file list receives a proper independent panel after selecting files',()=>{
  const html=read('frontend/dist/index.html');
  const js=read('frontend/dist/app.js');
  const css=read('frontend/dist/app.css');
  for(const id of ['fileList','queueCount','queuedFilesLabel','queueFilesShownCount',
                   'dropzone','queueFootnote']){
    assert.equal((html.match(new RegExp('id="'+id+'"','g'))||[]).length,1);
  }
  assert.match(html,/id="queuedFilesLabel" hidden/);
  assert.match(html,/id="fileList" role="list"/);
  assert.match(js,/dom\.fileList\.parentElement\.classList\.toggle\('has-files',hasFiles\)/);
  assert.match(js,/el\('queuedFilesLabel'\)\.hidden=!hasFiles/);
  assert.match(js,/el\('queueFilesShownCount'\)\.textContent=hasFiles\?String\(files\.length\):''/);
  assert.match(js,/dom\.fileList\.setAttribute\('aria-label',t\('queuedFiles'\)\)/);
  assert.match(js,/queuedFiles:'קבצים שנבחרו'/);
  assert.match(js,/queuedFiles:'Selected source files'/);

  const current=css.slice(css.indexOf('/* dev.5 —'));
  assert.ok(current.startsWith('/* dev.5 —'),'responsive source-queue rules missing');
  assert.match(current,/\.left-column\{\s*display:flex;\s*flex-direction:column;/);
  assert.match(current,/\.queue-panel \.queue-list\{[\s\S]*?overflow-y:auto;/);
  assert.match(current,/\.queue-files-heading\[hidden\]\{display:none!important\}/);
  assert.match(current,/@media\(max-width:960px\)/);
  assert.match(current,/@media\(max-height:700px\)/);
  const compact=css.slice(css.indexOf('/* dev.6 —'));
  assert.ok(compact.startsWith('/* dev.6 —'),'compact source-queue layout rules missing');
  assert.match(compact,/\.queue-panel\.has-files\{\s*height:auto;/);
  assert.match(compact,/\.queue-panel\.has-files \.queue-list\{[\s\S]*?max-height:min\(196px,30dvh\)/);
  assert.match(compact,/\.queue-panel\.has-files \.queue-files-heading,[\s\S]*?\.panel-bottom\{\s*display:none!important/);
  assert.match(compact,/@media \(min-width:741px\) and \(max-height:825px\)/);

});

test('empty queue remains compact and does not leave a visible scrollbar',()=>{
  const css=read('frontend/dist/app.css');
  const js=read('frontend/dist/app.js');
  const now=css.slice(css.indexOf('/* dev.5 —'));
  assert.match(now,/\.queue-panel\{\s*flex:none;\s*height:224px;/);
  const compact=css.slice(css.indexOf('/* dev.6 —'));
  assert.match(compact,/\.queue-panel\{\s*height:206px;/);
  assert.match(now,/\.queue-panel \.queue-list\[hidden\]\{display:none!important\}/);
  assert.match(js,/dom\.fileList\.hidden=!hasFiles/);
});

test('source file names display useful details and retain removal affordance',()=>{
  const css=read('frontend/dist/app.css');
  const app=read('frontend/dist/app.js');
  const now=css.slice(css.indexOf('/* dev.5 —'));
  assert.match(now,/\.queue-file-details strong\{[\s\S]*?-webkit-line-clamp:2/);
  assert.match(app,/const name=node\('strong','',file\.name\);name\.title=file\.path/);
  assert.match(app,/const location=node\('small','',file\.folder\+' · '\+file\.sizeLabel\)/);
  assert.match(app,/remove\.title=language==='he'\?'הסר מהתור'/);
});


test('conversion results do not render advisory warnings about virtual materials or physical toolheads',()=>{
  const app=read('frontend/dist/app.js');
  assert.doesNotMatch(app,/virtualSlotsReview|toolUseReview|summaryReview|requiresToolUseReview/);
  assert.doesNotMatch(app,/material-mapping-warning|tool-usage-warning/);
  assert.match(app,/result\.ok\?'good':'bad'/);
  assert.match(app,/result\.ok\?t\('summarySuccess'\):t\('summaryFailure'\)/);
  // Keep the result card quiet, including the old three tower banners;
  // the backend retains the exact diagnostic data for engineering QA.
  assert.doesNotMatch(app,/towerClearanceReview|towerPerPlateReview|towerPlateUnknownReview/);
  assert.doesNotMatch(app,/tower-clearance-warning|tower-per-plate-warning|tower-plate-unknown-warning/);
  assert.doesNotMatch(app,/identical_source_slots_coalesced/);
});


test('failed conversions keep their visible error while successful conversions stay concise',()=>{
  const app=read('frontend/dist/app.js');
  assert.match(app,/result\.ok\?t\('summarySuccess'\):t\('summaryFailure'\)/);
  assert.match(app,/parent\.append\(row\)/);
  assert.match(app,/node\('div','error-line'/);
  assert.doesNotMatch(app,/towerClearanceReview|towerPerPlateReview|towerPlateUnknownReview/);
});

test('keyboard-activatable dropzone has a real accessible button role and localized name',()=>{
  const html=read('frontend/dist/index.html');
  const js=read('frontend/dist/app.js');
  const tag=html.match(/<div id="dropzone"[^>]*>/)?.[0];
  assert.ok(tag,'dropzone is missing');
  assert.match(tag,/tabindex="0"/,'keyboard tab stop must remain');
  assert.match(tag,/role="button"/,'focusable action must expose button semantics');
  assert.match(tag,/aria-label="הוסף קבצים"/,'Hebrew fallback label must work before JS initializes');
  assert.match(tag,/data-i18n-title="addFiles"/,'label must update with language changes');
  assert.match(js,/addFiles:'הוסף קבצים'/);
  assert.match(js,/addFiles:'Add files'/);
  assert.match(js,/elem\.setAttribute\('aria-label',t\(elem\.dataset\.i18nTitle\)\)/);
  assert.match(js,/drop\.addEventListener\('keydown'/);
  assert.match(js,/if\(e\.key==='Enter'\|\|e\.key===' '\)\{e\.preventDefault\(\);pickFiles\(\);\}/);
});

test('dialog close controls keep localized accessible names in Hebrew and English',()=>{
  const html=read('frontend/dist/index.html');
  const js=read('frontend/dist/app.js');
  const ids=['closeMyPrintersBtn','closePrinterProfileBtn','closeSettingsBtn',
    'closeIntegrationsBtn','closeAboutBtn'];
  for(const id of ids){
    const tag=html.match(new RegExp('<button[^>]*id="'+id+'"[^>]*>'))?.[0];
    assert.ok(tag,'missing close control '+id);
    assert.match(tag,/aria-label="סגירה"/,'Hebrew fallback must not be English-only: '+id);
    assert.match(tag,/data-i18n-title="close"/,'label must follow language: '+id);
  }
  assert.match(js,/close:'סגירה'/);
  assert.match(js,/close:'Close'/);
});

test('accessibility region and form names follow Hebrew and English language switches',()=>{
  const html=read('frontend/dist/index.html');
  const js=read('frontend/dist/app.js');
  const names=new Map([
    ['resultList','resultsTitle'],
    ['progressTrack','conversionProgressLabel'],
    ['printerManufacturerFilter','manufacturerLabel'],
    ['catalogModelList','catalogModelsLabel'],
    ['settingsTabs','printerSelectionLabel'],
    ['browserTargetSelect','browserLabel'],
  ]);
  for(const [id,key] of names){
    const line=html.split('\n').find(line=>line.includes('id="'+id+'"'));
    assert.ok(line,'missing accessibility element: '+id);
    assert.match(line,new RegExp('data-i18n-aria-label="'+key+'"'));
    assert.match(line,/aria-label="[^"]+"/);
  }
  assert.match(html,/class="language-switch" role="group" aria-label="שפה" data-i18n-aria-label="languageLabel"/);
  assert.match(html,/class="conversion-progress" role="group" aria-label="התקדמות ההמרה" data-i18n-aria-label="conversionProgressLabel"/);
  assert.match(js,/document\.querySelectorAll\('\[data-i18n-aria-label\]'\)/);
  assert.match(js,/elem\.setAttribute\('aria-label',t\(elem\.dataset\.i18nAriaLabel\)\)/);
  for(const marker of [
    "languageLabel:'שפה'","languageLabel:'Language'",
    "catalogModelsLabel:'דגמי מדפסות'","catalogModelsLabel:'Catalogue models'",
    "manufacturerLabel:'בחירת יצרן'","manufacturerLabel:'Select manufacturer'",
    "printerSelectionLabel:'בחירת מדפסת'","printerSelectionLabel:'Printer selection'",
  ])assert.ok(js.includes(marker),'missing localized accessible name: '+marker);
});
