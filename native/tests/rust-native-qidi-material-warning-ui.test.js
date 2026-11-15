'use strict';
const {test}=require('node:test');
const assert=require('node:assert/strict');
const fs=require('node:fs');
const path=require('node:path');
const vm=require('node:vm');

const app=fs.readFileSync(path.resolve(__dirname,'../frontend/dist/app.js'),'utf8');
const start=app.indexOf('const translations = ')+'const translations = '.length;
const end=app.indexOf('\nconst printerNames',start);
const translations=vm.runInNewContext('('+app.slice(start,end).trim().replace(/;$/,'')+')');
const resultStart=app.indexOf('function resultLine(parent,result,source){');
const resultEnd=app.indexOf('\nfunction renderResults(){',resultStart);
assert.ok(resultStart>=0&&resultEnd>resultStart,'actual result renderer present');

function render(lang,flag) {
  function node(tag,cls='',text=''){
    return {tag,cls,text,children:[],append(...children){this.children.push(...children);}};
  }
  const root=node('section');
  const result={ok:true,target:'qidi',outputName:'converted.3mf',outputPath:'/output.3mf',
    report:{qidi_fifth_petg_preset_review_required:flag}};
  const source={nozzle:'0.4',filaments:[]};
  const context={root,result,source,node,
    targetDisplayName:()=> 'QIDI X-Plus 5',
    t:key=>translations[lang][key]||key,
    btn:(label)=>node('button','',label),
    addMetric:(parent,label,value)=>parent.append(node('span','',String(value))),
    filamentColors:()=>node('div'),
    busy:false,
    api:()=>({OpenInSlicer:async()=>{}}),
    setActivity:()=>{}};
  vm.runInNewContext(app.slice(resultStart,resultEnd)+'\nresultLine(root,result,source);',context);
  return root.children[0].children.filter(item=>item.cls.includes('material-mapping-warning'));
}

test('real QIDI Rust-native receipt displays an actionable, visible HE/EN material warning',()=>{
  for(const lang of ['he','en']){
    const notices=render(lang,true);
    assert.equal(notices.length,1);
    assert.match(notices[0].text,/PETG/);
    assert.match(notices[0].text,/PLA Rapido/);
    assert.match(notices[0].text,/Generic PETG/);
    assert.match(notices[0].text,/5/);
  }
});

test('other successful converters are not burdened with a speculative QIDI warning',()=>{
  assert.equal(render('he',false).length,0);
  assert.equal(render('en',undefined).length,0);
});
