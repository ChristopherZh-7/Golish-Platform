import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import crypto from "node:crypto";
import http from "node:http";
import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import {
  boundedHardTimeoutMs,
  captureDirectoryFor,
  classifyCollectionCompletion,
  fetchExactOrigin,
  isDangerousNavigationUrl,
  isExactOriginUrl,
  recoveryKindBlocksClosure,
} from "./browser_collect_js_api.mjs";

test("whole-helper deadline defaults and explicit zero remain bounded", () => {
  assert.equal(boundedHardTimeoutMs(undefined), 120_000);
  assert.equal(boundedHardTimeoutMs("0"), 120_000);
  assert.equal(boundedHardTimeoutMs("5000"), 10_000);
  assert.equal(boundedHardTimeoutMs("999999"), 600_000);
});

test("API response-body capture is diagnostic while JS/page failures block closure", () => {
  assert.equal(recoveryKindBlocksClosure("api_body"), false);
  assert.equal(recoveryKindBlocksClosure("recursive_fetch"), true);
  assert.equal(recoveryKindBlocksClosure("script_body"), true);
  assert.equal(recoveryKindBlocksClosure("navigation"), true);
  assert.equal(recoveryKindBlocksClosure("unknown"), false);
  assert.deepEqual(
    classifyCollectionCompletion({
      navigation_attempts: 1,
      successful_pages: 1,
      api_body_capture_errors: 1,
    }),
    {
      status: "ok",
      completion_state: "complete",
      closure_complete: true,
      reasons: [],
    },
  );
});

function listen(server) {
  return new Promise((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve(server.address()));
  });
}

function close(server) {
  return new Promise((resolve, reject) => {
    server.close((error) => (error ? reject(error) : resolve()));
  });
}

function runCollector(args) {
  const script = fileURLToPath(new URL("./browser_collect_js_api.mjs", import.meta.url));
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, [script, ...args], {
      cwd: path.dirname(path.dirname(script)),
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8");
    child.stderr.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      stdout += chunk;
    });
    child.stderr.on("data", (chunk) => {
      stderr += chunk;
    });
    child.once("error", reject);
    child.once("close", (code) => {
      if (code !== 0) {
        reject(new Error(`collector exited ${code}: ${stderr}`));
        return;
      }
      try {
        resolve(JSON.parse(stdout));
      } catch (error) {
        reject(new Error(`invalid collector JSON: ${error.message}\n${stdout}\n${stderr}`));
      }
    });
  });
}

test("capture directory keeps HTTP and HTTPS on the same port separate", () => {
  const workspace = path.join(path.sep, "tmp", "golish-browser-test");
  assert.equal(
    captureDirectoryFor(workspace, new URL("http://app.example:8443/"), "js"),
    path.join(workspace, ".golish", "captures", "app.example", "8443", "http", "js"),
  );
  assert.equal(
    captureDirectoryFor(workspace, new URL("https://app.example:8443/"), "js"),
    path.join(workspace, ".golish", "captures", "app.example", "8443", "https", "js"),
  );
});

test("exact origin identity rejects scheme port and sibling changes", () => {
  const origin = new URL("https://app.example:8443/").origin;
  assert.equal(isExactOriginUrl("https://app.example:8443/a.js", origin), true);
  assert.equal(isExactOriginUrl("http://app.example:8443/a.js", origin), false);
  assert.equal(isExactOriginUrl("https://app.example:443/a.js", origin), false);
  assert.equal(isExactOriginUrl("https://api.app.example:8443/a.js", origin), false);
});

test("dangerous navigation guard rejects double-encoded state-changing routes", () => {
  assert.equal(
    isDangerousNavigationUrl("https://app.example/%256cogout?next=%252fhome"),
    true,
  );
  assert.equal(
    isDangerousNavigationUrl(
      "https://app.example/?next=%256c%256f%2567%256f%2575%2574&x=%ZZ",
    ),
    true,
  );
  assert.equal(
    isDangerousNavigationUrl(
      "https://app.example/%25256c%25256f%252567%25256f%252575%252574",
    ),
    true,
  );
  for (const route of ["/actuator/shutdown", "/actuator/restart", "/refund", "/activate"]) {
    assert.equal(isDangerousNavigationUrl(`https://app.example${route}`), true);
  }
  assert.equal(isDangerousNavigationUrl("https://app.example/dashboard"), false);
});

