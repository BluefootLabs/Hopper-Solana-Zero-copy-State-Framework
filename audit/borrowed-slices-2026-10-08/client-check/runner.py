#!/usr/bin/env python3
"""Compile and execute generated batch clients against independent wire bytes.

Requires a local TypeScript installation and @solana/web3.js package. Output
must be a fresh ignored target/ directory. Rust uses the real SDK instruction
and pubkey crates behind a local facade matching the generator's imports.
"""
from pathlib import Path
import argparse
import json
import os
import subprocess

ROOT = Path(__file__).resolve().parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--typescript', required=True, type=Path)
    parser.add_argument('--web3', required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve()
    if not out.is_relative_to(ROOT / 'target') or out.exists():
        raise RuntimeError('use a fresh ignored target/ directory')
    out.mkdir(parents=True)
    env = os.environ.copy()
    env['CARGO_TARGET_DIR'] = str(ROOT / 'target/borrowed-slices-client-host-check')
    env['HOPPER_SLICE_CLIENT_OUT'] = str(out)
    commands = []

    def run(command):
        log = out / f'check-{len(commands)}.log'
        with log.open('wb') as stream:
            code = subprocess.run(command, cwd=ROOT, env=env, stdout=stream, stderr=subprocess.STDOUT).returncode
        commands.append({'command': command, 'exitCode': code, 'log': log.name})
        if code:
            raise RuntimeError(f'client validation failed; inspect {log}')

    run(['cargo', 'test', '-p', 'hopper-lang', '--test', 'borrowed_slices', '--features', 'proc-macros', '--locked', '--offline', '-j', '1'])
    (out / 'Cargo.toml').write_text('''[package]
name = "hopper-borrowed-slice-client-check"
version = "0.0.0"
edition = "2021"
[workspace]
[dependencies]
solana-instruction = "=3.4.0"
solana-pubkey = "=4.2.1"
[lib]
path = "check.rs"
''', encoding='utf-8')
    (out / 'check.rs').write_text(r'''
extern crate self as solana_program;
pub mod instruction { pub use solana_instruction::*; }
pub mod pubkey { pub use solana_pubkey::*; }
#[path = "client.rs"] mod client;
#[cfg(test)] mod tests {
    use super::{client::*, pubkey::Pubkey};
    #[test] fn wire_contract_and_bounds() {
        let mut first = [0; 10]; first[0..2].copy_from_slice(&[1, 1]); first[2..].copy_from_slice(&42u64.to_le_bytes());
        let mut second = [0; 10]; second[0..2].copy_from_slice(&[1, 7]); second[2..].copy_from_slice(&99u64.to_le_bytes());
        let args = SubmitArgs { orders: vec![first, second], nonce: 513 };
        let mut expected = vec![0, 2, 0]; expected.extend_from_slice(&first); expected.extend_from_slice(&second); expected.extend_from_slice(&[1, 2]);
        assert_eq!(encode_submit_data(&args).unwrap(), expected);
        let decoded = decode_submit_args(&expected).unwrap(); assert_eq!(decoded.orders, args.orders); assert_eq!(decoded.nonce, 513);
        for end in 0..expected.len() { assert!(decode_submit_args(&expected[..end]).is_err()); }
        let mut extra = expected.clone(); extra.push(0); assert!(decode_submit_args(&extra).is_err());
        let mut excessive = args.clone(); excessive.orders.resize(5, first); assert!(encode_submit_data(&excessive).is_err());
        excessive.orders.clear(); assert_eq!(encode_submit_data(&excessive).unwrap(), [0,0,0,1,2]);
        excessive.orders.resize(4, first); assert_eq!(decode_submit_args(&encode_submit_data(&excessive).unwrap()).unwrap().orders.len(), 4);
        for count in [3u16, 5, 65535] { let mut bad = expected.clone(); bad[1..3].copy_from_slice(&count.to_le_bytes()); assert!(decode_submit_args(&bad).is_err()); }
        let program = Pubkey::new_from_array([9;32]); let authority = Pubkey::new_from_array([8;32]);
        let ix = submit_ix(&program, &SubmitAccounts { authority }, &args).unwrap();
        assert_eq!(ix.data, expected); assert_eq!(ix.program_id, program); assert_eq!(ix.accounts[0].pubkey, authority); assert!(ix.accounts[0].is_signer); assert!(!ix.accounts[0].is_writable);
        let mixed = BytesArgs { bytes: vec![[42], [99]], note: "ok".into() };
        assert_eq!(encode_bytes_data(&mixed).unwrap(), [1,2,0,42,99,2,0,111,107]);
        assert_eq!(decode_bytes_args(&[1,2,0,42,99,2,0,111,107]).unwrap().note, "ok");
        assert!(decode_bytes_args(&[1,0,0,1,0,255]).is_err());
        assert!(encode_bytes_data(&BytesArgs { bytes: vec![], note: "Ã©".repeat(9) }).is_err());
    }
}
''', encoding='utf-8')
    env['CARGO_TARGET_DIR'] = str(ROOT / 'target/borrowed-slices-client-host-check')
    run(['cargo', 'test', '--manifest-path', str(out / 'Cargo.toml'), '--offline', '-j', '1'])
    (out / 'check.cjs').write_text(r'''
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const {createRequire} = require('node:module');
const ts = require(process.argv[2]), sdkRequire = createRequire(path.join(process.argv[3], 'package.json'));
const sdk = sdkRequire(process.argv[3]);
const source = fs.readFileSync(path.join(__dirname, 'instructions.ts'), 'utf8');
const compiled = ts.transpileModule(source, {compilerOptions:{target:ts.ScriptTarget.ES2022,module:ts.ModuleKind.CommonJS},reportDiagnostics:true});
assert.equal(compiled.diagnostics.filter(d=>d.category === ts.DiagnosticCategory.Error).length,0);
const m = {exports:{}}; new Function('require','module','exports',compiled.outputText)(sdkRequire,m,m.exports);
const {createSubmitInstruction, createBytesInstruction} = m.exports;
const authority = new sdk.PublicKey(new Uint8Array(32).fill(8)), program = new sdk.PublicKey(new Uint8Array(32).fill(9));
const first = Uint8Array.from([1,1,42,0,0,0,0,0,0,0]), second = Uint8Array.from([1,7,99,0,0,0,0,0,0,0]);
const ix = createSubmitInstruction({orders:[first,second],nonce:513},{authority},program);
assert.deepEqual([...ix.data],[0,2,0,...first,...second,1,2]);
assert(ix.programId.equals(program)); assert(ix.keys[0].pubkey.equals(authority)); assert.deepEqual(ix.keys.map(k=>[k.isSigner,k.isWritable]),[[true,false]]);
for (const count of [0,1,4]) assert.equal(createSubmitInstruction({orders:Array(count).fill(first),nonce:513},{authority},program).data.length, 5+count*10);
assert.throws(()=>createSubmitInstruction({orders:Array(5).fill(first),nonce:513},{authority},program),/capacity/);
for (const size of [0,9,11]) assert.throws(()=>createSubmitInstruction({orders:[new Uint8Array(size)],nonce:513},{authority},program),/size/);
assert.deepEqual([...createBytesInstruction({bytes:Uint8Array.from([42,99]),note:'ok'},{authority},program).data],[1,2,0,42,99,2,0,111,107]);
assert.throws(()=>createBytesInstruction({bytes:new Uint8Array(9),note:''},{authority},program),/capacity/);
assert.throws(()=>createBytesInstruction({bytes:new Uint8Array(0),note:'Ã©'.repeat(9)},{authority},program),/UTF-8/);
console.log('Generated TypeScript client wire, capacity, element width, UTF-8, and privilege checks passed.');
''', encoding='utf-8')
    run(['node', str(out / 'check.cjs'), str(args.typescript.resolve()), str(args.web3.resolve())])
    (out / 'summary.json').write_text(json.dumps({'allPassed': True, 'checks': commands}, indent=2)+'\n', encoding='utf-8')
    print('Generated Rust and TypeScript clients compiled and passed wire-contract checks.')


if __name__ == '__main__':
    main()
