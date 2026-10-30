'use strict';

// A native desktop surface: no automatic window resizing, no page navigation,
// local-only file handling, and a persisted per-printer hardware profile.
const translations = {
  he: {
    brandTagline:'מחברים בין סלייסרים',settings:'הגדרות',makerworldSetup:'חיבור ל־MakerWorld',checkUpdates:'בדיקת עדכונים',
    dismiss:'סגירה',workspaceTitle:'סביבת המרה',
    workspaceSubtitle:'המרת פרויקטים לכמה מדפסות, בלי לשנות את המקור.',
    languageLabel:'שפה',catalogModelsLabel:'דגמי מדפסות',manufacturerLabel:'בחירת יצרן',
    printerSelectionLabel:'בחירת מדפסת',browserLabel:'דפדפן',conversionProgressLabel:'התקדמות ההמרה',
    localOnly:'עיבוד מקומי בלבד',queueTitle:'קובצי מקור',clear:'ניקוי',
    addFiles:'הוסף קבצים',addFolder:'הוסף תיקייה',queuedFiles:'קבצים שנבחרו',
    dropHint:'גרור קובצי 3MF או תיקייה לכאן',queueFootnote:'עד 200 קובצי 3MF, כולל תיקיות משנה.',
    targetsTitle:'מדפסות יעד',selectAll:'בחר הכול',deselectAll:'בטל בחירה',
    resultsTitle:'תוצאות המרה',emptyTitle:'מוכן להמרה',
    emptyDescription:'הוסף פרויקטים, בחר מדפסות והפעל המרה. דו״ח מפורט יוצג כאן לכל מדפסת.',
    clearResults:'נקה תוצאות',validateHint:'חובה לבצע Slice מחדש לפני הדפסה',
    idle:'ממתין לקבצים',idleHint:'בחר קבצים ומדפסות כדי להתחיל',
    ready:'מוכן להמרה',readyHint:'{files} קבצים × {printers} מדפסות',
    converting:'ממיר פרויקטים…',convertingFile:'קובץ {index} מתוך {total}: {name}',
    finished:'ההמרה הושלמה',partialFinished:'ההמרה הושלמה חלקית',finishedHint:'{success} הצלחות, {failed} כישלונות',
    failed:'ההמרה נכשלה',failedHint:'לא נוצרו קובצי פלט',
    openFolder:'פתח תיקיית פלט',convert:'המרה',
    settingsTitle:'הגדרות מדפסות',
    settingsHint:'העדפות נשמרות במחשב ומיושמות על המרות חדשות.',
    physicalNozzle:'דיזה המותקנת במדפסת',
    nozzleHint:'דיזה קבועה משתמשת בפרופיל הדיזה הפיזית, גם כשהמקור הוכן עם דיזה אחרת.',
    preferredSlicer:'סלייסר מועדף',slicerHint:'אוטומטי מתעדף סלייסר יצרן מותקן עם פרופיל תקין. אם אין, נבדקת חלופה תואמת. בחירה ידנית נשמרת.',
    filamentPreferences:'פרופילי פילמנט מועדפים',
    filamentHint:'אפשר לשמור פרופיל מותקן שונה לכל סוג חומר. Auto משתמש בברירת המחדל של היצרן.',
    material:'סוג חומר',preferredFilament:'פרופיל מותקן',cancel:'ביטול',
    saveSettings:'שמור הגדרות',saved:'ההגדרות נשמרו בהצלחה.',
    autoNozzle:'אוטומטי — התאמה לקובץ המקור',
    autoSlicer:'אוטומטי — סלייסר היצרן תחילה',
    autoFilament:'אוטומטי — פרופיל יצרן',
    noNativeProfiles:'לא נמצאו פרופילי פילמנט תואמים.',
    autoPresetHint:'לבחירת פרופיל פילמנט מסוים, יש לבחור קודם דיזה פיזית קבועה.',
    autoPresetWarning:'נשמר פרופיל פילמנט לדיזה אוטומטית. בחר דיזה קבועה וערוך את הפרופיל לפני השמירה.',
    recoverSettingsQuestion:'קובץ ההגדרות אינו תקין. לשמור עותק גיבוי של הקובץ המקורי ולחזור להגדרות ברירת מחדל?',
    recoveredSettings:'ההגדרות שוחזרו לברירת המחדל.',
    backupSaved:'הגיבוי המקורי נשמר כאן: {path}',
    unsupportedPrinter:'סלייסר לא זמין',openSlicer:'פתח בסלייסר',
    details:'פרטי המרה',source:'מקור',target:'יעד',nozzle:'דיזה',
    materialSlots:'סלילי חומר',plates:'משטחים',objects:'אובייקטים',
    instances:'מופעים',triangles:'משולשים',geometry:'גיאומטריה',
    geometryPreserved:'נשמרה במלואה',nativeProfile:'פרופיל סלייסר',
    materials:'חומרים וצבעים',parameters:'פרמטרים שנשמרו',
    overridden:'פרופילי חומר מותאמים',none:'ללא',unknown:'לא צוין',
    cacheRemoved:'קובצי Slice ישנים שהוסרו',
    sourceAssumed:'קוטר דיזת המקור לא זוהה — הונחה 0.4 מ״מ.',
    switchedNozzle:'בוצעה התאמת קוטר דיזה לפי הגדרות המדפסת.',
    virtualSlotsReview:'כל {n} ערוצי הצבע והחומר מהמקור נשמרו בנפרד. בפרופיל היעד רשומים {physical} ראשי הדפסה פיזיים; נתון זה אינו קובע את קיבולת מערכת הזנת הפילמנט. יש לבדוק את מערכת החלפת החומר ולשייך כל ערוץ בסלייסר היעד. אין לאחד או להסיר צבעים ללא בחירה מפורשת.',
    toolUseReview:'פרויקט רב־צבע: המרת 3MF אינה מוכיחה שכל החומרים מופעלים. בדוק בסלייסר היעד שיוך צבעים לכלים, תצוגת שכבות ופקודות החלפת חומר ב־G-code (אם נתמכות). אין להדפיס לפני אימות.',
    towerClearanceReview:'מיקום מגדל הניקוי לא אומת. יש לבדוק בסלייסר היעד שאין התנגשות בין המגדל או שוליו לבין המודל, ה־brim או אזורים מוחרגים של משטח ההדפסה. SliceBridge אינו מזיז את המגדל אוטומטית.',
    towerPlateUnknownReview:'לא ניתן לקבוע מתוך מטא־דאטה של קובץ ה־3MF המומר כמה משטחים קיימים, ולכן לא ניתן לבדוק אם לכל משטח הוגדר מיקום מגדל ניקוי. פתח את הפרויקט בסלייסר היעד ובדוק את כל המשטחים ומרווחי מגדל הניקוי לפני הדפסה.',
    towerPerPlateReview:'ב־3MF המומר יש {plates} משטחים, אך נמצאו רק {x} ערכי X ו־{y} ערכי Y למיקומי מגדל הניקוי. בדוק וקבע מיקום נפרד לכל משטח בסלייסר היעד לפני חיתוך או הדפסה. SliceBridge לא המציא מיקומים.',
    summarySuccess:'הומר בהצלחה',summaryReview:'מיפוי לבדיקה',summaryFailure:'נכשל',
    noSource:'הוסף לפחות קובץ אחד.',noTargets:'בחר מדפסת יעד אחת לפחות.',
    fileLimit:'מותר להוסיף לכל היותר 200 קבצים.',queueEmpty:'עדיין אין קבצים בתור.',
    afterSlice:'יש לבדוק חומרים, דיזה ותמיכות ולבצע Slice מחדש.',
    close:'סגירה',settingsError:'שגיאה בהגדרות',
    addError:'לא ניתן להוסיף קבצים',convertError:'המרה נכשלה',
    launchError:'לא ניתן לפתוח את הסלייסר',
    noUpdate:'לא נבדקו עדכונים',
    checkingUpdates:'בודק עדכונים ב־GitHub…',updateAvailable:'גרסה חדשה זמינה: {v}',
    latestInstalled:'הגרסה המותקנת מעודכנת.',
    noPublishedReleases:'עדיין לא פורסמו Releases. אפשר לראות בניות Beta.',
    platformUnavailable:'גרסה {v} קיימת, אך לא למחשב זה.',
    unsupportedPlatform:'אין עדכונים זמינים למערכת זו.',authRequired:'מאגר פרטי — נדרש GitHub CLI מחובר.',
    updateError:'בדיקת עדכונים נכשלה',openRelease:'פתיחת Release',
    openReleases:'צפייה בגרסאות',openPreviewBuilds:'צפייה בבניות Beta',
    prusaProjectName:'פרויקט PrusaSlicer',prusaProjectSummary:'כללי · בחירת מדפסת ב־PrusaSlicer',
    prusaGenericReview:'פרויקט PrusaSlicer כללי: יש לבחור את המדפסת המדויקת ב־PrusaSlicer ולבדוק את כל ערוצי החומר לפני חיתוך והדפסה.'
  },
  en: {
    brandTagline:'Bridge your prints',settings:'Settings',makerworldSetup:'MakerWorld Setup',checkUpdates:'Check for updates',
    dismiss:'Dismiss',workspaceTitle:'Conversion workspace',
    workspaceSubtitle:'Convert projects for multiple printers without altering your originals.',
    languageLabel:'Language',catalogModelsLabel:'Catalogue models',manufacturerLabel:'Select manufacturer',
    printerSelectionLabel:'Printer selection',browserLabel:'Browser',conversionProgressLabel:'Conversion progress',
    localOnly:'Completely local',queueTitle:'Source projects',clear:'Clear',
    addFiles:'Add files',addFolder:'Add folder',queuedFiles:'Selected source files',
    dropHint:'Drop 3MF files or folders here',queueFootnote:'Up to 200 projects, including subfolders.',
    targetsTitle:'Target printers',selectAll:'Select all',deselectAll:'Clear selection',
    resultsTitle:'Conversion results',emptyTitle:'Ready to convert',
    emptyDescription:'Add projects, select printers and convert. Each output gets its own detailed report.',
    clearResults:'Clear results',validateHint:'Always re-slice before printing',
    idle:'Waiting for projects',idleHint:'Add projects and select printers to start',
    ready:'Ready to convert',readyHint:'{files} projects × {printers} printers',
    converting:'Converting projects…',convertingFile:'Project {index} of {total}: {name}',
    finished:'Conversion completed',partialFinished:'Conversion partially completed',finishedHint:'{success} successful, {failed} failed',
    failed:'Conversion failed',failedHint:'No output projects created',
    openFolder:'Open output folder',convert:'Convert',
    settingsTitle:'Printer settings',
    settingsHint:'Preferences are stored on this computer and applied to future conversions.',
    physicalNozzle:'Physically installed nozzle',
    nozzleHint:'A fixed nozzle uses its native machine preset even if the source project has a different nozzle.',
    preferredSlicer:'Preferred slicer',slicerHint:'Automatic prefers a complete installed manufacturer profile, otherwise a compatible fallback. Manual overrides are preserved.',
    filamentPreferences:'Preferred filament presets',
    filamentHint:'Save a different installed preset per material type. Auto uses the manufacturer default.',
    material:'Material type',preferredFilament:'Installed filament preset',cancel:'Cancel',
    saveSettings:'Save settings',saved:'Preferences saved.',
    autoNozzle:'Automatic — match source project',
    autoSlicer:'Automatic — manufacturer slicer first',
    autoFilament:'Automatic — native manufacturer default',
    noNativeProfiles:'No compatible installed filament presets found.',
    autoPresetHint:'Choose a fixed physical nozzle before selecting a specific filament preset.',
    autoPresetWarning:'A filament preset is saved with Auto nozzle. Choose a fixed nozzle and review the preset before saving.',
    recoverSettingsQuestion:'Settings cannot be loaded. Back up the original settings file and restore defaults?',
    recoveredSettings:'Default settings restored.',
    backupSaved:'Original settings saved at: {path}',
    unsupportedPrinter:'Slicer unavailable',openSlicer:'Open in slicer',
    details:'Conversion details',source:'Source',target:'Target',nozzle:'Nozzle',
    materialSlots:'Material slots',plates:'Plates',objects:'Objects',
    instances:'Instances',triangles:'Triangles',geometry:'Geometry',
    geometryPreserved:'Fully preserved',nativeProfile:'Slicer profile',
    materials:'Materials and colors',parameters:'Preserved settings',
    overridden:'Custom material presets',none:'None',unknown:'Not specified',
    cacheRemoved:'Stale sliced files removed',
    sourceAssumed:'No source nozzle specified — 0.4 mm assumed.',
    switchedNozzle:'Nozzle diameter was adapted to match the saved target configuration.',
    virtualSlotsReview:'All {n} source color/material slots were preserved separately. The target profile reports {physical} physical toolhead(s); this does not establish feeder capacity. Verify the actual filament changer/feeder and explicitly assign every material in the destination slicer. Do not combine or discard colors unless intentionally requested.',
    toolUseReview:'Multi-color project: a valid 3MF does not prove that every material is used. Verify material-to-tool assignments, native slicer layer preview and G-code tool changes (where applicable). Do not print before verification.',
    towerClearanceReview:'Target prime/wipe tower clearance is unverified. Check for collision with the model, brim and excluded bed areas in the native slicer before printing. SliceBridge does not move tower coordinates automatically.',
    towerPlateUnknownReview:'The converted 3MF has an unknown plate count in its metadata, so the prime tower positions could not be audited for each plate. Inspect all plates and tower clearance in the native slicer before printing.',
    towerPerPlateReview:'The converted 3MF has {plates} plates but only {x} X and {y} Y prime tower coordinate entries. Set and verify per-plate tower placement for each plate in the native slicer before slicing or printing. SliceBridge has not invented positions.',
    summarySuccess:'Converted',summaryReview:'Review mapping',summaryFailure:'Failed',
    noSource:'Add at least one source project.',noTargets:'Select one or more printers.',
    fileLimit:'You can queue a maximum of 200 projects.',queueEmpty:'No files in the queue yet.',
    afterSlice:'Review materials, nozzle and supports; re-slice before printing.',
    close:'Close',settingsError:'Settings error',
    addError:'Could not add projects',convertError:'Conversion failed',
    launchError:'Could not launch the slicer',
    noUpdate:'Updates have not been checked',
    checkingUpdates:'Checking GitHub Releases…',updateAvailable:'New version available: {v}',
    latestInstalled:'You have the latest published version.',
    noPublishedReleases:'No GitHub Releases yet. You can view beta builds.',
    platformUnavailable:'Version {v} exists, but not for this platform.',
    unsupportedPlatform:'No update package exists for this platform.',authRequired:'Private repo — connect GitHub CLI.',
    updateError:'Update check failed',openRelease:'Open release',
    openReleases:'View releases',openPreviewBuilds:'View beta builds',
    prusaProjectName:'PrusaSlicer project',prusaProjectSummary:'Generic · choose printer in PrusaSlicer',
    prusaGenericReview:'Generic PrusaSlicer project: choose the exact printer in PrusaSlicer and review every material slot before slicing or printing.'
  }
};
const printerNames = Object.freeze({u1:'Snapmaker U1',c5:'Flashforge Creator 5',cc2:'ELEGOO CC2',qidi:'QIDI Plus5'});
// Fixed generic target. It is listed only when the sidecar file probe succeeds.
const prusaTargetId='prusa-project';
const nozzleChoices = Object.freeze({u1:['0.2','0.4','0.6','0.8'],c5:['0.25','0.4','0.6','0.8'],cc2:['0.2','0.4','0.6','0.8'],qidi:['0.2','0.4','0.6','0.8']});
const targetOrder = ['u1','c5','cc2','qidi'];
// Installed Orca-family machines, as listed by the backend. Only rows the
// backend resolved completely are present; the picker never invents one.
let installedTargets=[];
// Per-printer PrusaSlicer targets. Optional: an older binary without
// InstalledPrusaTargets leaves this empty, and no such row is ever shown.
let installedPrusaTargets=[];
let installedRows=[];
let prusaAvailable=false;
function targetDisplayName(id){
  if(id===prusaTargetId)return t('prusaProjectName');
  return printerNames[id]||installedTargets.find((item)=>item.id===id)?.title||
    installedPrusaTargets.find((item)=>item.id===id)?.title||id;
}
function prusaCheckbox(){return document.querySelector('[data-prusa-target] input');}
const el = (id) => document.getElementById(id);
const dom = {
  fileList:el('fileList'),queueCount:el('queueCount'),resultList:el('resultList'),
  reportCounts:el('reportCounts'),convertBtn:el('convertBtn'),convertCount:el('convertCount'),
  activityText:el('activityText'),activityDetail:el('activityDetail'),
  activityIcon:el('activityIcon'),folderBtn:el('openFolderBtn'),settings:el('settingsDialog')
};
let language = /^he\b/i.test(navigator.language||'')?'he':'en';
try { const saved=localStorage.getItem('3mfBridgeLanguage');if(saved==='he'||saved==='en')language=saved; } catch{}
let files=[];
let available=[];
let settings=null;
let busy=false;
let lastBatch=null;
let progress=null;
let currentTarget='u1';
let currentMaterial='PLA';
let draftPrefs=null;
let formChoices=null;
let installedVersion=null;
let updateResult=null;
let updateError='';
let updateChecking=false;
let memoryStatus=null;
const api = () => window.go?.main?.App || null;

