'use strict';
// Manual rendered-layout QA in a real Chromium engine. No Wails RPC, printer or
// slicer actions are invoked; the file selection is a local synthetic fixture.
const fs=require('node:fs');
const os=require('node:os');
const path=require('node:path');
const {spawnSync}=require('node:child_process');
const assert=require('node:assert/strict');
const {pathToFileURL}=require('node:url');
const src=path.resolve(__dirname,'../frontend/dist');
// WebView2 uses Edge on Windows; prefer its Chromium renderer and retain Chrome
// as a fallback if the runner image has a broken headless browser launch.
// CHROME_BIN stays strict when explicitly provided (useful for local diagnosis).
const preferredBrowsers=process.env.CHROME_BIN?[process.env.CHROME_BIN]:
  process.platform==='win32'?[
    'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe',
    'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'
  ]:process.platform==='darwin'
    ?['/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge']
    :['/usr/bin/google-chrome'];
const browsers=[...new Set(preferredBrowsers)].filter(p=>fs.existsSync(p));
if(!browsers.length)throw Error('No Chromium renderer found: '+preferredBrowsers.join(', '));
console.log('CHROMIUM_QA_CANDIDATES',browsers.join(' | '));

const original=fs.readFileSync(path.join(src,'index.html'),'utf8');
const fixtureScript=[
  "(function(){",
  "  const query=new URL(location.href).searchParams;",
  "  const count=Number(query.get('files')||'0');",
  "  const lang=query.get('lang')||'he';",
  "  const motion=query.get('motion')||'default';",
  "  document.documentElement.lang=lang;",
  "  document.documentElement.dir=lang==='he'?'rtl':'ltr';",
  "  document.body.dir=document.documentElement.dir;",
  "  const panel=document.querySelector('.queue-panel');",
  "  const list=document.getElementById('fileList');",
  "  const heading=document.getElementById('queuedFilesLabel');",
  "  panel.classList.toggle('has-files',count>0);",
  "  heading.hidden=count===0;",
  "  list.hidden=count===0;",
  "  document.getElementById('queueCount').textContent=String(count);",
  "  document.getElementById('queueFilesShownCount').textContent=String(count);",
  "  for(let i=0;i<count;i++){",
  "    const item=document.createElement('div');",
  "    item.className='queue-item';",
  "    item.setAttribute('role','listitem');",
  "    const badge=document.createElement('span');",
  "    badge.className='file-badge';badge.textContent='3MF';",
  "    const info=document.createElement('div');",
  "    info.className='queue-file-details';",
  "    const name=document.createElement('strong');",
  "    name.textContent='11-Box+Kettle+large+complex-project-variant-'+(i+1)+'.3mf';",
  "    const small=document.createElement('small');",
  "    small.textContent='C:/models/downloaded · 5.8 MB';",
  "    info.append(name,small);",
  "    const remove=document.createElement('button');",
  "    remove.className='remove-file-btn';remove.textContent='×';",
  "    item.append(badge,info,remove);list.append(item);",
  "  }",
  "  function rect(q){const r=document.querySelector(q).getBoundingClientRect();",
  "    return {x:r.x,y:r.y,width:r.width,height:r.height,bottom:r.bottom};}",
  "  function report(){",
  "    const add=document.getElementById('addFilesBtn');",
  "    if(motion==='reduce')document.getElementById('activityIcon').classList.add('busy');",
  "    const out={",
  "      viewport:{width:innerWidth,height:innerHeight},",
  "      language:document.documentElement.lang,",
  "      files:count,",
  "      motion:{enabled:matchMedia('(prefers-reduced-motion: reduce)').matches,",
  "        buttonTransition:getComputedStyle(add).transitionDuration,",
  "        targetTransition:getComputedStyle(document.querySelector('.target-option')).transitionDuration,",
  "        busySpinner:getComputedStyle(document.getElementById('activityIcon'),'::after').animationName},",
  "      panel:rect('.queue-panel'),",
  "      targets:rect('.targets-panel'),",
  "      banner:rect('.brand-banner'),",
  "      bannerText:rect('.brand-banner-content'),",
  "      list:rect('.queue-list'),",
  "      heading:rect('.queue-files-heading'),",
  "      first:count?rect('.queue-item'):null,",
  "      footnote:rect('#queueFootnote'),",
  "      workspace:rect('.workspace'),",
  "      results:rect('.results-panel'),",
  "      panelDisplay:getComputedStyle(panel).display,",
  "      listOverflow:getComputedStyle(list).overflowY,",
  "      scrollableFiles:list.scrollHeight-list.clientHeight,",
  "      listClientHeight:list.clientHeight,",
  "      scrollableWorkspace:document.querySelector('.workspace').scrollHeight-document.querySelector('.workspace').clientHeight,",
  "      resultsOverflowX:document.querySelector('.results-panel').scrollWidth-document.querySelector('.results-panel').clientWidth,",
  "      docOverflow:document.documentElement.scrollHeight-innerHeight",
  "    };",
  "    // Record original panel geometry before changing scroll position.",
  "    add.scrollIntoView({block:'center'});",
  "    const a=add.getBoundingClientRect();",
  "    const hit=document.elementFromPoint(a.x+a.width/2,a.y+a.height/2);",
  "    out.addButtonClickable=add.contains(hit);",
  "    document.documentElement.setAttribute('data-layout-qa',encodeURIComponent(JSON.stringify(out)));",
  "  }",
  "  setTimeout(report,180);",
  "})();"
].join('\n');
const fixture=original
 .replace(/<script defer src="\.\/[^"]+"[^>]*><\/script>/g,'')
 .replace('</body>','<script>'+fixtureScript+'</script></body>');
