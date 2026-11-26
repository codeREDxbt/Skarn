// Native WebView2 smoke checks. This driver never sends model requests or edits user workspaces.
// Build the frontend and desktop binary first, then run:
// node scripts/agents-window-smoke.mjs --exe target/debug/skarn-desktop.exe
import { spawn, execFileSync } from 'node:child_process';
import { mkdir, writeFile, readFile } from 'node:fs/promises';
import { resolve, join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const argument = (name, fallback) => {
  const index = process.argv.indexOf(name);
  return index < 0 ? fallback : process.argv[index + 1];
};
const executable = resolve(root, argument('--exe', 'target/debug/skarn-desktop.exe'));
const port = Number(argument('--port', '9347'));
const artifacts = resolve(argument('--artifacts', join(process.env.LOCALAPPDATA ?? process.env.TEMP,
  'Temp', 'opencode', `selora-agents-smoke-${Date.now()}`)));
const results = [];
const connections = [];
let child;

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function until(action, description, timeout = 30000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    try {
      const value = await action();
      if (value) return value;
    } catch (error) { last = error; }
    await delay(150);
  }
  throw new Error(`Timed out: ${description}${last ? ` (${last.message})` : ''}`);
}

async function pages() {
  const response = await fetch(`http://127.0.0.1:${port}/json/list`);
  return (await response.json()).filter((target) => target.type === 'page');
}

class Page {
  constructor(socket) {
    this.socket = socket;
    this.next = 0;
    this.pending = new Map();
    this.errors = [];
    this.logs = [];
    socket.addEventListener('message', ({ data }) => {
      const value = JSON.parse(data);
      if (value.method === 'Runtime.exceptionThrown') this.errors.push(value.params.exceptionDetails);
      if (value.method === 'Runtime.consoleAPICalled' || value.method === 'Log.entryAdded') this.logs.push(value);
      const request = this.pending.get(value.id);
      if (!request) return;
      this.pending.delete(value.id);
      clearTimeout(request.timer);
      if (value.error) request.reject(new Error(value.error.message));
      else request.resolve(value.result);
    });
    socket.addEventListener('close', () => {
      for (const request of this.pending.values()) {
        clearTimeout(request.timer);
        request.reject(new Error('WebView closed'));
      }
      this.pending.clear();
    });
  }
  static async connect(target) {
    const socket = new WebSocket(target.webSocketDebuggerUrl);
    await new Promise((resolve, reject) => {
      socket.addEventListener('open', resolve, { once: true });
      socket.addEventListener('error', reject, { once: true });
    });
    const page = new Page(socket);
    connections.push(page);
    await page.call('Runtime.enable');
    await page.call('Log.enable').catch(() => {});
    return page;
  }
  call(method, params = {}) {
    const id = ++this.next;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(id);
        reject(new Error(`CDP timeout: ${method}`));
      }, 20000);
      this.pending.set(id, { resolve, reject, timer });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }
  async evaluate(expression) {
    const result = await this.call('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text);
    return result.result?.value;
  }
  invoke(command, args = {}) {
    return this.evaluate(`window.__TAURI_INTERNALS__.invoke(${JSON.stringify(command)},${JSON.stringify(args)})`);
  }
  async screenshot(name) {
    const { data } = await this.call('Page.captureScreenshot', { format: 'png' });
    await writeFile(join(artifacts, `${name}.png`), Buffer.from(data, 'base64'));
  }
  async click(label) {
    const found = await this.evaluate(`(() => {
      const button = [...document.querySelectorAll('button')].find(button => button.getAttribute('aria-label') === ${JSON.stringify(label)});
      if (!button) return false;
      button.click(); return true;
    })()`);
    assert(found, `Missing button: ${label}`);
  }
}

async function context(page) {
  const bootstrap = await page.invoke('get_bootstrap_state', { protocolVersion: 1 });
  return { protocol_version: 1, workspace_id: bootstrap.workspace.id,
    core_instance_id: bootstrap.core_instance_id, session_id: bootstrap.chat.active_session_id,
    request_id: crypto.randomUUID() };
}

async function dispatch(page, payload) {
  return page.invoke('dispatch_command', { envelope: { ...await context(page), payload } });
}

async function check(name, run) {
  await run();
  results.push({ name, passed: true });
  console.log(`PASS ${name}`);
}

