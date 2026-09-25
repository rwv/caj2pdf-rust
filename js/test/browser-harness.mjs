// SPDX-License-Identifier: MIT

/**
 * Zero-dependency harness for the real-browser tests: a localhost static
 * server, a headless Chromium process, and a minimal Chrome DevTools
 * Protocol client over Node's global WebSocket. Nothing is fetched from the
 * network; Chromium is told to resolve every host except 127.0.0.1 to
 * nothing and to skip its background services.
 */
import { spawn } from "node:child_process";
import { existsSync, readdirSync, rmSync } from "node:fs";
import { mkdtemp, readFile, realpath, rm } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { extname, join, resolve, sep } from "node:path";

const PW_BROWSERS = process.env.PLAYWRIGHT_BROWSERS_PATH || "/opt/pw-browsers";

/**
 * Locate a Chromium-based browser: `CAJ2PDF_CHROME`, then a system Chrome
 * or Chromium, then a preinstalled Playwright browser directory. Returns
 * `null` when none is found; throws when `CAJ2PDF_CHROME` names a missing
 * file, since that is a configuration error.
 */
export function findChrome() {
  const configured = process.env.CAJ2PDF_CHROME;
  if (configured) {
    if (!existsSync(configured)) throw new Error(`CAJ2PDF_CHROME=${configured} does not exist`);
    return configured;
  }
  const candidates = [
    "/usr/bin/google-chrome",
    "/usr/bin/google-chrome-stable",
    "/usr/bin/chromium",
    "/usr/bin/chromium-browser",
  ];
  let installed = [];
  try {
    installed = readdirSync(PW_BROWSERS).sort().reverse();
  } catch {
    // No Playwright browser directory.
  }
  for (const name of installed) {
    if (name.startsWith("chromium_headless_shell-")) {
      candidates.push(join(PW_BROWSERS, name, "chrome-linux", "headless_shell"));
    } else if (name.startsWith("chromium-")) {
      candidates.push(join(PW_BROWSERS, name, "chrome-linux", "chrome"));
    }
  }
  return candidates.find((path) => existsSync(path)) ?? null;
}

const TYPES = {
  ".html": "text/html; charset=utf-8",
  ".mjs": "text/javascript; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".wasm": "application/wasm",
};

function notFound() {
  throw Object.assign(new Error("not found"), { code: "ENOENT" });
}

/**
 * Serve `root` read-only (GET and HEAD) on 127.0.0.1 plus in-memory
 * `routes` (`{ "/path": Uint8Array | string }`). Paths that resolve outside
 * `root`, including through symbolic links, are refused.
 */
export async function startServer(root, routes) {
  const base = await realpath(root);
  const server = createServer(async (request, response) => {
    if (request.method !== "GET" && request.method !== "HEAD") {
      response.writeHead(405, { allow: "GET, HEAD" });
      response.end();
      return;
    }
    try {
      const path = decodeURIComponent(new URL(request.url, "http://127.0.0.1").pathname);
      let body = Object.hasOwn(routes, path) ? routes[path] : undefined;
      if (body === undefined) {
        const file = await realpath(resolve(base, `.${path}`)).catch(notFound);
        if (!file.startsWith(base + sep)) notFound();
        body = await readFile(file);
      }
      response.writeHead(200, {
        "content-type": TYPES[extname(path)] ?? "application/octet-stream",
        "cache-control": "no-store",
      });
      response.end(request.method === "HEAD" ? undefined : body);
    } catch (error) {
      response.writeHead(["ENOENT", "ENOTDIR", "EISDIR"].includes(error.code) ? 404 : 500);
      response.end();
    }
  });
  await new Promise((done, fail) => {
    server.once("error", fail);
    server.listen(0, "127.0.0.1", done);
  });
  return {
    origin: `http://127.0.0.1:${server.address().port}`,
    close() {
      server.closeAllConnections();
      return new Promise((done) => server.close(() => done()));
    },
  };
}

const COMMAND_TIMEOUT = 30_000;

function withTimeout(promise, milliseconds, what) {
  let timer;
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(new Error(`${what} timed out after ${milliseconds} ms`)), milliseconds);
  });
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timer));
}

/** A minimal CDP client: flat sessions, request/response, and events. */
class Cdp {
  #socket;
  #nextId = 1;
  #pending = new Map();
  #listeners = new Set();

  constructor(socket) {
    this.#socket = socket;
    socket.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      const pending = this.#pending.get(message.id);
      if (pending) {
        this.#pending.delete(message.id);
        if (message.error) pending.reject(new Error(`CDP ${pending.method}: ${message.error.message}`));
        else pending.resolve(message.result);
      } else if (message.method) {
        for (const listener of this.#listeners) listener(message);
      }
    });
    socket.addEventListener("close", () => {
      for (const pending of this.#pending.values()) pending.reject(new Error("CDP connection closed"));
      this.#pending.clear();
    });
  }

  static async connect(url) {
    const socket = new WebSocket(url);
    await new Promise((done, fail) => {
      socket.addEventListener("open", done, { once: true });
      socket.addEventListener("error", () => fail(new Error(`cannot connect to ${url}`)), { once: true });
    });
    return new Cdp(socket);
  }

