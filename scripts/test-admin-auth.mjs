import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

const source = readFileSync(new URL('../src/utils/request.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 },
}).outputText;

async function rejectedRequest(initialKey, currentKey) {
  let key = initialKey;
  let respond;
  const events = [];
  const exports = {};
  const context = vm.createContext({
    exports,
    module: { exports },
    console: { error() {} },
    window: { dispatchEvent: event => events.push(event.type) },
    sessionStorage: { getItem: () => key },
    CustomEvent: class { constructor(type) { this.type = type; } },
    fetch: () => new Promise(resolve => { respond = resolve; }),
  });
  new vm.Script(compiled).runInContext(context);
  const result = exports.request('load_config').catch(error => error);
  key = currentKey;
  respond({ ok: false, status: 401, json: async () => ({ error: 'Unauthorized' }) });
  assert.equal(await result, 'Unauthorized');
  return events;
}

assert.deepEqual(await rejectedRequest(null, 'new-login'), [], 'late anonymous 401 must preserve login');
assert.deepEqual(await rejectedRequest('old-key', 'new-key'), [], 'late old-key 401 must preserve replacement');
assert.deepEqual(await rejectedRequest('current-key', 'current-key'), ['abv-unauthorized'], 'invalid current key must still require login');
console.log('Admin authentication response ordering: 3 checks passed');
