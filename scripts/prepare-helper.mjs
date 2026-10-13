// Apply the canonical unpublished helper delta to the pinned upstream revision.
// Fail closed on any human edits or partial setup; never reset the submodule.
import {createHash} from 'node:crypto';
import {readFileSync,existsSync} from 'node:fs';
import {dirname,resolve,relative,isAbsolute,sep} from 'node:path';
import {fileURLToPath} from 'node:url';
import {execFileSync} from 'node:child_process';
const sha=data=>createHash('sha256').update(data.toString('utf8').replace(/\r\n/g,'\n')).digest('hex');
export function overlayState(root,manifest){
 const states=manifest.files.map(file=>{
  const path=resolve(root,file.path);
  const rel=relative(resolve(root),path);
  if(isAbsolute(rel)||rel==='..'||rel.startsWith(`..${sep}`))throw new Error('Invalid overlay path');
  const hash=existsSync(path)?sha(readFileSync(path)):null;
  if(hash===file.after)return 'applied';
  if(hash===file.before)return 'base';
  throw new Error(`Preserve local edits: helper overlay differs at ${file.path}`);
 });
 if(states.every(s=>s==='applied'))return 'applied';
 if(states.every(s=>s==='base'))return 'base';
 throw new Error('Mixed helper overlay state; preserve files and inspect manually');
}
export function prepare(root){
 const repo=resolve(root,'vendor/aevra');
 const manifest=JSON.parse(readFileSync(resolve(root,'patches/helper-parity.json'),'utf8'));
 const revision=execFileSync('git',['rev-parse','HEAD'],{cwd:repo,encoding:'utf8'}).trim();
 if(revision!==manifest.baseRevision)throw new Error('Pinned helper revision changed; regenerate and review the canonical overlay');
 const state=overlayState(repo,manifest);
 if(state==='base'){
  const patch=resolve(root,'patches/helper-parity.patch');
  execFileSync('git',['apply','--check','--ignore-space-change',patch],{cwd:repo,stdio:'inherit'});
  execFileSync('git',['apply','--ignore-space-change',patch],{cwd:repo,stdio:'inherit'});
 }
 if(overlayState(repo,manifest)!=='applied')throw new Error('Helper overlay verification failed');
 return state;
}
if(process.argv[1]&&resolve(process.argv[1])===fileURLToPath(import.meta.url)){
 const root=resolve(dirname(fileURLToPath(import.meta.url)),'..');
 console.log(`Canonical helper overlay: ${prepare(root)}`);
}