  /** Send a command; it rejects if the connection closes or `timeout` passes. */
  send(method, params = {}, sessionId, timeout = COMMAND_TIMEOUT) {
    if (this.#socket.readyState !== WebSocket.OPEN) {
      return Promise.reject(new Error(`CDP ${method}: connection is not open`));
    }
    const id = this.#nextId++;
    const response = new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve, reject, method });
      this.#socket.send(JSON.stringify({ id, method, params, sessionId }));
    });
    return withTimeout(response, timeout, `CDP ${method}`).finally(() => this.#pending.delete(id));
  }

  /** Resolve with the first event named `method` on `sessionId`. */
  once(method, sessionId) {
    return new Promise((resolve) => {
      const listener = (message) => {
        if (message.method === method && message.sessionId === sessionId) {
          this.#listeners.delete(listener);
          resolve(message.params);
        }
      };
      this.#listeners.add(listener);
    });
  }

  on(listener) {
    this.#listeners.add(listener);
  }

  close() {
    this.#socket.close();
  }
}

/**
 * Launch headless Chromium with a throwaway profile and connect over CDP.
 * `close()` always kills the process and removes the profile.
 */
export async function launchChrome(executable, { startupTimeout = 30_000 } = {}) {
  const profile = await mkdtemp(join(tmpdir(), "caj2pdf-chrome-"));
  const child = spawn(executable, [
    "--headless=new",
    "--no-sandbox",
    "--remote-debugging-port=0",
    `--user-data-dir=${profile}`,
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-gpu",
    "--disable-dev-shm-usage",
    "--disable-extensions",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-default-apps",
    "--disable-sync",
    "--no-proxy-server",
    "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1",
    "about:blank",
  ], { stdio: ["ignore", "ignore", "pipe"], detached: true });
  let stderr = "";
  child.stderr.setEncoding("utf8").on("data", (text) => {
    stderr = (stderr + text).slice(-4096);
  });
  let failure;
  child.once("error", (error) => {
    failure = error;
  });
  // "close" waits for every Chromium process, since they share stderr.
  const exited = new Promise((resolve) => child.once("close", resolve));
  // Chromium leads its own process group so that its helper processes can
  // be killed at once; a no-op once they have exited.
  const kill = () => {
    try {
      process.kill(-child.pid, "SIGKILL");
    } catch {
      // Not started, or already gone.
    }
  };
  // Last resort when the test process ends without close(), e.g. after a
  // hook timeout or on SIGINT/SIGTERM: orphaned Chromium processes and the
  // profile would otherwise outlive it.
  const onExit = () => {
    kill();
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
    } catch {
      // Best effort while exiting.
    }
  };
  const onSignal = (signal) => {
    onExit();
    unregister();
    process.kill(process.pid, signal);
  };
  const unregister = () => {
    process.off("exit", onExit).off("SIGINT", onSignal).off("SIGTERM", onSignal);
  };
  process.once("exit", onExit).once("SIGINT", onSignal).once("SIGTERM", onSignal);
  let cdp;
  let closing;
  const close = () => {
    closing ??= (async () => {
      if (cdp) {
        await cdp.send("Browser.close", {}, undefined, 5_000).catch(() => {});
        cdp.close();
      }
      kill();
      await withTimeout(exited, 10_000, "Chromium exit").catch(() => {});
      await rm(profile, { recursive: true, force: true, maxRetries: 5, retryDelay: 50 });
      unregister();
    })();
    return closing;
  };
  try {
    const portFile = join(profile, "DevToolsActivePort");
    let waiting = true;
    const ready = (async () => {
      while (waiting) {
        if (failure || child.exitCode !== null || child.signalCode !== null) {
          const reason = failure?.message ?? child.exitCode ?? child.signalCode;
          throw new Error(`Chromium exited early (${reason}):\n${stderr}`);
        }
        // "<port>\n<path>"; Chromium may still be writing it.
        const [port, path] = (await readFile(portFile, "utf8").catch(() => "")).split("\n");
        if (/^\d+$/.test(port) && /^\/devtools\/browser\/[\w-]+$/.test(path ?? "")) {
          return `ws://127.0.0.1:${port}${path}`;
        }
        await new Promise((done) => setTimeout(done, 50));
      }
    })();
    const endpoint = await withTimeout(ready, startupTimeout, "Chromium startup").finally(() => {
      waiting = false;
    });
    cdp = await withTimeout(Cdp.connect(endpoint), startupTimeout, "DevTools connection");
    return { cdp, close };
  } catch (error) {
    await close();
    throw error;
  }
}

/**
 * Open `url` in a new tab and return `evaluate(expression)`, which awaits the
 * expression's promise in the page and returns its JSON value. Each step
 * times out after `timeout` milliseconds.
 */
export async function openPage(cdp, url, { timeout = COMMAND_TIMEOUT } = {}) {
  const { targetId } = await cdp.send("Target.createTarget", { url: "about:blank" });
  const { sessionId } = await cdp.send("Target.attachToTarget", { targetId, flatten: true });
  const errors = [];
  cdp.on((message) => {
    if (message.sessionId === sessionId && message.method === "Runtime.exceptionThrown") {
      errors.push(message.params.exceptionDetails.exception?.description ?? message.params.exceptionDetails.text);
    }
  });
  await cdp.send("Page.enable", {}, sessionId);
  await cdp.send("Runtime.enable", {}, sessionId);
  const loaded = cdp.once("Page.loadEventFired", sessionId);
  const navigation = await cdp.send("Page.navigate", { url }, sessionId);
  if (navigation.errorText) throw new Error(`navigation to ${url} failed: ${navigation.errorText}`);
  await withTimeout(loaded, timeout, `loading ${url}`);
  return {
    errors,
    async evaluate(expression) {
      const { result, exceptionDetails } = await cdp.send(
        "Runtime.evaluate",
        { expression, awaitPromise: true, returnByValue: true },
        sessionId,
        timeout,
      );
      if (exceptionDetails) {
        throw new Error(`page threw: ${exceptionDetails.exception?.description ?? exceptionDetails.text}`);
      }
      return result.value;
    },
  };
}
