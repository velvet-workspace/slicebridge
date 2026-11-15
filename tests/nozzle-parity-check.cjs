"use strict";
// Go/Rust nozzle parity. Read-only: never slices or creates 3MF/G-code.
const fs=require("node:fs"),os=require("node:os"),path=require("node:path");
const child=require("node:child_process"),assert=require("node:assert/strict");
const root=path.resolve(__dirname,".."), dir=path.join(root,"tests","fixtures");
const sources=fs.readdirSync(dir).filter(f=>f.endsWith(".3mf")).sort();
assert.ok(sources.length>=10,"original 3MF fixtures missing");
const full=sources.map(f=>path.join(dir,f));
const rust=path.join(root,"rust","slicebridge-alpha","target","debug",
  os.platform()==="win32"?"slicebridge-alpha.exe":"slicebridge-alpha");
assert.ok(fs.existsSync(rust),"Rust CLI must be built first");
const oracle=JSON.parse(child.execFileSync("go",
  ["run","../tests/nozzle-parity/inspect.go",...full],
  {cwd:path.join(root,"engine"),encoding:"utf8",maxBuffer:1024*1024}));
let matched=0;
for(const file of sources) {
  assert.ok(oracle[file],"original Go engine did not report "+file);
  const reply=JSON.parse(child.execFileSync(rust,[
    "source.inspect",JSON.stringify({path:path.join(dir,file)})
  ],{encoding:"utf8",maxBuffer:1024*1024}));
  assert.equal(reply.ok,true,"Rust inspection failed for "+file);
  for(const field of ["source_nozzle","source_nozzle_source","source_nozzle_assumed"]) {
    assert.deepEqual(reply.data[field],oracle[file][field],
      "Go/Rust nozzle mismatch "+file+" "+field);
  }
  matched++;
}
console.log(JSON.stringify({status:"SOURCE_NOZZLE_GO_RUST_PARITY_PASS",
  fixtures:matched, fields:["source_nozzle","source_nozzle_source","source_nozzle_assumed"],
  platform:os.platform()}));
