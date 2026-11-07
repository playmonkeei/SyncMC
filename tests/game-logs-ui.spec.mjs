// Run against a local ng serve. Playwright can be supplied through SYNCMC_PLAYWRIGHT_MODULE.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';

const { chromium } = await import(process.env.SYNCMC_PLAYWRIGHT_MODULE ?? 'playwright');
const browser = await chromium.launch({ channel: 'chrome', headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } });
const page = await context.newPage();
page.setDefaultTimeout(12000);
const errors = [];
page.on('pageerror', error => errors.push(error.message));

const timestamp = 1794043253;
const fixtureLogs = [
  { timestamp, stream: 'launcher', level: 'INFO', logger: 'SyncMC', line: 'Minecraft 1.21.1 gestartet.' },
  { timestamp, stream: 'stdout', level: 'INFO', logger: 'FabricLoader', thread: 'main', line: 'Loading 2 mods:\n\t- fabric-api\n\t- sodium' },
  { timestamp: timestamp + 1, stream: 'stdout', level: 'WARN', logger: 'Sodium', thread: 'Render thread', line: 'Missing sound for event: minecraft:item.goat_horn.play' },
  { timestamp: timestamp + 2, stream: 'stdout', level: 'ERROR', logger: 'Minecraft', thread: 'Render thread', line: 'Could not load shader\n\tat Game.render(Game.java:12)' },
  { timestamp: timestamp + 3, stream: 'stdout', level: 'FATAL', logger: 'Engine', thread: 'main', line: 'Could not initialize engine' },
  { timestamp: timestamp + 4, stream: 'stderr', level: 'UNKNOWN', line: 'UNSUPPORTED (log once): gldCopyBufferSubData: NEEDS IMPLEMENTATION' },
];

await page.addInitScript(({ logs }) => {
  const callbacks = new Map();
  const events = new Map();
  let nextId = 1;
  let currentLogs = logs;
  window.__unexpectedLogCommands = [];
  localStorage.setItem('syncmc.workspace.v1', JSON.stringify({ version: 1, selectedId: 'qa-logs', instances: [
    { id: 'qa-logs', name: 'Log QA', mcVersion: '1.21.1', loader: 'vanilla', cover: 'vanilla', contents: [], servers: [] },
  ] }));
  localStorage.setItem('syncmc.device.v1', JSON.stringify({ memoryMb: 4096, fullscreen: false, deviceName: 'Log QA' }));
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event, id) => events.delete(id) };
  window.__TAURI_INTERNALS__ = {
    transformCallback: callback => { const id = nextId++; callbacks.set(id, callback); return id; },
    invoke: async (command, args = {}) => {
      switch (command) {
        case 'plugin:event|listen': { const id = nextId++; events.set(id, args); return id; }
        case 'plugin:event|unlisten': return;
        case 'runtime_info': case 'validate_java': return { os: 'macos', arch: 'aarch64', javaAvailable: true, javaVersion: 'OpenJDK 21', javaMajor: 21, javaPath: '/qa/java/bin/java' };
        case 'get_minecraft_account': case 'get_minecraft_installation': return null;
        case 'curseforge_status': return { configured: false };
        case 'game_status': return { running: false, exitCode: 0 };
        case 'get_game_logs': return structuredClone(currentLogs);
        default: window.__unexpectedLogCommands.push(command); throw new Error(`Unexpected log QA command: ${command}`);
      }
    },
  };
  window.__replaceGameLogs = logs => { currentLogs = logs; };
  window.__emitGameLog = log => {
    currentLogs.push(log);
    for (const [id, listener] of events) {
      if (listener.event === 'minecraft-game-log') callbacks.get(listener.handler)?.({ event: listener.event, id, payload: log });
    }
  };
  Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
    writeText: async text => { window.__copiedGameLogs = text; },
  } });
}, { logs: fixtureLogs });

async function openLogs() {
  await page.getByRole('button', { name: 'Spielen', exact: true }).click();
  await page.getByRole('button', { name: 'Spielprotokoll anzeigen', exact: true }).click();
  await page.getByRole('heading', { name: 'Minecraft-Spielprotokoll.' }).waitFor();
}

async function expectRows(count) {
  await page.waitForFunction(expected => document.querySelectorAll('.log-entry').length === expected, count);
  assert.equal(await page.locator('.log-entry').count(), count);
}

