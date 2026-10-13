import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createHash} from 'node:crypto';
import {mkdtempSync,writeFileSync,rmSync} from 'node:fs';
import {tmpdir} from 'node:os';
import {join} from 'node:path';
import {overlayState} from './prepare-helper.mjs';
const sha=s=>createHash('sha256').update(s).digest('hex');
test('helper overlay validates all files before applying and recognizes repeat setup',()=>{
 const dir=mkdtempSync(join(tmpdir(),'helper-overlay-'));
 try {
  const manifest={files:[{path:'source.rs',before:sha('old'),after:sha('new')},{path:'added.rs',before:null,after:sha('added')}]};
  writeFileSync(join(dir,'source.rs'),'old');
  assert.equal(overlayState(dir,manifest),'base');
  writeFileSync(join(dir,'source.rs'),'human edit');
  assert.throws(()=>overlayState(dir,manifest),/preserve/i);
  writeFileSync(join(dir,'source.rs'),'new');writeFileSync(join(dir,'added.rs'),'added');
  assert.equal(overlayState(dir,manifest),'applied');
  writeFileSync(join(dir,'source.rs'),'old');
  assert.throws(()=>overlayState(dir,manifest),/mixed/i);
  assert.throws(()=>overlayState(dir,{files:[{path:'../escape',before:null,after:sha('x')}]}),/path/i);
 }finally{rmSync(dir,{recursive:true,force:true});}
});