test("explicit fetch follows only same-origin redirects", async () => {
  let foreignHits = 0;
  let dangerousHits = 0;
  const foreign = http.createServer((_request, response) => {
    foreignHits += 1;
    response.writeHead(200, { "content-type": "text/javascript" });
    response.end("export default 1");
  });
  const foreignAddress = await listen(foreign);

  const target = http.createServer((request, response) => {
    if (request.url === "/same") {
      response.writeHead(302, { location: "/ok" });
      response.end();
      return;
    }
    if (request.url === "/cross") {
      response.writeHead(302, {
        location: `http://127.0.0.1:${foreignAddress.port}/foreign.js`,
      });
      response.end();
      return;
    }
    if (request.url === "/dangerous-redirect") {
      response.writeHead(302, { location: "/%256cogout?confirm=true" });
      response.end();
      return;
    }
    if (
      request.url?.startsWith("/%256cogout") ||
      request.url?.startsWith("/%25256c%25256f%252567%25256f%252575%252574")
    ) {
      dangerousHits += 1;
      response.writeHead(500);
      response.end("dangerous redirect target must never be requested");
      return;
    }
    response.writeHead(200, { "content-type": "text/javascript" });
    response.end("export default 1");
  });
  const targetAddress = await listen(target);
  const targetOrigin = `http://127.0.0.1:${targetAddress.port}`;

  try {
    const sameOriginResponse = await fetchExactOrigin(
      `${targetOrigin}/same`,
      targetOrigin,
    );
    assert.equal(sameOriginResponse.status, 200);
    await assert.rejects(
      fetchExactOrigin(`${targetOrigin}/cross`, targetOrigin),
      /exact-origin redirect blocked/,
    );
    await assert.rejects(
      fetchExactOrigin(`${targetOrigin}/dangerous-redirect`, targetOrigin),
      /read-only redirect blocked/,
    );
    await assert.rejects(
      fetchExactOrigin(`${targetOrigin}/%256cogout?confirm=true`, targetOrigin),
      /read-only fetch blocked/,
    );
    await assert.rejects(
      fetchExactOrigin(
        `${targetOrigin}/?next=%256c%256f%2567%256f%2575%2574&x=%ZZ`,
        targetOrigin,
      ),
      /read-only fetch blocked/,
    );
    await assert.rejects(
      fetchExactOrigin(
        `${targetOrigin}/%25256c%25256f%252567%25256f%252575%252574`,
        targetOrigin,
      ),
      /read-only fetch blocked/,
    );
    assert.equal(foreignHits, 0, "the foreign redirect target must never be requested");
    assert.equal(dangerousHits, 0, "the dangerous redirect target must never be requested");
  } finally {
    await close(target);
    await close(foreign);
  }
});

test("authorized-origin terminal redirect is a safe empty observation without requesting foreign target", async () => {
  let foreignHits = 0;
  let targetHits = 0;
  const foreign = http.createServer((_request, response) => {
    foreignHits += 1;
    response.writeHead(200, { "content-type": "text/html" });
    response.end("foreign");
  });
  const foreignAddress = await listen(foreign);
  const target = http.createServer((_request, response) => {
    targetHits += 1;
    response.writeHead(302, {
      location: `http://127.0.0.1:${foreignAddress.port}/foreign`,
    });
    response.end();
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "golish-browser-scope-"));

  try {
    const result = await runCollector([
      "--url",
      `http://127.0.0.1:${targetAddress.port}/`,
      "--workspace",
      workspace,
      "--max-pages",
      "1",
      "--max-actions",
      "0",
      "--max-recursive-scripts",
      "10",
      "--hard-timeout-ms",
      "20000",
      "--ai-assist",
      "false",
    ]);
    assert.equal(targetHits, 1, "navigation preflight must not request the source URL twice");
    assert.equal(foreignHits, 0, "the redirected origin must be blocked before request");
    assert.equal(result.status, "ok");
    assert.equal(result.completion_state, "complete");
    assert.equal(result.closure_complete, true);
    assert.equal(result.scope_violations, 0);
    assert.equal(result.scope_exclusions, 1);
    assert.deepEqual(result.terminal_cross_origin_redirects, [
      {
        from: `http://127.0.0.1:${targetAddress.port}/`,
        to: `http://127.0.0.1:${foreignAddress.port}/foreign`,
        status: 302,
      },
    ]);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
    await close(foreign);
  }
});

