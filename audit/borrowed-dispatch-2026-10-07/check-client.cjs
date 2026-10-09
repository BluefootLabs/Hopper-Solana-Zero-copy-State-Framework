const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const ts = require('D:/tmp/Hopper-Website/node_modules/typescript');
const root = path.join(__dirname, 'client');
const sdk = require(path.join(root, 'node_modules/@solana/web3.js'));
const source = fs.readFileSync(path.join(root, 'instructions.ts'), 'utf8');
const compiled = ts.transpileModule(source, {compilerOptions: {target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.CommonJS}, reportDiagnostics: true});
assert.equal((compiled.diagnostics || []).filter(d => d.category === ts.DiagnosticCategory.Error).length, 0);
fs.writeFileSync(path.join(root, 'instructions.cjs'), compiled.outputText);
const {createSubmitInstruction,createEmptyInstruction} = require(path.join(root, 'instructions.cjs'));
const authority = new sdk.PublicKey(new Uint8Array(32).fill(8));
const programId = new sdk.PublicKey(new Uint8Array(32).fill(9));
const order = Uint8Array.from([1,1,0,255,42,0,0,0,0,0,0,0]);
const instruction = createSubmitInstruction({order,nonce:513},{authority},programId);
assert.deepEqual(Array.from(instruction.data), [0,...order,1,2]);
assert(instruction.programId.equals(programId));
assert(instruction.keys[0].pubkey.equals(authority));
assert.deepEqual(instruction.keys.map(k=>[k.isSigner,k.isWritable]),[[true,false]]);
const rejected = [0,1,11,13,14,65535];
for (const length of rejected) {
  assert.throws(()=>createSubmitInstruction({order:new Uint8Array(length),nonce:513},{authority},programId),/order must encode exactly 12 bytes/);
}
const emptyInstruction = createEmptyInstruction({empty:new Uint8Array(0),nonce:513},{authority},programId);
assert.deepEqual(Array.from(emptyInstruction.data),[1,1,2]);
for (const length of [1,2]) assert.throws(()=>createEmptyInstruction({empty:new Uint8Array(length),nonce:513},{authority},programId),/empty must encode exactly 0 bytes/);
const result = {sdk:'@solana/web3.js@1.98.4',expectedWireHex:Buffer.from(instruction.data).toString('hex'),exactPayloadPassed:true,rejectedLengths:rejected,zeroWidthPayloadPassed:true,zeroWidthRejectedLengths:[1,2],accountPrivilegesPassed:true};
fs.writeFileSync(path.join(__dirname,'client-check.json'),JSON.stringify(result,null,2)+'\n');
console.log(JSON.stringify(result));
