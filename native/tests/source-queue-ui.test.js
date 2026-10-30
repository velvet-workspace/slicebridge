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


test('extra virtual filament slots are disclosed without any silent merge',()=>{
  const app=read('frontend/dist/app.js');
  assert.match(app,/virtualSlotsReview:'כל \{n\} ערוצי הצבע/);
  assert.match(app,/virtualSlotsReview:'All \{n\} source color\/material/);
  assert.match(app,/נתון זה אינו קובע את קיבולת מערכת הזנת הפילמנט/);
  assert.match(app,/does not establish feeder capacity/);
  assert.match(app,/towerPerPlateReview:'ב־3MF המומר יש/);
  assert.match(app,/towerPerPlateReview:'The converted 3MF has/);
  assert.match(app,/const mappingReview=Boolean\(result\.ok&&requiresToolUseReview\(result\.report\)\)/);
  assert.match(app,/row\.append\(node\('p','warning-line material-mapping-warning'/);
  assert.match(app,/t\('virtualSlotsReview',\{n:report\.virtual_filament_slots,physical:report\.physical_toolheads\}\)/);
  assert.doesNotMatch(app,/identical_source_slots_coalesced/);
});

test('multicolor tool usage remains unverified even if four slots fit four tools',()=>{
  const app=read('frontend/dist/app.js');
  const match=app.match(/function requiresToolUseReview\(report\)\{[\s\S]*?\n\}/);
  assert.ok(match,'actual UI tool-use safety decision helper is missing');
  const check=Function(match[0]+';return requiresToolUseReview;')();
  assert.equal(check({user_slot_mapping_required:false,tool_usage_verification_required:true}),true);
  assert.equal(check({user_slot_mapping_required:true,tool_usage_verification_required:false}),true);
  assert.equal(check({user_slot_mapping_required:false,tool_usage_verification_required:false}),false);
  assert.equal(check(null),false);
  assert.match(app,/toolUseReview:'[^']*G-code/);
  assert.match(app,/warning-line tool-usage-warning/);
});


test('unknown target plate metadata has bilingual manual tower review wording',()=>{
  const app=read('frontend/dist/app.js');
  assert.match(app,/towerPlateUnknownReview:'לא ניתן לקבוע/);
  assert.match(app,/towerPlateUnknownReview:'The converted 3MF/);
  assert.match(app,/tower-plate-unknown-warning/);
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