test("same-origin browser redirects request every URL exactly once", async () => {
  let rootHits = 0;
  let landingHits = 0;
  const target = http.createServer((request, response) => {
    if (request.url === "/") {
      rootHits += 1;
      response.writeHead(302, { location: "/landing" });
      response.end();
      return;
    }
    if (request.url === "/landing") {
      landingHits += 1;
      response.writeHead(200, { "content-type": "text/html" });
      response.end("<!doctype html><title>landing</title>");
      return;
    }
    response.writeHead(404);
    response.end();
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "golish-browser-redirect-once-"));

  try {
    const result = await runCollector([
      "--url",
      `http://127.0.0.1:${targetAddress.port}/`,
      "--workspace",
      workspace,
      "--max-pages",
      "1",
      "--max-actions",
      "0",
      "--max-recursive-scripts",
      "10",
      "--hard-timeout-ms",
      "20000",
      "--ai-assist",
      "false",
    ]);

    assert.equal(result.completion_state, "complete");
    assert.equal(rootHits, 1);
    assert.equal(landingHits, 1);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("browser blocks every foreign subresource websocket and service worker before request", async () => {
  let foreignRequestHits = 0;
  let foreignUpgradeHits = 0;
  let serviceWorkerHits = 0;
  const foreign = http.createServer((_request, response) => {
    foreignRequestHits += 1;
    response.writeHead(200, { "content-type": "application/octet-stream" });
    response.end("foreign");
  });
  foreign.on("upgrade", (_request, socket) => {
    foreignUpgradeHits += 1;
    socket.destroy();
  });
  const foreignAddress = await listen(foreign);
  const foreignHttp = `http://127.0.0.1:${foreignAddress.port}`;
  const foreignWs = `ws://127.0.0.1:${foreignAddress.port}`;

  const target = http.createServer((request, response) => {
    if (request.url === "/sw.js") {
      serviceWorkerHits += 1;
      response.writeHead(200, { "content-type": "text/javascript" });
      response.end("self.addEventListener('fetch', () => {});");
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    response.end(`<!doctype html>
      <style>
        @font-face { font-family: Foreign; src: url('${foreignHttp}/font.woff2'); }
        body { font-family: Foreign; background-image: url('${foreignHttp}/bg.png'); }
      </style>
      <script src="${foreignHttp}/foreign.js"></script>
      <img src="${foreignHttp}/image.png">
      <script>
        fetch('${foreignHttp}/fetch');
        const xhr = new XMLHttpRequest();
        xhr.open('GET', '${foreignHttp}/xhr');
        xhr.send();
        new WebSocket('${foreignWs}/socket');
        navigator.serviceWorker?.register('/sw.js').catch(() => {});
      </script>`);
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "golish-browser-subresource-"));

  try {
    const result = await runCollector([
      "--url",
      `http://127.0.0.1:${targetAddress.port}/`,
      "--workspace",
      workspace,
      "--max-pages",
      "1",
      "--max-actions",
      "0",
      "--max-recursive-scripts",
      "10",
      "--hard-timeout-ms",
      "20000",
      "--ai-assist",
      "false",
    ]);

    assert.equal(foreignRequestHits, 0, "foreign subresources must be aborted before request");
    assert.equal(foreignUpgradeHits, 0, "foreign websocket must never reach upgrade");
    assert.equal(serviceWorkerHits, 0, "service worker scripts must not be requested");
    assert.equal(result.status, "ok");
    assert.equal(result.completion_state, "complete");
    assert.equal(result.closure_complete, true);
    assert.equal(result.scope_violations, 0);
    assert.ok(result.scope_exclusions >= 5);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
    await close(foreign);
  }
});

test("all navigation failures are an error instead of checked-empty", () => {
  const result = classifyCollectionCompletion({
    navigation_attempts: 2,
    successful_pages: 0,
    navigation_errors: 2,
  });
  assert.equal(result.status, "error");
  assert.equal(result.completion_state, "error");
  assert.equal(result.closure_complete, false);
});

test("complete zero-JS collection writes a closure-proving manifest", async () => {
  const target = http.createServer((_request, response) => {
    response.writeHead(200, { "content-type": "text/html" });
    response.end("<!doctype html><title>no scripts</title>");
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "golish-browser-manifest-"));

  try {
    const result = await runCollector([
      "--url",
      `http://127.0.0.1:${targetAddress.port}/`,
      "--workspace",
      workspace,
      "--max-pages",
      "1",
      "--max-actions",
      "0",
      "--max-recursive-scripts",
      "10",
      "--hard-timeout-ms",
      "20000",
      "--ai-assist",
      "false",
      "--run-id",
      "run-current",
      "--session-id",
      "session-current",
      "--operation-id",
      "00000000-0000-0000-0000-000000000010",
      "--stage-started-at",
      "2020-01-01T00:00:00Z",
    ]);
    assert.equal(result.completion_state, "complete");
    const manifest = JSON.parse(
      await fs.readFile(path.join(workspace, result.script_manifest), "utf8"),
    );
    assert.equal(manifest.closure_complete, true);
    assert.equal(manifest.completion_state, "complete");
    assert.equal(manifest.producer_run_id, "run-current");
    assert.equal(manifest.producer_session_id, "session-current");
    assert.equal(
      manifest.producer_operation_id,
      "00000000-0000-0000-0000-000000000010",
    );
    assert.equal(manifest.producer_stage_started_at, "2020-01-01T00:00:00Z");
    assert.match(manifest.captured_at, /^\d{4}-\d{2}-\d{2}T/);
    assert.deepEqual(manifest.scripts, []);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("a script removed between runs is not carried forward from the old manifest", async () => {
  let includeScript = true;
  const target = http.createServer((request, response) => {
    if (request.url === "/app.js") {
      response.writeHead(200, { "content-type": "text/javascript" });
      response.end("fetch('/api/current')");
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    response.end(
      includeScript
        ? '<!doctype html><script src="/app.js"></script>'
        : "<!doctype html><title>script removed</title>",
    );
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "golish-browser-freshness-"));
  const args = [
    "--url",
    `http://127.0.0.1:${targetAddress.port}/`,
    "--workspace",
    workspace,
    "--max-pages",
    "1",
    "--max-actions",
    "0",
    "--max-recursive-scripts",
    "10",
    "--hard-timeout-ms",
    "20000",
    "--ai-assist",
    "false",
  ];

  try {
    const first = await runCollector(args);
    assert.equal(first.completion_state, "complete");
    assert.equal(first.scripts_observed, 1);
    assert.equal(first.scripts.length, 1);
    const historicalScriptPath = path.join(workspace, first.scripts[0].path);
    await fs.access(historicalScriptPath);

    includeScript = false;
    const second = await runCollector(args);
    assert.equal(second.completion_state, "complete");
    assert.equal(second.scripts_cached_preloaded, 0);
    assert.equal(second.scripts_observed, 0);
    assert.deepEqual(second.scripts, []);
    await fs.access(historicalScriptPath);

    const manifest = JSON.parse(
      await fs.readFile(path.join(workspace, second.script_manifest), "utf8"),
    );
    assert.equal(manifest.closure_complete, true);
    assert.deepEqual(manifest.scripts, []);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("same-run page checkpoints finish a finite crawl while a new run starts fresh", async () => {
  const target = http.createServer((request, response) => {
    const scripts = {
      "/root.js": "window.rootLoaded = true;",
      "/two.js": "window.pageTwoLoaded = true;",
      "/three.js": "window.pageThreeLoaded = true;",
    };
    if (Object.hasOwn(scripts, request.url)) {
      response.writeHead(200, { "content-type": "text/javascript" });
      response.end(scripts[request.url]);
      return;
    }
    if (request.url === "/api/root") {
      response.writeHead(200, { "content-type": "application/json" });
      response.end('{"ok":true}');
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    if (request.url === "/page-2") {
      response.end(
        '<!doctype html><script src="/two.js"></script><a href="/page-3">three</a>',
      );
      return;
    }
    if (request.url === "/page-3") {
      response.end('<!doctype html><script src="/three.js"></script>');
      return;
    }
    response.end(`<!doctype html>
      <script src="/root.js"></script>
      <script>fetch('/api/root')</script>
      <a href="/page-2">two</a>
      <a href="/page-3">three</a>`);
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "golish-browser-page-resume-"));
  const baseArgs = [
    "--url",
    `http://127.0.0.1:${targetAddress.port}/`,
    "--workspace",
    workspace,
    "--max-pages",
    "1",
    "--max-actions",
    "0",
    "--max-recursive-scripts",
    "10",
    "--hard-timeout-ms",
    "20000",
    "--ai-assist",
    "false",
  ];
  const sameRunArgs = [
    ...baseArgs,
    "--run-id",
    "run-page-a",
    "--session-id",
    "session-page-a",
    "--operation-id",
    "00000000-0000-0000-0000-000000000011",
    "--stage-started-at",
    "2020-01-01T00:00:00Z",
  ];

  try {
    const first = await runCollector(sameRunArgs);
    assert.equal(first.completion_state, "partial");
    assert.equal(first.page_resume_applied, false);
    assert.equal(first.scripts_saved, 1);
    assert.equal(first.api_requests_total, 1);
    assert.equal(first.page_queue_remaining, 2);

    const second = await runCollector(sameRunArgs);
    assert.equal(second.completion_state, "partial");
    assert.equal(second.page_resume_applied, true);
    assert.equal(second.scripts_cached_preloaded, 1);
    assert.equal(second.scripts_saved, 2);
    assert.equal(second.api_requests_total, 1);
    assert.equal(second.page_queue_remaining, 1);

    const third = await runCollector(sameRunArgs);
    assert.equal(third.completion_state, "complete");
    assert.equal(third.page_resume_applied, true);
    assert.equal(third.scripts_cached_preloaded, 2);
    assert.equal(third.scripts_saved, 3);
    assert.equal(third.api_requests_total, 1);
    assert.equal(third.pages_visited.length, 3);
    const completedManifest = JSON.parse(
      await fs.readFile(path.join(workspace, third.script_manifest), "utf8"),
    );
    assert.equal(completedManifest.closure_complete, true);
    assert.equal(completedManifest.scripts.length, 3);
    assert.equal(completedManifest.api_requests.length, 1);
    assert.deepEqual(completedManifest.pending_pages, []);

    const newRun = await runCollector([
      ...baseArgs,
      "--run-id",
      "run-page-b",
      "--session-id",
      "session-page-b",
      "--operation-id",
      "00000000-0000-0000-0000-000000000012",
      "--stage-started-at",
      "2020-01-01T00:00:00Z",
    ]);
    assert.equal(newRun.completion_state, "partial");
    assert.equal(newRun.page_resume_applied, false);
    assert.equal(newRun.scripts_cached_preloaded, 0);
    assert.equal(newRun.scripts_saved, 1);
    const newManifest = JSON.parse(
      await fs.readFile(path.join(workspace, newRun.script_manifest), "utf8"),
    );
    assert.equal(newManifest.producer_run_id, "run-page-b");
    assert.equal(newManifest.producer_session_id, "session-page-b");
    assert.equal(
      newManifest.producer_operation_id,
      "00000000-0000-0000-0000-000000000012",
    );
    assert.equal(newManifest.producer_stage_started_at, "2020-01-01T00:00:00Z");
    assert.equal(newManifest.scripts.length, 1);

    const differentStageAgainstPartialCheckpoint = await runCollector([
      ...baseArgs,
      "--run-id",
      "run-page-b",
      "--session-id",
      "session-page-b",
      "--operation-id",
      "00000000-0000-0000-0000-000000000012",
      "--stage-started-at",
      "2020-01-02T00:00:00Z",
    ]);
    assert.equal(differentStageAgainstPartialCheckpoint.completion_state, "partial");
    assert.equal(differentStageAgainstPartialCheckpoint.page_resume_applied, false);
    assert.equal(differentStageAgainstPartialCheckpoint.scripts_cached_preloaded, 0);
    assert.equal(differentStageAgainstPartialCheckpoint.scripts_saved, 1);
    assert.deepEqual(differentStageAgainstPartialCheckpoint.pages_visited, [
      `http://127.0.0.1:${targetAddress.port}/`,
    ]);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("page budget checkpoints every safe same-origin link instead of dropping candidates", async () => {
  const links = Array.from(
    { length: 301 },
    (_unused, index) => `<a href="/page-${index + 1}">page ${index + 1}</a>`,
  ).join("\n");
  const target = http.createServer((request, response) => {
    response.writeHead(200, { "content-type": "text/html" });
    response.end(request.url === "/" ? `<!doctype html>${links}` : "<!doctype html>");
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(
    path.join(os.tmpdir(), "golish-browser-wide-page-resume-"),
  );
  const args = [
    "--url",
    `http://127.0.0.1:${targetAddress.port}/`,
    "--workspace",
    workspace,
    "--max-pages",
    "1",
    "--max-actions",
    "0",
    "--max-recursive-scripts",
    "10",
    "--hard-timeout-ms",
    "20000",
    "--ai-assist",
    "false",
    "--run-id",
    "run-wide-page",
    "--session-id",
    "session-wide-page",
    "--operation-id",
    "00000000-0000-0000-0000-000000000013",
    "--stage-started-at",
    "2020-01-01T00:00:00Z",
  ];

  try {
    const first = await runCollector(args);
    assert.equal(first.completion_state, "partial");
    assert.equal(first.page_queue_remaining, 301);
    assert.equal(first.page_candidates_dropped, 0);
    assert.deepEqual(first.closure_incomplete_reasons, ["page_queue_remaining"]);

    const manifest = JSON.parse(
      await fs.readFile(path.join(workspace, first.script_manifest), "utf8"),
    );
    assert.equal(manifest.pending_pages.length, 301);

    const second = await runCollector(args);
    assert.equal(second.page_resume_applied, true);
    assert.equal(second.pages_visited_this_run.length, 1);
    assert.equal(second.page_queue_remaining, 300);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("same-provenance recursive checkpoint advances a bounded chunk chain", async () => {
  const target = http.createServer((request, response) => {
    if (request.url === "/") {
      response.writeHead(200, { "content-type": "text/html" });
      response.end('<!doctype html><script src="/chunk-0.js"></script>');
      return;
    }
    const match = /^\/chunk-(\d+)\.js$/.exec(request.url ?? "");
    if (match) {
      const index = Number(match[1]);
      response.writeHead(200, { "content-type": "text/javascript" });
      response.end(
        index < 3
          ? `const nextChunk = "/chunk-${index + 1}.js";`
          : "window.chunkClosureComplete = true;",
      );
      return;
    }
    response.writeHead(404);
    response.end();
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(
    path.join(os.tmpdir(), "golish-browser-recursive-resume-"),
  );
  const args = [
    "--url",
    `http://127.0.0.1:${targetAddress.port}/`,
    "--workspace",
    workspace,
    "--max-pages",
    "1",
    "--max-actions",
    "0",
    "--max-recursive-scripts",
    "1",
    "--hard-timeout-ms",
    "20000",
    "--ai-assist",
    "false",
    "--run-id",
    "run-recursive",
    "--session-id",
    "session-recursive",
    "--operation-id",
    "00000000-0000-0000-0000-000000000014",
    "--stage-started-at",
    "2020-01-01T00:00:00Z",
  ];

  try {
    const first = await runCollector(args);
    assert.equal(first.completion_state, "partial");
    assert.equal(first.scripts_saved, 2);
    assert.equal(first.recursive_queue_remaining, 1);
    assert.equal(first.recursive_resume_applied, false);

    const second = await runCollector(args);
    assert.equal(second.completion_state, "partial");
    assert.equal(second.recursive_resume_applied, true);
    assert.equal(second.scripts_cached_preloaded, 2);
    assert.equal(second.scripts_saved, 3);
    assert.equal(second.recursive_queue_remaining, 1);

    const third = await runCollector(args);
    assert.equal(third.completion_state, "complete");
    assert.equal(third.recursive_resume_applied, true);
    assert.equal(third.scripts_cached_preloaded, 3);
    assert.equal(third.scripts_saved, 4);
    assert.equal(third.recursive_queue_remaining, 0);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("repeated navigation failure exhausts one retry signature without a third request", async () => {
  let brokenHits = 0;
  const target = http.createServer((request, response) => {
    if (request.url === "/broken") {
      brokenHits += 1;
      request.socket.destroy();
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    response.end('<!doctype html><a href="/broken">broken</a>');
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(
    path.join(os.tmpdir(), "golish-browser-recovery-breaker-"),
  );
  const args = [
    "--url",
    `http://127.0.0.1:${targetAddress.port}/`,
    "--workspace",
    workspace,
    "--max-pages",
    "2",
    "--max-actions",
    "0",
    "--max-recursive-scripts",
    "10",
    "--hard-timeout-ms",
    "20000",
    "--ai-assist",
    "false",
    "--run-id",
    "run-recovery-breaker",
    "--session-id",
    "session-recovery-breaker",
    "--operation-id",
    "00000000-0000-0000-0000-000000000015",
    "--stage-started-at",
    "2020-01-01T00:00:00Z",
  ];

  try {
    const first = await runCollector(args);
    assert.equal(first.completion_state, "partial");
    assert.equal(first.automatic_retry_allowed, true);
    assert.equal(first.recovery_failures.length, 1);
    assert.equal(first.recovery_failures[0].kind, "navigation");
    assert.equal(first.recovery_failures[0].count, 1);

    const second = await runCollector(args);
    assert.equal(second.completion_state, "error");
    assert.equal(second.recovery_exhausted, true);
    assert.equal(second.automatic_retry_allowed, false);
    assert.equal(second.recovery_failures[0].count, 2);
    assert.match(second.recovery_instruction, /new trusted producer operation/);
    const hitsAfterExhaustion = brokenHits;

    const third = await runCollector(args);
    assert.equal(third.completion_state, "error");
    assert.equal(third.checkpoint_resume_applied, true);
    assert.equal(third.recovery_exhausted, true);
    assert.equal(third.automatic_retry_allowed, false);
    assert.equal(brokenHits, hitsAfterExhaustion);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("cached script clears a stale script-body recovery failure after its retry page drained", async () => {
  const target = http.createServer((request, response) => {
    if (request.url === "/app.js") {
      response.writeHead(200, { "content-type": "text/javascript" });
      response.end("window.cachedRecoveryClosed = true;");
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    response.end('<!doctype html><script src="/app.js"></script>');
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(
    path.join(os.tmpdir(), "golish-browser-stale-script-recovery-"),
  );
  const targetUrl = `http://127.0.0.1:${targetAddress.port}/`;
  const scriptUrl = `http://127.0.0.1:${targetAddress.port}/app.js`;
  const args = [
    "--url",
    targetUrl,
    "--workspace",
    workspace,
    "--max-pages",
    "1",
    "--max-actions",
    "0",
    "--max-recursive-scripts",
    "10",
    "--hard-timeout-ms",
    "20000",
    "--ai-assist",
    "false",
    "--run-id",
    "run-stale-script-recovery",
    "--session-id",
    "session-stale-script-recovery",
    "--operation-id",
    "00000000-0000-0000-0000-000000000016",
    "--stage-started-at",
    "2020-01-01T00:00:00Z",
  ];

  try {
    const first = await runCollector(args);
    assert.equal(first.completion_state, "complete");
    assert.equal(first.scripts_saved, 1);

    const manifestPath = path.join(workspace, first.script_manifest);
    const manifest = JSON.parse(await fs.readFile(manifestPath, "utf8"));
    const signature = crypto
      .createHash("sha256")
      .update(`script_body\0${scriptUrl}`)
      .digest("hex");
    Object.assign(manifest, {
      closure_complete: false,
      completion_state: "partial",
      closure_incomplete_reasons: ["recovery_pending"],
      pending_pages: [],
      pending_recursive_scripts: [],
      checkpoint_resume_count: 13,
      recovery_failures: [
        {
          signature,
          kind: "script_body",
          url: scriptUrl,
          count: 1,
          reason:
            "response.body: Protocol error (Network.getResponseBody): No resource with given identifier found",
        },
      ],
      recovery_exhausted: false,
      automatic_retry_allowed: true,
    });
    await fs.writeFile(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

    const resumed = await runCollector(args);
    assert.equal(resumed.checkpoint_resume_applied, true);
    assert.equal(resumed.scripts_cached_preloaded, 1);
    assert.equal(resumed.completion_state, "complete");
    assert.equal(resumed.closure_complete, true);
    assert.deepEqual(resumed.recovery_failures, []);
    assert.equal(resumed.recovery_exhausted, false);
    assert.equal(resumed.automatic_retry_allowed, true);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("Enumeration observes unsafe APIs but never sends mutations clicks or dangerous routes", async () => {
  let mutationHits = 0;
  const target = http.createServer((request, response) => {
    if (
      request.url?.startsWith("/mutate") ||
      request.url?.startsWith("/delete-account") ||
      request.url?.startsWith("/logout") ||
      request.url?.startsWith("/remove")
    ) {
      mutationHits += 1;
      response.writeHead(500);
      response.end("mutation must never be reached");
      return;
    }
    response.writeHead(200, { "content-type": "text/html" });
    response.end(`<!doctype html>
      <div role="button" onclick="fetch('/delete-account', { method: 'DELETE' })">Delete</div>
      <a href="/logout?confirm=true">Logout</a>
      <script>
        new WebSocket('ws://' + location.host + '/socket');
        fetch('/mutate?source=automatic', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ account_id: 7 })
        }).catch(() => {});
      </script>`);
  });
  target.on("upgrade", (_request, socket) => {
    mutationHits += 1;
    socket.destroy();
  });
  const targetAddress = await listen(target);
  const workspace = await fs.mkdtemp(path.join(os.tmpdir(), "golish-browser-read-only-"));

  try {
    const result = await runCollector([
      "--url",
      `http://127.0.0.1:${targetAddress.port}/`,
      "--workspace",
      workspace,
      "--max-pages",
      "10",
      "--max-actions",
      "5",
      "--max-recursive-scripts",
      "10",
      "--hard-timeout-ms",
      "20000",
      "--ai-assist",
      "false",
      "--recipe-json",
      JSON.stringify({
        routes: ["/remove"],
        manifest_paths: ["/%256cogout?confirm=true"],
        script_urls: ["/delete-account"],
        click_texts: ["Delete"],
      }),
    ]);

    assert.equal(mutationHits, 0);
    assert.equal(result.completion_state, "complete");
    assert.equal(result.read_only_enumeration, true);
    assert.equal(result.interactive_actions_authorized, false);
    assert.equal(result.requested_max_actions, 5);
    assert.equal(result.max_actions, 0);
    assert.equal(result.actions_clicked, 0);
    assert.equal(result.disabled_recipe_click_texts, 1);
    assert.equal(result.disabled_recipe_manifest_paths, 1);
    assert.equal(result.disabled_recipe_script_urls, 1);
    assert.ok(result.blocked_websocket_urls.some((url) => url.includes("/socket")));
    assert.ok(result.blocked_read_only_routes.some((url) => url.includes("/logout")));
    assert.ok(result.blocked_read_only_routes.some((url) => url.includes("/remove")));
    const unsafe = result.api_requests.find(
      (request) => request.method === "POST" && request.url.includes("/mutate"),
    );
    assert.ok(unsafe, "the blocked POST must remain an observed API candidate");
    assert.equal(unsafe.status, null);
    assert.equal(unsafe.read_only_blocked, true);
    assert.equal(unsafe.read_only_block_reason, "method_not_read_only");
    assert.match(unsafe.request_body, /account_id/);
    assert.ok(result.scope_exclusions >= 3);
    assert.equal(result.scope_violations, 0);
  } finally {
    await fs.rm(workspace, { recursive: true, force: true });
    await close(target);
  }
});

test("partial navigation page truncation and byte caps remain incomplete", () => {
  for (const input of [
    { navigation_attempts: 2, successful_pages: 1, navigation_errors: 1 },
    { navigation_attempts: 1, successful_pages: 1, page_queue_remaining: 3 },
    { navigation_attempts: 1, successful_pages: 1, page_candidates_dropped: 2 },
    { navigation_attempts: 1, successful_pages: 1, script_byte_limit_skips: 1 },
  ]) {
    const result = classifyCollectionCompletion(input);
    assert.equal(result.status, "closure_partial");
    assert.equal(result.completion_state, "partial");
    assert.equal(result.closure_complete, false);
  }
});

test("a successful unbounded collection remains complete", () => {
  const result = classifyCollectionCompletion({
    navigation_attempts: 1,
    successful_pages: 1,
  });
  assert.deepEqual(result, {
    status: "ok",
    completion_state: "complete",
    closure_complete: true,
    reasons: [],
  });
});
