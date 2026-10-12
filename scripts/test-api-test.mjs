import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createServer } from 'node:http';
import { once } from 'node:events';
import ts from 'typescript';

const source = readFileSync(new URL('../src/utils/apiTest.ts', import.meta.url), 'utf8');
const compiled = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 } }).outputText;
const { gatewayRoot, testPayload, testEndpoint, apiTestRequest, inspectTestResponse, modelsFromResult } = await import(`data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`);
const key = 'sk-test-fixture-not-a-real-key-1234';
const calls = [];
const server = createServer(async (request, response) => {
    let body = '';
    for await (const chunk of request) body += chunk;
    calls.push({ url: request.url, headers: request.headers, body });
    response.setHeader('X-Request-ID', 'bill-fixture-trace');
    response.setHeader('X-Mapped-Model', 'selected-account-medium');
    response.setHeader('X-Account-Email', 'fixture@example.invalid');
    if (request.url.endsWith('/models')) {
        response.setHeader('Content-Type', 'application/json');
        response.end(JSON.stringify({ data: [{ id: 'model-b' }, { id: 'model-a' }, { id: 'model-a' }] }));
    } else if (request.url.includes('streamGenerateContent')) {
        response.setHeader('Content-Type', 'text/event-stream');
        const bytes = Buffer.from('data: {"candidates":[{"content":{"parts":[{"text":"你✅"}]}}]}\n\n');
        for (const byte of bytes) response.write(Buffer.from([byte]));
        response.end();
    } else if (request.url === '/error') {
        response.writeHead(429, { 'Content-Type': 'application/json' });
        response.end(JSON.stringify({ error: { message: 'quota exhausted' }, echo: key }));
    } else {
        response.setHeader('Content-Type', 'application/json');
        response.end(JSON.stringify({ choices: [{ message: { content: 'OK' } }] }));
    }
});
server.listen(0, '127.0.0.1');
await once(server, 'listening');
const root = `http://127.0.0.1:${server.address().port}`;
const signal = new AbortController().signal;
let checks = 0;
try {
    assert.equal(gatewayRoot(`${root}/v1/`), root);
    assert.equal(gatewayRoot(`${root}/prefix/v1beta`), `${root}/prefix`);
    assert.throws(() => gatewayRoot('https://user:secret@example.invalid/v1'));
    checks++;
    for (const auth of ['bearer', 'x-api-key', 'x-goog-api-key']) {
        const result = await apiTestRequest({ url: `${root}/v1/models`, key, auth, signal });
        const actual = calls.at(-1).headers;
        assert.equal(actual[auth === 'bearer' ? 'authorization' : auth], auth === 'bearer' ? `Bearer ${key}` : key);
        for (const other of ['authorization', 'x-api-key', 'x-goog-api-key'].filter(name => name !== (auth === 'bearer' ? 'authorization' : auth))) assert.equal(actual[other], undefined);
        assert.deepEqual(modelsFromResult(result), ['model-a', 'model-b']);
        checks++;
    }
    for (const protocol of ['chat', 'responses', 'claude', 'gemini']) {
        const payload = testPayload(protocol, 'model-a', 'test message', 64, false);
        const result = await apiTestRequest({ url: testEndpoint(root, protocol, 'model-a', false), key, auth: 'bearer', signal, payload, model: 'model-a' });
        assert.equal(result.status, 200);
        assert.equal(result.mappedModel, 'selected-account-medium');
        assert.equal(result.output, 'OK');
        const sent = JSON.parse(calls.at(-1).body);
        assert.equal(protocol === 'gemini' ? sent.contents[0].parts[0].text : protocol === 'responses' ? sent.input : sent.messages[0].content, 'test message');
        if (protocol === 'claude') assert.equal(calls.at(-1).headers['anthropic-version'], '2023-06-01');
        checks++;
    }
    const rawBody = '{\n  "model": "model-a", "large_id": 9007199254740993\n}';
    await apiTestRequest({ url: `${root}/v1/chat/completions`, key, auth: 'bearer', signal, payload: JSON.parse(rawBody), rawBody });
    assert.equal(calls.at(-1).body, rawBody, 'custom JSON must be transmitted byte-for-byte');
    checks++;
    const error = await apiTestRequest({ url: `${root}/error`, key, auth: 'bearer', signal });
    assert.equal(error.status, 429);
    assert.equal(error.error, 'quota exhausted');
    assert.ok(!error.body.includes(key), 'results must redact echoed keys');
    assert.deepEqual(modelsFromResult(error), []);
    checks++;
    const streamed = await apiTestRequest({ url: testEndpoint(root, 'gemini', 'model-a', true), key, auth: 'x-goog-api-key', signal, payload: testPayload('gemini', 'model-a', 'test', 64, true) });
    assert.equal(streamed.output, '你✅');
    const sseError = inspectTestResponse('event: error\ndata: {"error":{"message":"stream failed"}}\n\n');
    assert.equal(sseError.error, 'stream failed', 'HTTP 200 streams may still fail');
    const responses = inspectTestResponse('data: {"type":"response.output_text.delta","delta":"OK"}\n\ndata: {"type":"response.completed","response":{"output":[{"content":[{"text":"OK"}]}]}}\n\n');
    assert.equal(responses.output, 'OK', 'do not duplicate completed Responses output');
    checks++;
    console.log(`API test request/response regressions: ${checks} checks passed`);
} finally {
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
}