try {
  await page.goto(process.env.SYNCMC_TEST_URL ?? 'http://127.0.0.1:4200/');
  await openLogs();
  const rows = page.locator('.log-entry');
  const query = page.getByRole('searchbox', { name: 'Protokoll durchsuchen', exact: true });
  const follow = page.getByRole('checkbox', { name: 'Automatisch scrollen', exact: true });
  const panel = page.locator('.game-log');
  await page.waitForFunction(() => document.querySelectorAll('.log-entry').length === 6);
  assert.doesNotMatch(await panel.innerText(), /log4j:|<!\[CDATA\[/);
  assert.ok((await rows.nth(1).locator('.log-message').textContent()).includes('\n\t- fabric-api'));
  assert.equal(await rows.last().locator('.log-level').innerText(), 'STDERR');
  if (process.env.SYNCMC_LOG_SCREENSHOT) {
    const output = process.env.SYNCMC_LOG_SCREENSHOT;
    await mkdir(output.slice(0, output.lastIndexOf('/')), { recursive: true });
    await page.screenshot({ path: output.replace(/\.png$/u, '-desktop.png'), fullPage: true });
  }

  await page.getByRole('button', { name: 'Warnungen', exact: true }).click();
  await expectRows(1);
  assert.match(await panel.innerText(), /Missing sound/);
  await query.fill(' RENDER THREAD ');
  await expectRows(1);
  await query.fill('shader');
  await page.getByText('Keine passenden Meldungen.', { exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Ansicht kopieren', exact: true }).isDisabled(), true);
  await page.getByRole('button', { name: 'Fehler', exact: true }).click();
  await expectRows(1);
  await query.fill('');
  await expectRows(2);
  await page.getByRole('button', { name: 'Ansicht kopieren', exact: true }).click();
  const copied = await page.evaluate(() => window.__copiedGameLogs);
  assert.match(copied, /2026-11-07T.*Z.*\[ERROR\].*\[Minecraft\].*\[Render thread\]/);
  assert.match(copied, /\n    \tat Game\.render\(Game\.java:12\)/);
  assert.match(copied, /\[FATAL\]/);
  assert.doesNotMatch(copied, /Minecraft 1\.21\.1 gestartet|Missing sound|UNSUPPORTED/);

  await page.getByRole('button', { name: 'Alle', exact: true }).click();
  await query.fill('sodium');
  await expectRows(2);
  await query.fill('');
  assert.equal(await follow.isChecked(), true);
  await page.evaluate(({ timestamp }) => {
    for (let index = 0; index < 70; index++) window.__emitGameLog({ timestamp: timestamp + index + 5, stream: 'stdout', level: 'INFO', logger: 'Minecraft', thread: 'Render thread', line: `Live message ${index}` });
  }, { timestamp });
  await page.waitForFunction(() => document.querySelectorAll('.log-entry').length === 76);
  await page.waitForFunction(() => { const panel = document.querySelector('.game-log'); return panel.scrollHeight - panel.scrollTop - panel.clientHeight <= 2; });
  await panel.evaluate(element => { element.scrollTop = 0; element.dispatchEvent(new Event('scroll')); });
  await page.waitForFunction(() => !document.querySelector('.log-summary input[type="checkbox"]').checked && document.querySelector('.game-log').scrollTop < 2);
  assert.equal(await follow.isChecked(), false, 'scrolling up pauses following new messages');
  await page.evaluate(({ timestamp }) => window.__emitGameLog({ timestamp, stream: 'stdout', level: 'WARN', logger: 'Minecraft', line: 'Live warning while reading earlier output' }), { timestamp });
  await page.getByText('Live warning while reading earlier output', { exact: true }).waitFor({ state: 'attached' });
  assert.ok(await panel.evaluate(element => element.scrollTop < 2), 'live output does not move the reading position');
  await follow.check();
  await page.waitForFunction(() => { const panel = document.querySelector('.game-log'); return panel.scrollHeight - panel.scrollTop - panel.clientHeight <= 2; });
  await page.evaluate(({ timestamp }) => window.__emitGameLog({ timestamp, stream: 'stdout', level: 'INFO', logger: 'very.long.package.name.for.a.minecraft.mod.without.spaces', thread: 'Render thread', line: 'Long detail: ' + 'a'.repeat(400) }), { timestamp });

  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForFunction(() => document.querySelector('.primary-modal').getBoundingClientRect().width <= window.innerWidth);
  const dimensions = await page.evaluate(() => {
    const modal = document.querySelector('.primary-modal').getBoundingClientRect();
    const panel = document.querySelector('.game-log');
    return { width: innerWidth, modalLeft: modal.left, modalRight: modal.right, pageWidth: document.documentElement.scrollWidth, panelWidth: panel.clientWidth, panelContentWidth: panel.scrollWidth };
  });
  assert.ok(dimensions.modalLeft >= 0 && dimensions.modalRight <= dimensions.width, 'the log dialog fits a narrow screen');
  assert.ok(dimensions.pageWidth <= dimensions.width, 'the page has no horizontal overflow');
  assert.ok(dimensions.panelContentWidth <= dimensions.panelWidth + 1, 'long messages and sources wrap within the console');
  if (process.env.SYNCMC_LOG_SCREENSHOT) {
    const output = process.env.SYNCMC_LOG_SCREENSHOT;
    await mkdir(output.slice(0, output.lastIndexOf('/')), { recursive: true });
    await page.screenshot({ path: output, fullPage: true });
  }

  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.evaluate(() => window.__replaceGameLogs([]));
  await page.setViewportSize({ width: 1440, height: 1100 });
  await openLogs();
  await page.getByText('Noch keine Meldungen. Starte Minecraft, um das Protokoll zu sehen.', { exact: true }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Ansicht kopieren', exact: true }).isDisabled(), true);
  assert.deepEqual(await page.evaluate(() => window.__unexpectedLogCommands), []);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ passed: true, checks: ['structured messages', 'severity filters', 'metadata search', 'filtered clipboard', 'multiline details', 'unclassified stderr', 'live output', 'follow and manual scrolling', 'narrow layout', 'empty state'], pageErrors: errors }, null, 2));
} finally {
  await browser.close();
}