const markerDir=fs.mkdtempSync(path.join(src,'.source-qa-'));
const browserProfileRoot=fs.mkdtempSync(path.join(os.tmpdir(),'slicebridge-chromium-'));
const htmlPath=path.join(markerDir,'fixture.html');
fs.writeFileSync(htmlPath,fixture,'utf8');
// Resolve the CSS and icon from the actual frontend dist; do not copy them.
const testHTML=fs.readFileSync(htmlPath,'utf8')
 .replace('href="./app.css"','href="../app.css"')
 .replace('src="./brand-icon.png"','src="../brand-icon.png"');
fs.writeFileSync(htmlPath,testHTML,'utf8');
const trials=process.env.QUEUE_LAYOUT_SINGLE?[[1280,900,Number(process.env.QUEUE_LAYOUT_SINGLE),'he']]:[
  [1100,750,1,'he'],
  [1100,750,2,'he'],
  [1100,750,2,'en'],
  [1100,750,15,'he'],
  [1280,900,1,'he'],
  [1280,900,2,'en'],
  [1280,900,12,'en'],
  [780,580,1,'he'],
  [780,580,8,'en'],
  [746,620,2,'he'],
  [746,620,2,'en'],
  [760,620,2,'he'],
  [760,620,2,'en'],
  // On Windows headless Chrome reserves 16px of outer width.
  // These two widths exercise the *actual* 745/746px responsive boundary.
  [761,620,2,'he'],
  [761,620,2,'en'],
  [762,620,2,'he'],
  [762,620,2,'en'],
  [820,620,2,'he'],
  [820,620,2,'en'],
  [960,620,2,'he'],
  [960,620,2,'en'],
  [900,830,2,'he'],
  [900,830,2,'en'],
  [900,620,2,'he'],
  [900,620,2,'en'],
  [720,500,2,'he'],
  [720,500,2,'en'],
  [1100,750,0,'he'],
  // The same renderer also tests actual computed styles under OS motion preference.
  [1100,750,0,'he','reduce'],
  [1100,750,0,'en','reduce']
];
// The whole geometry suite must pass in a real Chromium renderer. A headless
// launch failure is not a layout failure: report it and try another installed
// Chromium binary instead of silently dropping any viewport/assertions.
let selectedBrowser=0;
function renderWithChromium(args,url,scenario,requestedWidth,requestedHeight){
  const failures=[];
  for(let i=selectedBrowser;i<browsers.length;i++){
    const browser=browsers[i];
    const profile=path.join(browserProfileRoot,'profile-'+i);
    const p=spawnSync(browser,[...args,'--user-data-dir='+profile,url],{
      encoding:'utf8',timeout:30000,maxBuffer:8*1024*1024
    });
    let geometry=null, renderProblem=null;
    if(p.status===0){
      const match=(p.stdout||'').match(/data-layout-qa="([^"]+)"/);
      try{if(match)geometry=JSON.parse(decodeURIComponent(match[1]));}
      catch(e){renderProblem='invalid geometry report: '+e.message;}
      if(!geometry)renderProblem=renderProblem||'no geometry report';
      else if(geometry.viewport.width<requestedWidth-20 ||
              geometry.viewport.height<requestedHeight-160 ||
              geometry.viewport.height>requestedHeight+2)
        renderProblem='renderer ignored requested viewport: '+JSON.stringify(geometry.viewport);
      if(!renderProblem){
        if(i!==selectedBrowser||!fs.existsSync(path.join(browserProfileRoot,'.selected'))){
          console.log('CHROMIUM_QA_SELECTED',path.basename(browser));
          fs.writeFileSync(path.join(browserProfileRoot,'.selected'),String(i));
        }
        selectedBrowser=i;
        return p;
      }
    }
    const failure={
      scenario,browser,status:p.status,signal:p.signal,renderProblem,
      error:p.error?{code:p.error.code,message:p.error.message}:null,
      stderr:(p.stderr||'').slice(-1200),
      stdout:(p.stdout||'').slice(-600)
    };
    console.error('CHROMIUM_QA_LAUNCH_FAILURE',JSON.stringify(failure));
    failures.push(failure);
  }
  throw Error('No headless Chromium renderer could complete '+scenario+': '+JSON.stringify(failures));
}
try{
  for(const [w,h,files,lang,motion='default'] of trials){
    const url=pathToFileURL(htmlPath).href+'?files='+files+'&lang='+lang+'&motion='+motion;
    const args=[
      '--headless=new','--disable-gpu','--disable-extensions',
      '--disable-background-networking','--disable-background-mode',
      '--disable-component-update','--disable-sync',
      '--no-first-run','--no-default-browser-check','--no-sandbox',
      '--window-size='+w+','+h,
      '--virtual-time-budget=2500','--dump-dom',
      ...(motion==='reduce'?['--force-prefers-reduced-motion=reduce']:[])
    ];
    const p=renderWithChromium(args,url,w+'x'+h+' '+lang+' files='+files,w,h);
    const m=p.stdout.match(/data-layout-qa="([^"]+)"/);
    if(!m)throw Error('No computed-geometry report: '+p.stdout.slice(-1000)+' '+p.stderr.slice(-1000));
    const q=JSON.parse(decodeURIComponent(m[1]));
    assert.equal(q.language,lang);
    if(motion==='reduce'){
      assert.equal(q.motion.enabled,true,'Chromium did not enable reduced motion');
      assert.equal(q.motion.buttonTransition,'0s','button still animates under reduced motion');
      assert.equal(q.motion.targetTransition,'0s','printer option still animates under reduced motion');
      assert.equal(q.motion.busySpinner,'none','busy activity icon still spins under reduced motion');
      console.log('REDUCED_MOTION_RENDER_PASS',lang,JSON.stringify(q.motion));
    }
    assert.ok(q.bannerText.y>=q.banner.y-2 && q.bannerText.bottom<=q.banner.bottom+2,
      'fixed banner text is clipped in this viewport: '+JSON.stringify(q));
    // Use the actual Chromium viewport; its virtual chrome may reduce height.
    assert.ok(q.viewport.width>=w-20,'renderer ignored requested test width');
    // Headless Chrome may reserve 151px for window decorations. Reject render
    // heights outside that measured tolerance in either direction.
    assert.ok(q.viewport.height>=h-160 && q.viewport.height<=h+2,
      'renderer ignored requested compact-window height: '+JSON.stringify(q.viewport));
    assert.ok(q.panel.width>320,'source panel width too narrow');
    const overlap=(a,b)=>Math.max(0,Math.min(a.x+a.width,b.x+b.width)-Math.max(a.x,b.x)) *
      Math.max(0,Math.min(a.bottom,b.bottom)-Math.max(a.y,b.y));
    assert.ok(overlap(q.panel,q.results)<=2,'results panel overlaps source panel: '+JSON.stringify(q));
    assert.ok(overlap(q.targets,q.results)<=2,'results panel overlaps target printers: '+JSON.stringify(q));
    assert.ok(q.results.width>=300 && q.resultsOverflowX<=2,
      'results text area is cramped or horizontally clipped: '+JSON.stringify(q));
    if(q.viewport.width>=746){
      assert.ok(lang==='he' ? q.panel.x>q.results.x : q.panel.x<q.results.x,
        'source/results order does not respect reading direction: '+JSON.stringify(q));
    }
    assert.ok(q.addButtonClickable,
      'Add files button is intercepted by another panel at '+w+'x'+h+': '+JSON.stringify(q));
    if(files){
      assert.ok(q.panel.height>=178 && q.panel.height<=355,
        'source panel not content-sized: '+JSON.stringify(q));
      if(files<=2)assert.ok(q.panel.height<300,'few files should not consume most workspace');
      assert.ok(q.listClientHeight>=50,'file list clipped: '+JSON.stringify(q));
      assert.ok(q.first.y>=q.list.y-2,'first row above scroll frame');
      assert.ok(q.first.bottom<=q.list.bottom+2,'first row below scroll frame');
      assert.ok(q.footnote.bottom<=q.panel.bottom+2,'queue footnote outside panel');
      assert.equal(q.listOverflow,'auto');
      if(files>=8)assert.ok(q.scrollableFiles>100,'long file list fails to scroll');
      if(files<=2 && q.viewport.width>=1000 && q.viewport.height>=720){
        assert.ok(q.targets.bottom<=q.workspace.bottom+2,
          'target-printer panel hidden below workspace with few source files: '+JSON.stringify(q));
        assert.ok(q.scrollableWorkspace<=2,
          'unexpected workspace scrolling for few files in normal desktop window: '+JSON.stringify(q));
      }
    }else{
      assert.ok(q.panel.height<300,'empty queue should stay compact');
    }
    assert.ok(q.docOverflow<=1,'document itself scrolled (should scroll workspace)');
    if(h<=580&&files)assert.ok(q.scrollableWorkspace>100,'compact view missing workspace scroll');
    console.log('QUEUE_RENDER_PASS',w+'x'+h,'actual='+q.viewport.width+'x'+q.viewport.height,lang,'files='+files,
      'panel='+Math.round(q.panel.height),'targetsBottom='+Math.round(q.targets.bottom),'workspaceBottom='+Math.round(q.workspace.bottom),
      'list='+q.listClientHeight,'listScroll='+q.scrollableFiles,
      'workspaceScroll='+q.scrollableWorkspace);
  }
}finally{
 fs.rmSync(markerDir,{recursive:true,force:true});
 fs.rmSync(browserProfileRoot,{recursive:true,force:true,maxRetries:5,retryDelay:100});
}