function t(key,params={}){
  let value=translations[language][key]||key;
  for(const [k,v] of Object.entries(params)){value=value.replaceAll('{'+k+'}',String(v));}
  return value;
}
function node(tag,cls,text){
  const n=document.createElement(tag);
  if(cls)n.className=cls;
  if(text!==undefined&&text!==null)n.textContent=String(text);
  return n;
}
function btn(text,cls,action){
  const b=node('button',cls,text);b.type='button';b.addEventListener('click',action);return b;
}
function renderCopy(){
  document.documentElement.lang=language;
  document.documentElement.dir=language==='he'?'rtl':'ltr';
  document.body.dir=document.documentElement.dir;
  document.querySelectorAll('[data-i18n]').forEach((elem)=>{elem.textContent=t(elem.dataset.i18n);});
  document.querySelectorAll('[data-i18n-title]').forEach((elem)=>{elem.title=t(elem.dataset.i18nTitle);elem.setAttribute('aria-label',t(elem.dataset.i18nTitle));});
  document.querySelectorAll('[data-i18n-aria-label]').forEach((elem)=>{elem.setAttribute('aria-label',t(elem.dataset.i18nAriaLabel));});
  el('heBtn').classList.toggle('active',language==='he');
  el('enBtn').classList.toggle('active',language==='en');
  el('heBtn').setAttribute('aria-pressed',language==='he');
  el('enBtn').setAttribute('aria-pressed',language==='en');
}
function languageSwitch(lang){
  language=lang;
  try{localStorage.setItem('3mfBridgeLanguage',language);}catch{}
  renderCopy();renderQueue();renderInstalledTargets();renderTargets();renderResults();renderActionBar();renderUpdates();
  if(dom.settings.open){renderSettingsTabs();paintSettings();}
  window.SliceBridgeMyPrinters?.render();
}
// Only visible rows convert. Rows are hidden until My Printers selects a supported destination.
function getSelectedTargets(){
  return Array.from(document.querySelectorAll('#targetList input[type=checkbox]:checked:not(:disabled)'))
    .filter((e)=>!e.closest?.('.target-option')?.hidden).map((e)=>e.value);
}
let completedBatchFiles=0;
let progressPhase='idle';
let totalBatchFiles=0;
// Progress is derived from completed source projects, not guessed elapsed time.
// Native conversions use multiple targets concurrently within each file.
function renderProgress(){
  const text=el('progressDescription');
  const percent=el('progressPercent');
  const track=el('progressTrack');
  const fill=el('progressFill');
  if(!text||!percent||!track||!fill)return;
  const total=Math.max(1,totalBatchFiles||progress?.fileCount||files.length||1);
  const completed=Math.min(total,Math.max(0,completedBatchFiles));
  const value=Math.min(100,Math.floor(100*completed/total));
  const current=progress?.fileName || '';
  const label=progressPhase==='working'
    ? (language==='he'?'מעבד פרויקט':'Converting project')+' '+Math.min(total,completed+1)+'/'+total+(current?' · '+current:'')
    :progressPhase==='done'
      ? (language==='he'?'ההמרה הסתיימה':'Conversion finished')
      :progressPhase==='error'
        ? (language==='he'?'ההמרה הופסקה — בדוק תוצאות':'Conversion stopped — review results')
        : (language==='he'?'ממתין להמרה':'Waiting for conversion');
  text.textContent=label;
  percent.textContent=value+'%';
  fill.style.width=value+'%';
  track.setAttribute('aria-valuenow',String(value));
  track.setAttribute('aria-valuetext',label+', '+value+'%');
  track.classList.toggle('is-working',progressPhase==='working');
  track.classList.toggle('is-error',progressPhase==='error');
}
function renderActionBar(){
  renderProgress();
  const selected=getSelectedTargets().length;
  const work=files.length*selected;
  dom.convertCount.textContent=work>0?'('+work+')':'';
  dom.convertBtn.disabled=busy||!work;
  el('addFilesBtn').disabled=el('addFolderBtn').disabled=el('clearQueueBtn').disabled=busy;
  el('selectAllBtn').disabled=busy;
  el('settingsBtn').disabled=busy;
  el('myPrintersBtn').disabled=busy||!window.SliceBridgeMyPrinters?.isReady();
  dom.folderBtn.disabled=busy||!(lastBatch?.successes>0);
  if(busy){
    dom.activityIcon.className='activity-icon busy';
    dom.activityText.textContent=t('converting');
    dom.activityDetail.textContent=progress?t('convertingFile',{index:progress.fileIndex,total:progress.fileCount,name:progress.fileName}):'…';
    return;
  }
  if(memoryStatus){
    dom.activityIcon.className='activity-icon '+(memoryStatus.type||'');
    dom.activityText.textContent=memoryStatus.title;
    dom.activityDetail.textContent=memoryStatus.detail||'';
    return;
  }
  dom.activityIcon.className='activity-icon';
  dom.activityText.textContent=work?t('ready'):t('idle');
  dom.activityDetail.textContent=work?t('readyHint',{files:files.length,printers:selected}):t('idleHint');
}
function setActivity(title,detail,type=''){
  memoryStatus={title,detail,type};renderActionBar();
}
function renderQueue(){
  dom.fileList.replaceChildren();
  const hasFiles=files.length>0;
  dom.fileList.hidden=!hasFiles;
  dom.fileList.parentElement.classList.toggle('has-files',hasFiles);
  el('queuedFilesLabel').hidden=!hasFiles;
  el('queueFilesShownCount').textContent=hasFiles?String(files.length):'';
  dom.fileList.setAttribute('aria-label',t('queuedFiles'));
  dom.queueCount.textContent=String(files.length);
  el('clearQueueBtn').disabled=busy||!files.length;
  if(!files.length){
    renderActionBar();
    return;
  }
  const fragment=document.createDocumentFragment();
  files.forEach((file,index)=>{
    const item=node('div','queue-item');item.setAttribute('role','listitem');
    const origin=file.makerworld?.kind==='makerworld'&&file.makerworld?.confidence==='high';
    const badge=node('div',origin?'file-badge makerworld-badge':'file-badge',origin?'MW':'3MF');
    if(origin)badge.title=language==='he'?'הורד מ־MakerWorld':'Downloaded from MakerWorld';
    const info=node('div','queue-file-details');
    const name=node('strong','',file.name);name.title=file.path;
    const location=node('small','',file.folder+' · '+file.sizeLabel);location.title=file.folder;
    info.append(name,location);
    const remove=btn('×','remove-file-btn',()=>{
      if(busy)return;files.splice(index,1);memoryStatus=null;renderQueue();
    });
    remove.title=language==='he'?'הסר מהתור':'Remove from queue';remove.disabled=busy;
    item.append(badge,info,remove);fragment.append(item);
  });
  dom.fileList.append(fragment);
  renderActionBar();
}
// A destination is shown only when My Printers selects it. Built-in rows also need
// a passing Availability result; an unavailable adapter's row stays hidden.
function builtInVisible(target){
  const data=available.find((a)=>a.target===target);
  return data?.available===true&&Boolean(window.SliceBridgeMyPrinters?.isSelectedTarget(target));
}
function isSelectedDestination(id){
  return Boolean(window.SliceBridgeMyPrinters?.isSelectedTarget(id));
}
// Rows that become visible start checked, as a newly chosen printer should.
// Rows that stay visible keep their state; hidden rows are always unchecked.
function setRowVisible(row,checkbox,visible){
  const wasHidden=row.hidden;
  row.hidden=!visible;
  if(!visible)checkbox.checked=false;
  else if(wasHidden)checkbox.checked=true;
}
function renderTargets(){
  const builtIns=targetOrder.map((target)=>{
    const row=document.querySelector('[data-target-row="'+target+'"]');
    return {target,row,checkbox:row.querySelector('input')};
  });
  for(const {target,row,checkbox} of builtIns){
    const visible=builtInVisible(target);
    setRowVisible(row,checkbox,visible);
    // A running job temporarily locks controls; it must never erase the
    // selected targets for the next conversion.
    checkbox.disabled=busy||!visible;
    if(!visible)continue;
    const summary=row.querySelector('[data-printer-summary]');
    const pref=settings?.printers?.[target];
    let text=(pref?.nozzle&&pref.nozzle!=='auto'?pref.nozzle+' mm':t('autoNozzle'));
    if(pref?.slicer&&pref.slicer!=='auto')text+=' · '+pref.slicer;
    const materialCount=Object.values(pref?.filaments||{}).filter(Boolean).length;
    if(materialCount>0)text+=' · '+materialCount+' '+(language==='he'?'פרופילים':'presets');
    summary.textContent=text;
    summary.title=text;
  }
  for(const entry of installedRows){
    setRowVisible(entry.row,entry.checkbox,isSelectedDestination(entry.id));
    entry.checkbox.disabled=busy||entry.row.hidden;
  }
  const prusaRow=document.querySelector('[data-prusa-target]');
  if(prusaRow){
    const box=prusaCheckbox();
    const visible=prusaAvailable&&isSelectedDestination(prusaTargetId);
    setRowVisible(prusaRow,box,visible);
    box.disabled=busy||!visible;
  }
  // All available visible rows count, including installed Orca and Prusa.
  const shown=[...builtIns.map(({row,checkbox})=>({row,checkbox})),
    ...installedRows.map(({row,checkbox})=>({row,checkbox})),
    ...(prusaRow?[{row:prusaRow,checkbox:prusaCheckbox()}]:[])]
    .filter(({row,checkbox})=>!row.hidden&&!checkbox.disabled);
  const checkedShown=shown.filter(({checkbox})=>checkbox.checked).length;
  el('selectAllBtn').hidden=shown.length===0;
  el('selectAllBtn').textContent=checkedShown===shown.length?t('deselectAll'):t('selectAll');
  // Shown only once My Printers is ready and nothing is selected for conversion.
  const myPrintersReady=Boolean(window.SliceBridgeMyPrinters?.isReady());
  const none=myPrintersReady&&window.SliceBridgeMyPrinters.selectedTargetIds().length===0;
  el('emptyTargetNotice').hidden=!none;
  el('emptyTargetNotice').textContent=language==='he'?
    'לא נבחרו מדפסות יעד. אפשר להוסיף דרך סמל המדפסת בראש החלון.':
    'No target printers selected. Use the printer icon at the top to add them.';
  renderActionBar();
}
// Publishes the destination inputs to My Printers, which owns the supported list.
function publishSupportedTargets(){
  window.SliceBridgeMyPrinters?.setSupportedTargets({available,installedTargets,installedPrusaTargets,prusaAvailable});
}
async function refreshAvailability(){
  if(!api())return;
  try{
    const reply=await api().Availability();
    available=Array.isArray(reply)?reply:[];
  }
  catch(e){setActivity(t('settingsError'),e?.message||String(e),'warning');}
}
// Rows the backend marks ready. Readiness is checked again in the picker before a row can show.
function installedRowTargets(){
  return [...installedTargets,...installedPrusaTargets.filter((target)=>target?.ready===true)];
}
// Installed rows are rebuilt only when the backend list changes; checked state survives.
// Each row starts hidden unless My Printers selects it (see renderTargets).
function renderInstalledTargets(){
  const list=el('targetList');
  if(!list)return;
  const previous=new Set(installedRows.map(entry=>entry.id));
  const checked=new Set(installedRows.filter(entry=>entry.checkbox.checked).map(entry=>entry.id));
  for(const entry of installedRows)entry.row.remove();
  installedRows=[];
  const seen=new Set();
  for(const target of installedRowTargets()){
    if(typeof target?.id!=='string'||seen.has(target.id))continue;
    seen.add(target.id);
    const row=node('label','target-option installed-target');
    row.dataset.installedTarget=target.id;
    row.hidden=!isSelectedDestination(target.id);
    const checkbox=node('input');
    checkbox.type='checkbox';
    checkbox.value=target.id;
    // A newly discovered selected profile starts enabled. A row rebuilt
    // later preserves an explicit user uncheck instead of selecting it again.
    checkbox.checked=!row.hidden&&(!previous.has(target.id)||checked.has(target.id));
    checkbox.disabled=busy||row.hidden;
    const icon=node('span','target-printer-icon','▣');
    icon.setAttribute('aria-hidden','true');
    const label=node('span','printer-label');
    // The tile is a printer, not a technical profile summary. All profile IDs,
    // nozzle settings and slicer bindings remain intact for conversion.
    const vendor=target.vendor==='X 5 Series'?'QIDI':String(target.vendor||'').trim();
    const stem=String(target.model||target.title||'').trim()
      .replace(/\s+\d+(?:\.\d+)?(?:\s*\+\s*\d+(?:\.\d+)?)*\s*(?:mm)?(?:\s+nozzle)?$/i,'').trim();
    const lower=stem.toLocaleLowerCase('en'), prefix=vendor.toLocaleLowerCase('en');
    const fullName=lower===prefix||lower.startsWith(prefix+' ')?stem:[vendor,stem].filter(Boolean).join(' ');
    label.append(node('strong','',fullName));
    row.title=[fullName,target.nozzle?target.nozzle+' mm':null,target.slicer||null]
      .filter(Boolean).join(' · ');
    row.append(checkbox,icon,label);
    list.append(row);
    installedRows.push({id:target.id,row,checkbox});
  }
}
async function refreshInstalledTargets(){
  if(api()?.InstalledOrcaTargets){
    try{
      const reply=await api().InstalledOrcaTargets();
      installedTargets=Array.isArray(reply?.targets)?reply.targets:[];
    }catch(e){
      installedTargets=[];
      setActivity(t('settingsError'),e?.message||String(e),'warning');
    }
  }
  await refreshInstalledPrusaTargets();
  renderInstalledTargets();
  renderTargets();
  window.SliceBridgeMyPrinters?.refreshDestinationArt?.();
}
// Optional backend method. A missing method or a failed call means no per-printer
// PrusaSlicer targets this session; the built-in and Orca lists are unaffected.
async function refreshInstalledPrusaTargets(){
  if(!api()?.InstalledPrusaTargets){installedPrusaTargets=[];return;}
  try{
    const reply=await api().InstalledPrusaTargets();
    installedPrusaTargets=Array.isArray(reply?.targets)?reply.targets:[];
  }catch{installedPrusaTargets=[];}
}
// The PrusaSlicer row is shown only when the backend file probe finds the
// sidecar and My Printers selects it. A failed probe offers no target.
async function refreshPrusaEngine(){
  try{prusaAvailable=Boolean((await api()?.PrusaEngineStatus?.())?.available);}
  catch{prusaAvailable=false;}
}
async function addFiles(infos){
  if(!infos?.length)return;
  const known=new Set(files.map((f)=>f.path.toLowerCase()));
  const next=files.slice();
  for(const info of infos){
    if(!known.has(info.path.toLowerCase())){
      if(next.length>=200){setActivity(t('addError'),t('fileLimit'),'warning');return;}
      known.add(info.path.toLowerCase());
      next.push(info);
    }
  }
  if(next.length===files.length)return;
  files=next;
  memoryStatus=null;
  clearResults();
  renderQueue();
  // Check only already-selected files, without watching folders or scanning
  // unrelated downloads. Provenance can be absent, so unknown stays generic.
  if(api()?.DetectMakerWorldOrigins){
    const missing=next.filter(f=>!f.makerworld).map(f=>f.path);
    if(missing.length) {
      api().DetectMakerWorldOrigins(missing).then(found=>{
        let changed=false;
        for(const f of files){
          if(found?.[f.path]){f.makerworld=found[f.path];changed=true;}
        }
        if(changed)renderQueue();
      }).catch(()=>{}); // best-effort evidence, never block conversion
    }
  }
}
async function pickFiles(){
  if(busy||!api())return;
  try{await addFiles(await api().SelectFiles(language));}
  catch(e){setActivity(t('addError'),e?.message||String(e),'warning');}
}
async function pickFolder(){
  if(busy||!api())return;
  try{await addFiles(await api().SelectFolder(language));}
  catch(e){setActivity(t('addError'),e?.message||String(e),'warning');}
}
async function dropped(paths){
  if(busy||!api()||!paths?.length)return;
  try{await addFiles(await api().CollectInputs(paths));}
  catch(e){setActivity(t('addError'),e?.message||String(e),'warning');}
}

