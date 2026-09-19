import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { once } from "node:events";
import { createServer } from "node:http";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createInterface } from "node:readline";

// All credentials, output images and requests are confined to this fixture.
export async function verifyMcpProtocol(binary) {
  const temporary = await mkdtemp(path.join(tmpdir(), "711ev-mcp-protocol-"));
  const png = Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Wl6xAAAAABJRU5ErkJggg==", "base64");
  const requests = [];
  const server = createServer(async (request, response) => {
    const chunks = [];
    for await (const chunk of request) chunks.push(chunk);
    requests.push({ url: request.url, authorization: request.headers.authorization, body: Buffer.concat(chunks).toString() });
    response.setHeader("Content-Type", "application/json");
    response.end(JSON.stringify({ created: 1, model: "openai-image", data: [{ b64_json: png.toString("base64") }] }));
  });
  let child;
  try {
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const home = path.join(temporary, "codex");
    const runtime = path.join(temporary, "mcp", "profile-a");
    const output = path.join(temporary, "pictures");
    await mkdir(home, { recursive: true });
    await mkdir(runtime, { recursive: true });
    const mcp = path.join(runtime, process.platform === "win32" ? "image-mcp.exe" : "image-mcp");
    await writeFile(mcp, binary, { mode: 0o755 });
    await writeFile(path.join(runtime, "settings.json"), JSON.stringify({ schema_version: 1, codex_home: home }));
    await writeFile(path.join(home, "config.toml"), `model_provider = "local-test"\n[model_providers.local-test]\nbase_url = "http://127.0.0.1:${server.address().port}/v1"\nexperimental_bearer_token = "test-only-key"\n`);
    await writeFile(path.join(home, "auth.json"), JSON.stringify({ OPENAI_API_KEY: "test-only-key" }));
    child = spawn(mcp, [], { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
    const pending = new Map();
    let requestId = 0;
    let stderr = "";
    child.stderr.on("data", (data) => { stderr += data.toString(); });
    const lines = createInterface({ input: child.stdout });
    lines.on("line", (line) => {
      try {
        const message = JSON.parse(line);
        const task = pending.get(message.id);
        if (message.error) task?.reject(new Error(JSON.stringify(message.error)));
        else task?.resolve(message.result);
      } catch (error) {
        for (const task of pending.values()) task.reject(error);
      }
    });
    child.on("error", (error) => { for (const task of pending.values()) task.reject(error); });
    child.on("exit", (code) => {
      for (const task of pending.values()) task.reject(new Error(`MCP exited (${code}): ${stderr}`));
    });
    async function call(method, params = {}) {
      const id = ++requestId;
      let timeout;
      try {
        return await new Promise((resolve, reject) => {
          pending.set(id, { resolve, reject });
          timeout = setTimeout(() => reject(new Error(`MCP timed out: ${method}`)), 15000);
          child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
        });
      } finally {
        clearTimeout(timeout);
        pending.delete(id);
      }
    }
    assert.equal((await call("initialize")).serverInfo.name, "generate_image");
    assert.deepEqual((await call("tools/list")).tools.map((tool) => tool.name), ["generate_image", "edit_image"]);
    const generated = await call("tools/call", { name: "generate_image", arguments: { prompt: "local protocol test", output_dir: output } });
    assert.notEqual(generated.isError, true);
    assert.ok(generated.content.some((item) => item.type === "image"));
    const [generatedPath] = generated.structuredContent.outputPaths;
    assert.deepEqual(await readFile(generatedPath), png);
    const edited = await call("tools/call", { name: "edit_image", arguments: { prompt: "local edit test", image_path: generatedPath, output_dir: output } });
    assert.notEqual(edited.isError, true);
    assert.ok(edited.content.some((item) => item.type === "image"));
    assert.deepEqual(requests.map((request) => request.url), ["/v1/images/generations", "/v1/images/edits"]);
    assert.ok(requests.every((request) => request.authorization === "Bearer test-only-key"));
    assert.equal(JSON.parse(requests[0].body).model, "openai-image");
    assert.ok(requests[1].body.includes("openai-image"));
    await writeFile(path.join(home, "auth.json"), JSON.stringify({ OPENAI_API_KEY: "mismatched-test-key" }));
    const denied = await call("tools/call", { name: "generate_image", arguments: { prompt: "must not call server", output_dir: output } });
    assert.equal(denied.isError, true);
    assert.equal(requests.length, 2);
    console.log("MCP 协议验证通过：stdio 握手、生图/改图本地模拟、凭据不匹配拒绝请求。");
  } finally {
    if (child && child.exitCode === null && child.pid) {
      const exited = once(child, "exit");
      child.kill();
      await exited;
    }
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
    await rm(temporary, { recursive: true, force: true });
  }
}