try {
  assert(process.platform === 'win32', 'Native smoke requires Windows/WebView2.');
  const processes = execFileSync('tasklist', ['/FI', 'IMAGENAME eq skarn-desktop.exe', '/FO', 'CSV', '/NH'], { encoding: 'utf8' });
  assert(!processes.toLowerCase().includes('"skarn-desktop.exe"'), 'Close existing Selora processes before isolated smoke checks.');
  await mkdir(artifacts, { recursive: true });
  const settings = join(artifacts, 'settings');
  const workspace = join(artifacts, 'workspace');
  await mkdir(settings, { recursive: true });
  await mkdir(workspace, { recursive: true });
  await writeFile(join(workspace, 'smoke.ts'), 'export const nativeAgentsSmoke = true;\n');
  execFileSync('git', ['init', workspace], { stdio: 'ignore' });
  child = spawn(executable, [], { cwd: root, env: { ...process.env,
    SELORA_SETTINGS_DIR: settings, WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}` },
    stdio: ['ignore', 'pipe', 'pipe'] });
  let output = '';
  child.stdout.on('data', (value) => { output += value; });
  child.stderr.on('data', (value) => { output += value; });
  child.on('error', (error) => { output += error.stack; });
  const mainTarget = await until(async () => (await pages()).find((page) => !page.url.includes('surface=agents')), 'main native WebView');
  const main = await Page.connect(mainTarget);
   await until(() => main.evaluate(`!!document.querySelector("[aria-label='Open Agents Window']")`), 'main title bar');
  await dispatch(main, { type: 'open_folder_at', payload: { root: workspace } });
  const created = await dispatch(main, { type: 'new_session' });
  const sessionId = created?.session_id ?? (await main.invoke('get_bootstrap_state', { protocolVersion: 1 })).chat.active_session_id;
  let agents;
  await check('title bar creates one additional native Agents window', async () => {
    const before = await main.invoke('get_bootstrap_state', { protocolVersion: 1 });
     await main.click('Open Agents Window');
    const target = await until(async () => (await pages()).find((page) => page.url.includes('surface=agents')), 'Agents native WebView');
    agents = await Page.connect(target);
    await agents.evaluate(`(() => {
      window.__seloraSmoke = { bootstrap: 'pending', agents: 'pending' };
      window.__TAURI_INTERNALS__.invoke('get_bootstrap_state', { protocolVersion: 1 })
        .then(() => { window.__seloraSmoke.bootstrap = 'resolved'; })
        .catch((error) => { window.__seloraSmoke.bootstrap = 'rejected:' + String(error); });
      window.__TAURI_INTERNALS__.invoke('get_agents_snapshot', { protocolVersion: 1 })
        .then(() => { window.__seloraSmoke.agents = 'resolved'; })
        .catch((error) => { window.__seloraSmoke.agents = 'rejected:' + String(error); });
    })()`);
    await until(async () => {
      const state = await agents.evaluate(`(() => ({
        ready: document.readyState,
        text: document.body?.innerText?.slice(0, 600) ?? '',
        root: !!document.querySelector('.agents-workspace'),
        alerts: [...document.querySelectorAll('[role="alert"]')].map((node) => node.textContent),
        smoke: window.__seloraSmoke ?? null,
      }))()`);
      if (state.root) return true;
      if (state.alerts.length || state.text) {
        await writeFile(join(artifacts, 'agents-dom.json'), JSON.stringify(state, null, 2));
      }
      return false;
    }, 'Agents workspace');
     await main.click('Open Agents Window');
     await main.click('Open Agents Window');
    assert((await pages()).filter((page) => page.url.includes('surface=agents')).length === 1, 'Duplicate Agents WebViews');
    assert(await main.evaluate('!document.querySelector(".agents-workspace")'), 'Main surface was replaced');
    const after = await main.invoke('get_bootstrap_state', { protocolVersion: 1 });
    assert(JSON.stringify(before.layout) === JSON.stringify(after.layout), 'Opening Agents changed main dock layout');
  });
  await check('both windows expose the same authoritative sessions', async () => {
    const a = await main.invoke('get_agents_snapshot', { protocolVersion: 1 });
    const b = await agents.invoke('get_agents_snapshot', { protocolVersion: 1 });
    assert(a.workspace_id === b.workspace_id, 'Workspace mismatch');
    assert(a.sessions.some((session) => session.session_id === sessionId), 'Main session missing');
    assert(b.sessions.some((session) => session.session_id === sessionId), 'Agents session missing');
  });
  await check('minimized Agents restores through the same native open operation', async () => {
    await agents.invoke('plugin:window|minimize', { label: 'agents' });
     await main.click('Open Agents Window');
    await until(async () => !await agents.invoke('plugin:window|is_minimized', { label: 'agents' }), 'Agents unminimizes');
    assert((await pages()).filter((page) => page.url.includes('surface=agents')).length === 1, 'Restoring duplicated Agents');
  });
  await check('shared preferences synchronize through native persistence', async () => {
    const updated = await agents.invoke('update_agent_preferences', { patch: { systemInstructions: 'Native smoke fixture only.' } });
    const observed = await main.invoke('get_agent_preferences');
    assert(updated.revision === observed.revision && observed.systemInstructions === 'Native smoke fixture only.', 'Preference mismatch');
    const disk = JSON.parse(await readFile(join(settings, 'agent-preferences.json'), 'utf8'));
    assert(disk.revision === updated.revision, 'Preferences not persisted');
  });
  await check('read-only previews do not activate main editor buffers', async () => {
    const before = await main.invoke('get_bootstrap_state', { protocolVersion: 1 });
    const preview = await agents.invoke('agents_read_file_preview', { workspaceId: before.workspace.id, path: 'smoke.ts' });
    const after = await main.invoke('get_bootstrap_state', { protocolVersion: 1 });
    assert(preview.text.includes('nativeAgentsSmoke'), 'File preview missing');
    assert(after.active_buffer_id === before.active_buffer_id && after.buffers.length === before.buffers.length, 'Preview activated editor');
  });
  await check('computed layout stays inside bounded native viewports', async () => {
    for (const [width, height] of [[960, 640], [1280, 720], [1440, 900], [1920, 1080]]) {
      await agents.call('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 1, mobile: false });
      await delay(150);
      const layout = await agents.evaluate(`(() => {
        const root = document.querySelector('.agents-workspace');
        const main = document.querySelector('.agents-workspace__conversation');
        const composer = document.querySelector('textarea');
        const rect = element => { const r=element?.getBoundingClientRect(); return r ? {x:r.x,y:r.y,width:r.width,height:r.height,right:r.right,bottom:r.bottom}:null; };
        return {root:rect(root),main:rect(main),composer:rect(composer),width:innerWidth,height:innerHeight,
          scrollWidth:document.documentElement.scrollWidth,scrollHeight:document.documentElement.scrollHeight};
      })()`);
      assert(layout.root && layout.main, `Missing layout at ${width}x${height}`);
      assert(layout.scrollWidth <= width + 1 && layout.scrollHeight <= height + 1, `Outer overflow at ${width}x${height}`);
      assert(layout.main.width >= 480, `Conversation too narrow at ${width}x${height}`);
      assert(layout.composer && layout.composer.width > 200 && layout.composer.bottom <= height + 1, `Composer clipped at ${width}x${height}`);
      results.push({ name: `layout ${width}x${height}`, passed: true, layout });
      await agents.screenshot(`agents-${width}x${height}`);
    }
    await agents.call('Emulation.clearDeviceMetricsOverride');
  });
  await check('closing and reopening Agents leaves main backend alive', async () => {
    await agents.click('Close window');
    await until(async () => !(await pages()).some((page) => page.url.includes('surface=agents')), 'Agents closes');
    assert((await main.invoke('get_agents_snapshot', { protocolVersion: 1 })).sessions.some((session) => session.session_id === sessionId), 'Session lost on auxiliary close');
     await main.click('Open Agents Window');
    await until(async () => (await pages()).some((page) => page.url.includes('surface=agents')), 'Agents reopens');
  });
  assert(main.errors.length === 0 && agents.errors.length === 0, 'Renderer exceptions found');
  await writeFile(join(artifacts, 'native.log'), output);
} catch (error) {
  results.push({ name: 'smoke run', passed: false, error: error.stack });
  console.error(error.stack);
  process.exitCode = 1;
} finally {
  await mkdir(artifacts, { recursive: true });
  await writeFile(join(artifacts, 'results.json'), JSON.stringify({ artifacts, results,
    notTested: ['real gateway streaming', 'approval races', 'terminal survival', 'dirty-buffer close cancellation', 'physical monitor DPI changes'] }, null, 2));
  for (const [index, connection] of connections.entries()) {
    await writeFile(join(artifacts, `cdp-${index}.json`), JSON.stringify({
      errors: connection.errors,
      logs: connection.logs,
    }, null, 2));
    connection.socket.close();
  }
  if (child?.pid && child.exitCode === null) {
    try { execFileSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' }); } catch {}
  }
  console.log(`Artifacts: ${artifacts}`);
}