function addMetric(parent,label,value,full=false){
  const container=node('div',full?'detail-full':'');
  container.append(node('span','detail-key',label),node('span','detail-value',value));
  parent.append(container);
}
function filamentColors(filaments){
  const wrapper=node('div','color-list');
  for(const filament of filaments||[]){
    const token=node('span','color-token');
    const dot=node('i','color-dot');
    if(/^#[0-9a-fA-F]{6}$/.test(filament.color||''))dot.style.backgroundColor=filament.color;
    token.append(dot,node('span','',filament.type+' · '+filament.color));
    wrapper.append(token);
  }
  return wrapper;
}
function requiresToolUseReview(report){
  return Boolean(report?.user_slot_mapping_required || report?.tool_usage_verification_required);
}
function resultLine(parent,result,source){
  const row=node('div','result-row');
  const top=node('div','result-head');
  const content=node('div','result-content');
  const title=node('strong','',targetDisplayName(result.target));
  const subtitle=node('div','result-path',result.ok?result.outputName:result.error);
  subtitle.title=result.ok?result.outputPath:result.error;
  content.append(title,subtitle);
  const mappingReview=Boolean(result.ok&&requiresToolUseReview(result.report));
  const status=node('span','status-pill '+(result.ok?(mappingReview?'review':'good'):'bad'),
    result.ok?(mappingReview?t('summaryReview'):t('summarySuccess')):t('summaryFailure'));
  top.append(content,status);row.append(top);
  if(result.ok){
    const actions=node('div','result-actions');
    actions.append(btn(t('openSlicer')+' ↗','open-slicer',async ()=>{
      if(busy)return;
      try{await api().OpenInSlicer(result.target,result.outputPath);}
      catch(e){setActivity(t('launchError'),e?.message||String(e),'warning');}
    }));
    const details=node('details','report-details');
    const heading=node('summary','',t('details'));
    const grid=node('div','detail-grid');
    const report=result.report||{};
    if(report.user_slot_mapping_required){
      row.append(node('p','warning-line material-mapping-warning',
        t('virtualSlotsReview',{n:report.virtual_filament_slots,physical:report.physical_toolheads})));
    }
    if(report.tool_usage_verification_required){
      row.append(node('p','warning-line tool-usage-warning',t('toolUseReview')));
    }
    if(report.prime_tower_clearance_review_required){
      row.append(node('p','warning-line tower-clearance-warning',t('towerClearanceReview')));
    }
    if(report.prime_tower_clearance_review_required&&report.prime_tower_plate_coordinate_review_required){
      row.append(node('p','warning-line tower-per-plate-warning',
        t('towerPerPlateReview',{plates:report.prime_tower_plates,x:report.prime_tower_coordinate_x_entries,y:report.prime_tower_coordinate_y_entries})));
    }
    if(report.prime_tower_clearance_review_required&&report.prime_tower_plate_position_audit_status==='plate_count_unknown'){
      row.append(node('p','warning-line tower-plate-unknown-warning',t('towerPlateUnknownReview')));
    }
    addMetric(grid,t('source')+' '+t('nozzle'),(source.nozzle||'?')+' mm');
    addMetric(grid,t('target')+' '+t('nozzle'),(report.target_nozzle||'?')+' mm');
    addMetric(grid,t('materialSlots'),(report.source_filament_slots??(source.filaments||[]).length)+' → '+(report.target_filament_slots||'?'));
    addMetric(grid,t('triangles'),report.geometry_triangles??t('unknown'));
    addMetric(grid,t('objects'),(source.objects||'?')+' → '+(report.target_objects||'?'));
    addMetric(grid,t('instances'),(source.instances||'?')+' → '+(report.target_instances||'?'));
    addMetric(grid,t('plates'),(source.plates||'?')+' → '+(report.target_plates||'?'));
    addMetric(grid,t('geometry'),t('geometryPreserved'));
    addMetric(grid,t('cacheRemoved'),report.removed_slice_cache_entries??0);
    addMetric(grid,t('parameters'),(report.preserved_print_keys||[]).length);
    addMetric(grid,t('nativeProfile'),report.native_slicer||t('unknown'),true);
    const overridden=Object.entries(report.native_filament_overrides||{}).filter(([,p])=>p);
    addMetric(grid,t('overridden'),overridden.length?overridden.map(([m,p])=>m+': '+p).join(' · '):t('none'),true);
    if(source.filaments?.length){
      const colors=node('div','detail-full');
      colors.append(node('span','detail-key',t('materials')),filamentColors(source.filaments));
      grid.append(colors);
    }
    if(source.nozzleAssumed){
      grid.append(node('div','detail-full warning-line',t('sourceAssumed')));
    }else if(report.nozzle_remapped){
      grid.append(node('div','detail-full warning-line',t('switchedNozzle')));
    }
    if(report.printer_profile_generic){
      grid.append(node('div','detail-full warning-line',t('prusaGenericReview')));
    }
    details.append(heading,grid);
    actions.append(details);
    row.append(actions);
  }else{
    row.append(node('div','error-line',(result.code?result.code+': ':'')+(result.error||t('summaryFailure'))));
  }
  parent.append(row);
}
function renderResults(){
  dom.resultList.replaceChildren();
  el('clearResultsBtn').disabled=busy||!lastBatch;
  dom.reportCounts.replaceChildren();
  if(!lastBatch){
    const empty=node('div','empty-state');
    empty.append(
      node('div','empty-icon','↗'),
      node('h3','',t('emptyTitle')),
      node('p','',t('emptyDescription'))
    );
    dom.resultList.append(empty);return;
  }
  dom.reportCounts.append(
    node('span','passed','✓ '+lastBatch.successes),
    node('span','failed','× '+lastBatch.failures)
  );
  const fragment=document.createDocumentFragment();
  for(const file of lastBatch.files||[]){
    const block=node('article','file-report');
    const head=node('header','file-report-heading');
    const name=node('strong','',file.source.name);
    name.title=file.source.path;
    const done=node('span','file-report-meta',
      file.results.length+' / '+(file.results.length+file.failures.length));
    head.append(name,done);block.append(head);
    if(file.source.nozzle){
      const metrics=node('div','source-metrics');
      const types=[...new Set((file.source.filaments||[]).map((f)=>f.type))].join(', ');
      metrics.append(
        node('span','',t('source')+': '+file.source.nozzle+' mm'),
        node('span','',types||t('unknown')),
        node('span','',t('objects')+': '+(file.source.objects||'?')),
        node('span','',t('plates')+': '+(file.source.plates||'?'))
      );
      block.append(metrics);
    }
    for(const entry of [...file.results,...file.failures]){
      resultLine(block,entry,file.source);
    }
    if(file.error)block.append(node('p','warning-line',file.error));
    fragment.append(block);
  }
  dom.resultList.append(fragment);
}
function clearResults(){
  if(busy)return;
  lastBatch=null;
  progress=null;
  completedBatchFiles=0;totalBatchFiles=0;progressPhase='idle';
  memoryStatus=null;
  dom.resultList.scrollTop=0;
  renderResults();
  renderActionBar();
}
async function runBatch(){
  if(busy)return;
  if(files.length===0){setActivity(t('noSource'),'','warning');return;}
  const targets=getSelectedTargets();
  if(!targets.length){setActivity(t('noTargets'),'','warning');return;}
  busy=true;progress=null;memoryStatus=null;lastBatch=null;
  completedBatchFiles=0;totalBatchFiles=files.length;progressPhase='working';
  renderResults();renderQueue();renderTargets();renderActionBar();
  try{
    const response=await api().ConvertBatch({filePaths:files.map((f)=>f.path),targets});
    lastBatch=response;
    completedBatchFiles=Math.min(totalBatchFiles,response.filesDone||0);
    progressPhase=response.error?'error':'done';
    // A batch-level error outranks partial per-file success in the primary status.
    if(response.error){
      setActivity(t('convertError'),response.error,'warning');
    }else if(response.filesDone){
      const label=response.successes
        ? (response.failures?t('partialFinished'):t('finished'))
        : t('failed');
      setActivity(label,
        response.successes?t('finishedHint',{success:response.successes,failed:response.failures}):t('failedHint'),
        response.failures?'warning':'');
    }
    renderResults();
    el('resultsPanel').scrollTop=0;
  }catch(e){
    progressPhase='error';
    setActivity(t('convertError'),e?.message||String(e),'warning');
  }finally{
    busy=false;progress=null;
    renderResults();renderQueue();renderTargets();renderActionBar();
  }
}
async function openFolder(){
  try{await api().OpenOutputFolder();}
  catch(e){setActivity(t('launchError'),e?.message||String(e),'warning');}
}

function fillSelect(element,items,selected){
  element.replaceChildren();
  for(const option of items){
    const entry=node('option','',option.label);
    entry.value=option.value;
    if(option.disabled)entry.disabled=true;
    element.append(entry);
  }
  element.value=selected;
  if(element.selectedIndex<0&&element.options.length>0)element.selectedIndex=0;
}
function renderSettingsTabs(){
  const tabs=el('settingsTabs');
  tabs.replaceChildren();
  for(const target of targetOrder){
    const button=btn(printerNames[target],target===currentTarget?'active':'',async ()=>{
      if(currentTarget===target)return;
      currentTarget=target;
      currentMaterial='PLA';
      draftPrefs=structuredClone(settings?.printers?.[target]||{nozzle:'auto',slicer:'auto',filaments:{}});
      if(!draftPrefs.filaments)draftPrefs.filaments={};
      renderSettingsTabs();
      await paintSettings();
    });
    button.setAttribute('aria-pressed',target===currentTarget);
    tabs.append(button);
  }
}
async function promptForSettingsRecovery(error) {
  setActivity(t('settingsError'),error?.message||String(error),'warning');
  // Never reset damaged/future-version settings without an explicit choice.
  if(!api()?.RecoverInvalidSettings||!window.confirm(t('recoverSettingsQuestion')))return null;
  try{
    const recovered=await api().RecoverInvalidSettings(true);
    setActivity(t('recoveredSettings'),recovered.backupPath?t('backupSaved',{path:recovered.backupPath}):'');
    return recovered.settings;
  }catch(e){
    setActivity(t('settingsError'),e?.message||String(e),'warning');
    return null;
  }
}
async function openSettings(target){
  if(busy||!api())return;
  currentTarget=target||'u1';
  currentMaterial='PLA';
  try{settings=await api().GetSettings();}
  catch(e){
    settings=await promptForSettingsRecovery(e);
    if(!settings)return;
  }
  draftPrefs=structuredClone(settings.printers[currentTarget]||{nozzle:'auto',slicer:'auto',filaments:{}});
  if(!draftPrefs.filaments)draftPrefs.filaments={};
  renderSettingsTabs();
  dom.settings.showModal();
  await paintSettings();
}
let choiceSequence=0;
async function paintSettings(){
  if(!draftPrefs)return;
  const nozzle=el('settingsNozzle');
  const slicer=el('settingsSlicer');
  const material=el('settingsMaterial');
  const filaments=el('settingsFilament');
  el('settingsStatus').textContent='';
  fillSelect(nozzle,[
    {value:'auto',label:t('autoNozzle')},
    ...nozzleChoices[currentTarget].map((n)=>({value:n,label:n+' mm'}))
  ],draftPrefs.nozzle||'auto');
  material.value=currentMaterial;
  slicer.disabled=true;filaments.disabled=true;
  const seq=++choiceSequence;
  const effectiveNozzle=draftPrefs.nozzle==='auto'?'0.4':draftPrefs.nozzle;
  try{
    const options=await api().PrinterOptions(currentTarget,effectiveNozzle,draftPrefs.slicer||'auto',currentMaterial);
    if(seq!==choiceSequence||!dom.settings.open)return;
    formChoices=options;
    fillSelect(slicer,[
      {value:'auto',label:t('autoSlicer')},
      ...(options.slicers||[]).map((s)=>({
        value:s.id,label:s.name+(s.installed?'':' — '+t('unsupportedPrinter')),
        disabled:!s.installed
      }))
    ],draftPrefs.slicer||'auto');
    slicer.disabled=false;
    const autoNozzle=!draftPrefs.nozzle||draftPrefs.nozzle==='auto';
    fillSelect(filaments,[
      {value:'auto',label:t('autoFilament')},
      ...(!autoNozzle?(options.filaments||[]):[]).map((n)=>({value:n,label:n}))
    ],autoNozzle?'auto':draftPrefs.filaments[currentMaterial]||'auto');
    filaments.disabled=autoNozzle;
    if(autoNozzle){
      el('settingsStatus').textContent=Object.values(draftPrefs.filaments||{}).some(Boolean)
        ?t('autoPresetWarning'):t('autoPresetHint');
    }else if(options.error)el('settingsStatus').textContent=options.error;
    else if(!options.filaments?.length)el('settingsStatus').textContent=t('noNativeProfiles');
  }catch(e){
    if(seq!==choiceSequence)return;
    fillSelect(slicer,[{value:'auto',label:t('autoSlicer')}],draftPrefs.slicer||'auto');
    fillSelect(filaments,[{value:'auto',label:t('autoFilament')}],draftPrefs.filaments[currentMaterial]||'auto');
    slicer.disabled=false;filaments.disabled=false;
    el('settingsStatus').textContent=e?.message||String(e);
  }
}
async function saveSettings(){
  if(!draftPrefs||!api())return;
  const button=el('saveSettingsBtn');button.disabled=true;
  try{
    const nozzle=el('settingsNozzle').value;
    const slicer=el('settingsSlicer').value;
    draftPrefs.nozzle=nozzle;
    draftPrefs.slicer=slicer;
    if((!nozzle||nozzle==='auto')&&Object.values(draftPrefs.filaments||{}).some(Boolean)){
      el('settingsStatus').textContent=t('autoPresetWarning');
      return;
    }
    const next=await api().SavePrinterSettings(currentTarget,draftPrefs);
    settings=next;
    renderTargets();
    dom.settings.close();
    setActivity(t('saved'),printerNames[currentTarget]);
  }catch(e){
    el('settingsStatus').textContent=e?.message||String(e);
  }finally{button.disabled=false;}
}

function renderUpdates(){
  const banner=el('updateNotice');
  const open=el('openUpdateBtn');
  const status=el('updateStatus');
  el('installedVersion').textContent=installedVersion?'v'+installedVersion.version:'';
  if(installedVersion?.commit)el('installedVersion').title=installedVersion.commit;
  el('checkUpdatesBtn').disabled=updateChecking;
  open.hidden=true;open.dataset.kind='';open.dataset.tag='';
  status.removeAttribute('data-status');
  if(updateChecking){banner.hidden=false;status.textContent=t('checkingUpdates');return;}
  if(updateError){
    banner.hidden=false;status.textContent=t('updateError')+': '+updateError;status.dataset.status='error';return;
  }
  if(!updateResult){banner.hidden=true;return;}
  banner.hidden=false;
  status.dataset.status=updateResult.status;
  switch(updateResult.status){
    case 'update_available':
      status.textContent=t('updateAvailable',{v:updateResult.latestVersion});
      open.hidden=false;open.textContent=t('openRelease');open.dataset.kind='release';
      open.dataset.tag=updateResult.releaseTag||'';break;
    case 'up_to_date':status.textContent=t('latestInstalled');break;
    case 'no_releases':
      status.textContent=t('noPublishedReleases');open.hidden=false;
      open.textContent=t('openPreviewBuilds');open.dataset.kind='builds';break;
    case 'platform_unavailable':
      status.textContent=t('platformUnavailable',{v:updateResult.latestVersion});open.hidden=false;
      open.textContent=t('openReleases');open.dataset.kind='releases';break;
    case 'authentication_required':
      status.textContent=t('authRequired');open.hidden=false;
      open.textContent=t('openReleases');open.dataset.kind='releases';break;
    default:status.textContent=t('unsupportedPlatform');
  }
}
async function checkUpdates(){
  if(updateChecking||!api()?.CheckForUpdates)return;
  updateChecking=true;updateResult=null;updateError='';renderUpdates();
  try{updateResult=await api().CheckForUpdates();}
  catch(e){updateError=e?.message||String(e);}
  finally{updateChecking=false;renderUpdates();}
}
async function openUpdate(){
  const button=el('openUpdateBtn');
  if(!button.dataset.kind)return;
  try{await api().OpenUpdatePage(button.dataset.kind,button.dataset.tag||'');}
  catch(e){updateError=e?.message||String(e);renderUpdates();}
}
function selectAll(){
  const boxes=Array.from(document.querySelectorAll('#targetList .target-option input[type=checkbox]:not(:disabled)'))
    .filter(box=>!box.closest('.target-option').hidden);
  if(!boxes.length)return;
  const allSelected=boxes.every(box=>box.checked);
  for(const box of boxes)box.checked=!allSelected;
  memoryStatus=null;renderTargets();
}
function registerHandlers(){
  el('addFilesBtn').addEventListener('click',pickFiles);
  el('addFolderBtn').addEventListener('click',pickFolder);
  el('clearQueueBtn').addEventListener('click',()=>{
    if(busy)return;files=[];memoryStatus=null;renderQueue();
  });
  el('selectAllBtn').addEventListener('click',selectAll);
  el('convertBtn').addEventListener('click',runBatch);
  el('openFolderBtn').addEventListener('click',openFolder);
  el('clearResultsBtn').addEventListener('click',()=>{if(!busy)clearResults();});
  el('settingsBtn').addEventListener('click',()=>openSettings('u1'));
  el('closeSettingsBtn').addEventListener('click',()=>dom.settings.close());
  el('cancelSettingsBtn').addEventListener('click',()=>dom.settings.close());
  el('saveSettingsBtn').addEventListener('click',saveSettings);
  el('settingsNozzle').addEventListener('change',async (e)=>{
    draftPrefs.nozzle=e.target.value;await paintSettings();
  });
  el('settingsSlicer').addEventListener('change',async (e)=>{
    draftPrefs.slicer=e.target.value;await paintSettings();
  });
  el('settingsMaterial').addEventListener('change',async (e)=>{
    currentMaterial=e.target.value;await paintSettings();
  });
  el('settingsFilament').addEventListener('change',(e)=>{
    const profile=e.target.value;
    if(profile==='auto')delete draftPrefs.filaments[currentMaterial];
    else draftPrefs.filaments[currentMaterial]=profile;
    el('settingsStatus').textContent='';
  });
  dom.settings.addEventListener('close',()=>{draftPrefs=null;choiceSequence++;});
  for(const target of targetOrder){
    document.querySelector('[data-target-row="'+target+'"] input')
      .addEventListener('change',()=>{memoryStatus=null;renderTargets();});
  }
  el('heBtn').addEventListener('click',()=>languageSwitch('he'));
  el('enBtn').addEventListener('click',()=>languageSwitch('en'));
  el('checkUpdatesBtn').addEventListener('click',checkUpdates);
  el('openUpdateBtn').addEventListener('click',openUpdate);
  // My Printers changed (saved, or availability/installed targets refreshed):
  // rows follow the supported selection, newly chosen rows start checked.
  window.addEventListener('slicebridge:my-printers-changed',()=>renderTargets());
  el('dismissUpdateBtn').addEventListener('click',()=>{
    updateResult=null;updateError='';renderUpdates();
  });
  const drop=el('dropzone');
  drop.addEventListener('keydown',(e)=>{
    if(e.key==='Enter'||e.key===' '){e.preventDefault();pickFiles();}
  });
  document.addEventListener('dragover',(e)=>e.preventDefault());
  document.addEventListener('drop',(e)=>e.preventDefault());
  document.addEventListener('keydown',(e)=>{
    if(dom.settings.open||busy)return;
    if((e.ctrlKey||e.metaKey)&&e.key.toLowerCase()==='o'){
      e.preventDefault();if(e.shiftKey)pickFolder();else pickFiles();
    }else if((e.ctrlKey||e.metaKey)&&e.key===','){
      e.preventDefault();openSettings('u1');
    }
  });
  // A real desktop window: user-controlled size, no auto-fit or re-centering.
  // Persist only after resize settles, not on every layout/language change.
  let resizeTimer;
  window.addEventListener('resize',()=>{
    clearTimeout(resizeTimer);
    resizeTimer=setTimeout(()=>{
      if(api()?.RememberWindowSize)api().RememberWindowSize().catch(()=>{});
    },900);
  });
}
async function initialiseNative(){
  for(let n=0;n<80&&!api();n++)await new Promise((resolve)=>setTimeout(resolve,75));
  if(!api()){
    setActivity(t('convertError'),t('unsupportedPlatform'),'warning');return;
  }
  if(window.runtime?.OnFileDrop){
    window.runtime.OnFileDrop((_x,_y,paths)=>dropped(paths),true);
  }
  if(window.runtime?.EventsOn){
    window.runtime.EventsOn('bridge:batch-progress',(data)=>{
      if(!busy||!data)return;
      progress=data;
      totalBatchFiles=Math.max(1,data.fileCount||totalBatchFiles);
      if(data.completed){
        completedBatchFiles=Math.max(completedBatchFiles,data.fileIndex||0);
      }else{
        completedBatchFiles=Math.max(completedBatchFiles,(data.fileIndex||1)-1);
      }
      renderActionBar();
    });
    window.runtime.EventsOn('bridge:makerworld-imported',(event)=>{
      const file=event?.file;
      if(!file?.path)return;
      file.makerworld=event.source||{kind:'makerworld',confidence:'high'};
      addFiles([file]).catch((e)=>setActivity(t('addError'),String(e),'warning'));
    });
    window.runtime.EventsOn('bridge:makerworld-import-failed',(message)=>{
      // Native backend deliberately returns sanitized messages without signed URLs.
      setActivity('MakerWorld',String(message||'Import failed'),'warning');
    });
  }
  try{settings=await api().GetSettings();}
  catch(e){setActivity(t('settingsError'),e?.message||String(e),'warning');}
  try{installedVersion=await api().GetVersion();}
  catch{installedVersion=null;}
  renderUpdates();
  await refreshAvailability();
  await refreshInstalledTargets();
  await refreshPrusaEngine();
  // My Printers builds its supported list from these inputs, so they are handed
  // over once, after every probe and before its own init reads them.
  publishSupportedTargets();
  await window.SliceBridgeMyPrinters?.init();
  renderCopy();renderTargets();renderQueue();renderResults();renderActionBar();
  // The Go startup handler can receive external-app URLs before Wails event
  // subscriptions exist. Release its bounded queue only AFTER this UI and
  // both MakerWorld imported/failed event listeners have been registered.
  if(window.runtime?.EventsOn && api()?.MakerWorldImportEventsReady){
    try{await api().MakerWorldImportEventsReady();}
    catch(e){setActivity('MakerWorld',String(e),'warning');}
  }
}

registerHandlers();
renderCopy();renderQueue();renderResults();renderActionBar();renderUpdates();
// Run after all deferred frontend modules register their APIs. Fast native RPCs
// must not race my-printers.js loading and silently skip its initialisation.
setTimeout(initialiseNative,0);
