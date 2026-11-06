// Run against a local ng serve. Set SYNCMC_PLAYWRIGHT_MODULE if Playwright is installed elsewhere.
import assert from 'node:assert/strict';
import { mkdir } from 'node:fs/promises';
const { chromium } = await import(process.env.SYNCMC_PLAYWRIGHT_MODULE ?? 'playwright');
const browser = await chromium.launch({ channel: 'chrome', headless: true });
const context = await browser.newContext({ viewport: { width: 1440, height: 1100 } });
const page = await context.newPage();
page.setDefaultTimeout(12000);
const errors = [];
const defaultMicrosoftClientId = 'a653da4d-bfbd-4c80-83d4-5fb1a5272475';
const screenshotOutput = process.env.SYNCMC_TEST_SCREENSHOT;
const screenshotDirectory = screenshotOutput?.slice(0, screenshotOutput.lastIndexOf('/'));
if (screenshotDirectory) await mkdir(screenshotDirectory, { recursive: true });
page.on('pageerror', error => errors.push(error.message));
let nativeDialogCount = 0;
page.on('dialog', async dialog => { nativeDialogCount++; await dialog.dismiss(); });
const appConfirmation = page.getByRole('alertdialog');
async function answerConfirmation(actionLabel, accept = true, messagePattern) {
  await appConfirmation.waitFor();
  if (messagePattern) assert.match(await appConfirmation.innerText(), messagePattern);
  await appConfirmation.getByRole('button', { name: accept ? actionLabel : 'Abbrechen', exact: true }).click();
  await appConfirmation.waitFor({ state: 'hidden' });
}
await page.addInitScript(() => {
  const callbacks = new Map(); const events = new Map(); let nextId = 1;
  let account = null; let report = null; let game = { running: false }; let logs = []; let managedJava = false; let curseforgeConfigured = false;
  let installedProfile = null; const backups = []; const backupProfiles = new Map();
  window.__testCalls = [];
  const mod = { id: 'qa-root', projectId: 'QaRoot12', versionId: 'QaVersion1', name: 'QA Mod', version: '1.0.0',
    filename: 'qa-mod.jar', kind: 'mod', enabled: true, sha512: 'f'.repeat(128), source: 'modrinth', status: 'planned' };
  if (!localStorage.getItem('syncmc.workspace.v1')) {
    localStorage.setItem('syncmc.workspace.v1', JSON.stringify({ version: 1, selectedId: 'qa-fabric', instances: [
      { id: 'qa-fabric', name: 'QA Fabric', mcVersion: '1.21.1', loader: 'fabric', cover: 'redwood', contents: [mod], servers: [] },
      { id: 'qa-local-only', name: 'Nur lokal', mcVersion: '1.21.1', loader: 'fabric', cover: 'vanilla', contents: [], servers: [] },
    ] }));
    localStorage.setItem('syncmc.device.v1', JSON.stringify({ memoryMb: 4096, fullscreen: false, deviceName: 'QA device' }));
  }
  const emit = (event, payload) => { for (const [id, listener] of events) if (listener.event === event) callbacks.get(listener.handler)?.({ event, id, payload }); };
  const runtime = (args = {}) => ({ os: 'macos', arch: 'aarch64', javaAvailable: true,
    javaVersion: args.javaPath ? 'OpenJDK 17.0.9' : 'OpenJDK 21.0.9', javaMajor: args.javaPath ? 17 : 21,
    javaArch: 'aarch64', javaPath: args.javaPath || (managedJava ? '/qa/managed/java-21/bin/java' : '/qa/java/bin/java') });
  const backup = (profile, reason) => {
    const summary = { id: `qa-backup-${backups.length + 1}`, instanceId: profile.id, createdAt: 1793850000 + backups.length,
      reason, fileCount: 4, totalBytes: 2048, mcVersion: profile.mcVersion, loader: profile.loader,
      profileName: profile.name, contentsCount: profile.contents.length, verifiedContent: true,
      containedSections: ['profile', 'mods', 'config', 'worlds'] };
    backups.unshift(summary); backupProfiles.set(summary.id, structuredClone(profile)); return summary;
  };
  window.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event, id) => events.delete(id) };
  window.__TAURI_INTERNALS__ = {
    transformCallback: callback => { const id = nextId++; callbacks.set(id, callback); return id; },
    invoke: async (command, args = {}) => {
      window.__testCalls.push({ command, args });
      switch (command) {
        case 'plugin:event|listen': { const id = nextId++; events.set(id, args); return id; }
        case 'plugin:event|unlisten': return;
        case 'curseforge_status': return { configured: curseforgeConfigured };
        case 'set_curseforge_key': if (args.key !== 'qa-approved-cf-key') throw new Error('Der CurseForge-Schlüssel wurde abgelehnt.'); curseforgeConfigured = true; return { configured: true };
        case 'clear_curseforge_key': curseforgeConfigured = false; return { configured: false };
        case 'open_curseforge_application': return;
        case 'search_curseforge': {
          const grouped = window.__testGroupedSearch === true;
          if (!curseforgeConfigured) throw new Error('Kein CurseForge-Schlüssel eingerichtet.');
          if (window.__testCfSearchError) throw new Error('Mock CurseForge-Suche nicht verfügbar.');
          if (window.__testDelayCfSearch === true || window.__testDelayCfSearch === args.query) {
            await new Promise(resolve => { window.__testReleaseCfSearch = resolve; });
          }
          if (grouped) return { hits: [{ project_id: '34567890', title: 'Sodium', description: 'CF Sodium fixture', icon_url: '', author: 'CF author', downloads: 100, project_type: args.projectType, source: 'curseforge', source_url: 'https://github.com/CaffeineMC/sodium' }] };
          return { hits: [{ project_id: '12345678', title: args.query ? `CF QA Mod · ${args.query}` : 'CF QA Mod', description: 'Mock CurseForge project', icon_url: '',
            author: 'CF author', downloads: 100, project_type: args.projectType, source: 'curseforge' }] };
        }
        case 'get_curseforge_versions': {
          if (!curseforgeConfigured) throw new Error('Kein CurseForge-Schlüssel eingerichtet.');
          if (window.__testDelayCfVersions) await new Promise(resolve => { window.__testReleaseCfVersions = resolve; });
          if (window.__testCFBlocked) throw new Error('Die Projektverantwortlichen erlauben keine automatischen Downloads über CurseForge.');
          const file = { filename: 'cf-qa-mod.jar', primary: true, hashes: { sha1: 'b'.repeat(40) }, url: 'https://edge.forgecdn.net/files/1/2/cf-qa-mod.jar' };
          const base = { source: 'curseforge', project_id: args.projectId, project_type: args.projectType, game_versions: ['1.21.1'],
            loaders: args.projectType === 'mod' ? ['fabric'] : [], dependencies: [], files: [file], name: 'CF QA Mod' };
          return [{ ...base, id: '2001', version_number: '1.0.0', version_type: 'release', date_published: '2025-01-01T00:00:00Z' },
            { ...base, id: '2002', version_number: '1.1.0', version_type: 'release', date_published: '2026-01-01T00:00:00Z', files: [{ ...file, filename: 'cf-qa-mod-1.1.jar', hashes: { sha1: 'c'.repeat(40) } }] }];
        }
        case 'search_modrinth': {
          const grouped = window.__testGroupedSearch === true;
          if (window.__testDelayMrSearch === true || window.__testDelayMrSearch === args.query) {
            await new Promise(resolve => { window.__testReleaseMrSearch = resolve; });
          }
          if (grouped) return { hits: [{ project_id: 'SodiumQA', title: 'Sodium', description: 'MR Sodium fixture', icon_url: '', author: 'MR author', downloads: 42, project_type: args.projectType }] };
          return { hits: [{ project_id: '12345678', title: args.query ? `MR Numeric QA · ${args.query}` : 'MR Numeric QA', description: 'Mock Modrinth project',
            icon_url: '', author: 'MR author', downloads: 42, project_type: args.projectType }] };
        }
        case 'get_modrinth_project_links': {
          const projectIds = [...args.projectIds];
          if (window.__testDelayMrLinks) await new Promise(resolve => { window.__testReleaseMrLinks = resolve; });
          if (window.__testMrLinksError) throw new Error('Mock Projektreferenz nicht verfügbar.');
          return projectIds.map(project_id => ({ project_id, source_url: project_id === 'SodiumQA' ? 'https://github.com/CaffeineMC/sodium.git/' : null }));
        }
        case 'runtime_info': return runtime(args);
        case 'validate_java': if (args.javaPath) throw new Error('Java 21 benötigt, manueller Pfad zeigt auf Java 17.'); return runtime(args);
        case 'install_java_runtime':
          emit('java-install-progress', { major: args.requiredMajor, stage: 'downloading', downloadedBytes: 50, totalBytes: 100 });
          await new Promise(resolve => setTimeout(resolve, 30)); managedJava = true;
          emit('java-install-progress', { major: args.requiredMajor, stage: 'complete', downloadedBytes: 100, totalBytes: 100 }); return runtime();
        case 'list_instance_backups': return backups.filter(backup => backup.instanceId === args.instanceId);
        case 'create_instance_backup': return backup(args.profile, args.reason ?? 'manual');
        case 'restore_instance_backup': {
          const saved = structuredClone(backupProfiles.get(args.backupId));
          const target = backups.find(backup => backup.id === args.backupId);
          const rescueBackup = backup(installedProfile ?? saved, 'before-restore'); installedProfile = saved;
          return { backup: target, rescueBackup, profile: saved, contents: saved.contents, requiresInstallation: !!window.__testRestoreRequiresInstall, requiresContentInstall: !!window.__testRestoreRequiresInstall };
        }
        case 'get_modrinth_versions': {
          const file = { filename: args.projectId === 'SodiumQA' ? 'sodium-qa.jar' : 'qa-mod-1.1.jar', primary: true, hashes: { sha512: 'e'.repeat(128) }, url: 'https://cdn.modrinth.com/qa.jar' };
          const base = { project_id: args.projectId, game_versions: ['1.21.1'], loaders: ['fabric'], dependencies: [], files: [file], name: 'QA Mod' };
          return [{ ...base, id: 'QaVersion1', version_number: '1.0.0', version_type: 'release', date_published: '2025-01-01T00:00:00Z' },
            { ...base, id: 'QaVersion2', version_number: '1.1.0', version_type: 'release', date_published: '2026-01-01T00:00:00Z' },
            { ...base, id: 'QaBeta3', version_number: '2.0-beta', version_type: 'beta', date_published: '2026-03-01T00:00:00Z' },
            { ...base, id: 'QaForge4', version_number: '3.0', version_type: 'release', loaders: ['forge'], date_published: '2026-04-01T00:00:00Z' }];
        }
        case 'save_profile_export':
          await new Promise(resolve => setTimeout(resolve, 30));
          if (window.__testExportFail) throw new Error('Exportziel konnte nicht geschrieben werden.');
          return { path: `/qa/exports/${args.filename}`, bytes: new TextEncoder().encode(args.contents).length };
        case 'open_profile_exports': return;
        case 'get_minecraft_account': return account;
        case 'get_minecraft_installation': return report;
        case 'get_fabric_loaders': return [{ version: '0.18.0', stable: true }, { version: '0.17.3', stable: true }];
        case 'game_status': return game;
        case 'get_game_logs': return logs;
        case 'get_minecraft_versions': return { latest: { release: '1.21.1', snapshot: '1.21.1' }, versions: [
          { id: '1.21.1', type: 'release', releaseTime: '2024-08-08T00:00:00Z' }] };
        case 'install_minecraft':
          emit('minecraft-install-progress', { instanceId: args.instanceId, stage: 'downloading', completedFiles: 2, totalFiles: 3, downloadedBytes: 10 });
          await new Promise(resolve => setTimeout(resolve, 30));
          report = { instanceId: args.instanceId, versionId: args.versionId, loader: args.loader,
            loaderVersion: args.loaderVersion, directory: '/qa/minecraft/instances/qa-fabric', javaMajorVersion: 21,
            filesVerified: 3, bytesVerified: 10, nativeArchives: 1, installedAt: 1793850000 };
          return report;
        case 'install_instance_content': {
          emit('content-install-progress', { instanceId: args.instanceId, completedFiles: 1, totalFiles: 2 });
          await new Promise(resolve => setTimeout(resolve, 30));
          const before = installedProfile && JSON.stringify(installedProfile.contents) !== JSON.stringify(args.contents)
            ? backup(installedProfile, 'before-content-change') : undefined;
          const contents = [...args.contents.filter(content => !content.dependency).map(content => ({ ...content, ...(content.source === 'curseforge' ? { sha512: 'd'.repeat(128) } : {}), status: content.enabled ? 'downloaded' : 'planned' })),
            ...(args.contents.some(content => content.enabled && !content.dependency) ? [{ ...mod, id: 'qa-dependency', projectId: 'QaDep123', versionId: 'QaDepVer',
              name: 'QA Required API', filename: 'qa-api.jar', status: 'downloaded', dependency: true }] : [])];
          installedProfile = { ...structuredClone(args.profile), contents };
          return { contents, filesVerified: 2, dependencyCount: 1, backup: before };
        }
        case 'begin_microsoft_login': return { flowId: 'qa-flow', userCode: 'TEST-CODE', verificationUri: 'https://www.microsoft.com/link', expiresIn: 300, interval: 5 };
        case 'poll_microsoft_login': account = { name: 'QA Player', uuid: 'f'.repeat(32) }; return { status: 'complete', account };
        case 'cancel_microsoft_login': case 'open_microsoft_login': case 'open_microsoft_registration': return;
        case 'logout_minecraft_account': account = null; return;
        case 'launch_minecraft':
          game = { running: true, instanceId: args.instanceId, pid: 1234, startedAt: 1793850000 };
          logs = [{ timestamp: 1793850000, stream: 'stdout', line: 'Fabric initialized for QA' }];
          emit('minecraft-game-status', game); emit('minecraft-game-log', logs[0]); return game;
        case 'stop_minecraft': if (window.__testKeepRunning && !args.force) return; game = { ...game, running: false, exitCode: 0 }; emit('minecraft-game-status', game); return;
        default: throw new Error(`Unexpected QA command: ${command}`);
      }
    },
  };
});
try {
  await page.goto(process.env.SYNCMC_TEST_URL ?? 'http://127.0.0.1:4201/');
  await page.getByRole('button', { name: 'Spielen', exact: true }).click();
  await page.getByRole('button', { name: 'Minecraft & Inhalte installieren', exact: true }).click();
  await page.getByText('Geprüfte Spieldateien').waitFor();
  await page.getByRole('button', { name: 'Installation & Inhalte prüfen', exact: true }).waitFor({ state: 'visible' });
  assert.equal(await page.getByRole('button', { name: 'Minecraft starten', exact: true }).isDisabled(), true, 'starting requires authentication');
  await page.getByRole('button', { name: 'Java 21 automatisch einrichten', exact: true }).click();
  await page.getByText('Java 21 ist eingerichtet und wird automatisch verwendet.', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__testCalls.find(call => call.command === 'install_java_runtime').args.requiredMajor), 21);
  await page.getByRole('button', { name: 'Anmelden', exact: true }).click();
  assert.equal(await page.getByRole('button', { name: 'Mit Microsoft anmelden', exact: true }).isEnabled(), true, 'old device settings use the SyncMC app without setup');
  const clientIdInput = page.getByRole('textbox', { name: 'Microsoft-App-Client-ID', exact: true });
  assert.equal(await clientIdInput.isVisible(), false, 'client override stays in advanced settings');
  await page.getByText('Erweitert: Microsoft-App-Client-ID', { exact: true }).click();
  assert.equal(await clientIdInput.inputValue(), defaultMicrosoftClientId, 'missing legacy client ID migrates to SyncMC default');
  await clientIdInput.fill('11111111-1111-1111-1111-111111111111');
  await clientIdInput.press('Tab');
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.device.v1')).microsoftClientId), '11111111-1111-1111-1111-111111111111', 'explicit local app override remains supported');
  await clientIdInput.fill('');
  await clientIdInput.press('Tab');
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.device.v1')).microsoftClientId), defaultMicrosoftClientId, 'clearing override restores SyncMC default');
  const inFlightClientEdit = await page.evaluate(async () => {
    const input = document.querySelector('[aria-label="Microsoft-App-Client-ID"]');
    input.focus();
    input.value = '11111111-1111-1111-1111-111111111111';
    input.dispatchEvent(new Event('input', { bubbles: true })); input.dispatchEvent(new Event('change', { bubbles: true }));
    input.value = ''; input.dispatchEvent(new Event('input', { bubbles: true }));
    await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame);
    const beforeBlur = input.value;
    input.dispatchEvent(new Event('change', { bubbles: true })); input.blur();
    return { beforeBlur, saved: JSON.parse(localStorage.getItem('syncmc.device.v1')).microsoftClientId };
  });
  assert.equal(inFlightClientEdit.beforeBlur, '', 'a pending model render never overwrites the current input draft');
  assert.equal(inFlightClientEdit.saved, defaultMicrosoftClientId, 'clearing a draft before the next render still restores the app default');

  await page.getByText('Erweitert: Microsoft-App-Client-ID', { exact: true }).click();
  if (screenshotDirectory) await page.screenshot({ path: `${screenshotDirectory}/account-setup.png`, fullPage: true });
  await page.getByRole('button', { name: 'Mit Microsoft anmelden', exact: true }).click();
  await page.getByRole('button', { name: 'Anmeldecode kopieren', exact: true }).waitFor();
  await page.getByText('https://www.microsoft.com/link', { exact: true }).waitFor();
  assert.equal(await page.evaluate(() => window.__testCalls.find(call => call.command === 'begin_microsoft_login').args.clientId), defaultMicrosoftClientId, 'normal sign-in uses the public SyncMC app ID');
  if (screenshotDirectory) await page.screenshot({ path: `${screenshotDirectory}/device-code.png`, fullPage: true });
  await page.getByText('QA Player', { exact: true }).last().waitFor({ timeout: 15000 });
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: 'Spielen', exact: true }).click();
  const start = page.getByRole('button', { name: 'Minecraft starten', exact: true });
  await start.waitFor();
  await start.click();
  await page.getByRole('button', { name: 'Laufendes Spiel beenden', exact: true }).waitFor();
  const launchCalls = await page.evaluate(() => window.__testCalls.filter(call => call.command === 'launch_minecraft'));
  assert.equal(launchCalls.length, 1);
  assert.equal(launchCalls[0].args.loaderVersion, '0.18.0', 'resolved loader is pinned');
  assert.equal(launchCalls[0].args.memoryMb, 4096);
  assert.equal(launchCalls[0].args.contents.filter(content => content.dependency).length, 1, 'required dependency reaches launch');
  await page.getByRole('button', { name: 'Spielprotokoll anzeigen', exact: true }).click();
  await page.getByText('Fabric initialized for QA', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: 'Spiel beenden', exact: true }).click();
  await page.getByRole('button', { name: 'Spielen', exact: true }).click();
  await page.getByText('Letzter Spielprozess beendet · Exit-Code 0').waitFor();
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: 'Instanz öffnen', exact: true }).click();
  await page.getByRole('switch', { name: 'QA Mod aktivieren', exact: true }).click();
  await page.getByRole('button', { name: 'Spielen', exact: true }).click();
  await page.getByRole('button', { name: 'Minecraft starten', exact: true }).click();
  await page.getByRole('button', { name: 'Laufendes Spiel beenden', exact: true }).waitFor();
  const calls = await page.evaluate(() => window.__testCalls);
  assert.equal(calls.filter(call => call.command === 'launch_minecraft').at(-1).args.contents.some(content => content.dependency), false, 'disabling root removes unneeded auto dependencies before launch');
  assert.equal(calls.filter(call => call.command === 'launch_minecraft').at(-1).args.contents[0].enabled, false);
  await page.evaluate(() => { window.__testKeepRunning = true; });
  await page.getByRole('button', { name: 'Laufendes Spiel beenden', exact: true }).click();
  const forceStop = page.getByRole('dialog').getByRole('button', { name: 'Beenden erzwingen', exact: true });
  await forceStop.waitFor();
  await forceStop.click();
  await appConfirmation.waitFor();
  assert.match(await appConfirmation.innerText(), /Nicht gespeicherte Weltänderungen/);
  assert.equal(await page.locator('.app-shell').evaluate(element => element.inert), true);
  await page.waitForFunction(() => document.activeElement?.classList.contains('confirmation-cancel'));
  await page.keyboard.press('Shift+Tab');
  assert.equal(await appConfirmation.getByRole('button', { name: 'Minecraft sofort beenden', exact: true }).evaluate(element => element === document.activeElement), true, 'focus remains in confirmation');
  await page.keyboard.press('Escape');
  await appConfirmation.waitFor({ state: 'hidden' });
  await page.getByRole('dialog').getByRole('heading', { name: 'Minecraft starten.', exact: true }).waitFor();
  await page.waitForFunction(() => document.activeElement?.textContent?.trim() === 'Beenden erzwingen');
  await forceStop.click();
  await appConfirmation.waitFor();
  await page.locator('.confirmation-backdrop').click({ position: { x: 10, y: 10 } });
  await appConfirmation.waitFor({ state: 'hidden' });
  await forceStop.click();
  await answerConfirmation('Minecraft sofort beenden', false);
  assert.equal((await page.evaluate(() => window.__testCalls.filter(call => call.command === 'stop_minecraft' && call.args.force))).length, 0, 'cancel, Escape, and backdrop never force-stop the game');
  await forceStop.click();
  await answerConfirmation('Minecraft sofort beenden');
  await page.getByRole('button', { name: 'Minecraft starten', exact: true }).waitFor();
  assert.equal((await page.evaluate(() => window.__testCalls.filter(call => call.command === 'stop_minecraft' && call.args.force))).length, 1, 'forced stop requires explicit confirmation');
  // Each root mod is reviewed separately; newer betas and other loaders must be excluded.
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByRole('switch', { name: 'QA Mod aktivieren', exact: true }).click();
  await page.getByRole('button', { name: 'Mod-Updates prüfen', exact: true }).click();
  await page.getByRole('button', { name: 'Version prüfen', exact: true }).click();
  await page.getByText('1.1.0 · Release', { exact: true }).waitFor();
  assert.equal(await page.getByRole('dialog').getByText('2.0-beta', { exact: true }).count(), 0);
  await page.getByRole('button', { name: 'Diese Version installieren', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByText('QA Mod wurde auf 1.1.0 aktualisiert.', { exact: false }).waitFor();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances[0].contents[0].versionId), 'QaVersion2');
  const installCalls = await page.evaluate(() => window.__testCalls.filter(call => call.command === 'install_instance_content'));
  assert.equal(installCalls.at(-1).args.profile.contents[0].versionId, 'QaVersion2', 'new complete profile reaches native installer');

  // Native instance backups include files and restore matching old profile pins.
  await page.getByRole('button', { name: 'Sicherungen', exact: true }).click();
  await page.getByRole('button', { name: 'Sicherung erstellen', exact: true }).click();
  await page.getByText('„QA Fabric“ wurde mit 4 Dateien gesichert.', { exact: true }).waitFor();
  const beforeUpdateBackup = page.locator('.backup-card').filter({ hasText: 'Vor Inhaltsänderung' }).first();
  const restoreInstance = beforeUpdateBackup.getByRole('button', { name: /Sicherung .* wiederherstellen/ });
  await restoreInstance.click();
  await answerConfirmation('Instanz wiederherstellen', false, /Rettungssicherung/);
  assert.equal(await page.evaluate(() => window.__testCalls.filter(call => call.command === 'restore_instance_backup').length), 0);
  await page.evaluate(() => { window.__testRestoreRequiresInstall = true; });
  await restoreInstance.click();
  await answerConfirmation('Instanz wiederherstellen', true, /Profil, Mods, Konfiguration und Welten/);
  await page.getByText(/wiederhergestellt. Der vorherige Stand bleibt als Rettungssicherung erhalten/).waitFor();
  await page.getByText(/Minecraft und Inhalte müssen vor dem nächsten Spielstart erneut installiert oder geprüft werden/).waitFor();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances[0].contents[0].versionId), 'QaVersion1', 'native restore updates complete frontend profile');

  // Installing managed Java must keep an explicit manual path until the user chooses automation.
  await page.getByRole('button', { name: 'Einstellungen', exact: true }).first().click();
  const javaInput = page.getByRole('textbox', { name: 'Pfad zur Java-Datei', exact: true });
  await javaInput.fill('/qa/manual/java'); await javaInput.press('Tab');
  await page.getByText('Java 21 benötigt, manueller Pfad zeigt auf Java 17.', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Java 21 automatisch einrichten', exact: true }).click();
  await page.getByText(/Dein manuell gewählter Java-Pfad bleibt aktiv/).waitFor();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.device.v1')).javaPath), '/qa/manual/java');
  await page.getByRole('button', { name: 'Automatische Java-Auswahl verwenden', exact: true }).click();
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.device.v1')).javaPath), '');

  // A file merge requires an explicit decision for every conflicting ID and preserves local-only profiles.
  await page.getByRole('button', { name: 'Geräte & Sync', exact: true }).click();
  const deviceBeforeSync = await page.evaluate(() => localStorage.getItem('syncmc.device.v1'));
  const incoming = await page.evaluate(() => {
    const profile = structuredClone(JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances[0]);
    profile.name = 'QA Importiert'; profile.contents[0].versionId = 'QaImported3'; profile.contents[0].version = '3.0.0'; profile.contents[0].status = 'downloaded';
    return { format: 'syncmc-manifest', version: 1, instances: [profile,
      { id: 'qa-incoming', name: 'Aus Datei', mcVersion: '1.21.1', loader: 'vanilla', cover: 'vanilla', contents: [], servers: [] }] };
  });
  const inputFile = page.locator('input[type=file]');
  await inputFile.setInputFiles({ name: 'syncmc-incoming.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(incoming)) });
  const applySync = page.getByRole('button', { name: 'Profilabgleich übernehmen', exact: true });
  assert.equal(await applySync.isDisabled(), true, 'a same-ID conflict has no default decision');
  await page.getByRole('radio', { name: 'Importiertes Profil QA Importiert übernehmen', exact: true }).check();
  await applySync.click(); await page.getByRole('dialog').waitFor({ state: 'hidden' });
  const merged = await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances);
  assert.equal(merged.some(instance => instance.id === 'qa-local-only'), true);
  assert.equal(merged.some(instance => instance.id === 'qa-incoming'), true);
  assert.equal(merged.find(instance => instance.id === 'qa-fabric').contents[0].status, 'planned');
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.device.v1')), deviceBeforeSync);

  // Reload keeps snapshots; exporting and restoring one must work without a backend session.
  await page.reload();
  const profileRows = page.locator('.profile-backup-row');
  await profileRows.first().waitFor();
  const profileBackupCount = await profileRows.count();
  await profileRows.first().getByRole('button', { name: /Profilsicherung .* exportieren/ }).click();
  await page.locator('.export-feedback code').waitFor();
  assert.match(await page.locator('.export-feedback code').innerText(), /^\/qa\/exports\/syncmc-profilsicherung-/);
  const exportCall = await page.evaluate(() => window.__testCalls.find(call => call.command === 'save_profile_export'));
  assert.ok(JSON.parse(exportCall.args.contents).instances.length >= 2, 'native export receives the real saved profile data');
  await page.getByRole('button', { name: 'Exportordner öffnen', exact: true }).click();
  assert.equal(await page.evaluate(() => window.__testCalls.filter(call => call.command === 'open_profile_exports').length), 1);
  await page.evaluate(() => { window.__testExportFail = true; });
  await profileRows.first().getByRole('button', { name: /Profilsicherung .* exportieren/ }).click();
  await page.getByRole('alert').filter({ hasText: 'Exportziel konnte nicht geschrieben werden.' }).first().waitFor();
  assert.equal(await page.locator('.export-feedback code').count(), 0, 'failed native export never claims a saved path');
  await page.evaluate(() => { window.__testExportFail = false; });
  const workspaceBeforeProfileRestore = await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1'));
  await profileRows.first().getByRole('button', { name: /Profilsicherung .* wiederherstellen/ }).click();
  await answerConfirmation('Profile wiederherstellen', false, /Rettungssicherung/);
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1')), workspaceBeforeProfileRestore, 'cancel leaves profiles unchanged');
  assert.equal(await profileRows.count(), profileBackupCount);
  await profileRows.first().getByRole('button', { name: /Profilsicherung .* wiederherstellen/ }).click();
  await answerConfirmation('Profile wiederherstellen');
  await page.getByText('Instanzprofile wiederhergestellt. Eine Rettungssicherung der vorherigen Profile bleibt erhalten.', { exact: true }).waitFor();
  assert.equal(await profileRows.count(), profileBackupCount + 1);
  const restoredProfiles = await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances);
  assert.equal(restoredProfiles.some(instance => instance.id === 'qa-local-only'), true);
  assert.equal(restoredProfiles.find(instance => instance.id === 'qa-fabric').name, 'QA Fabric');
  assert.equal(restoredProfiles.flatMap(instance => instance.contents).every(content => content.status === 'planned'), true);
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.device.v1')), deviceBeforeSync);

  // Deleting a local profile snapshot also uses the same accessible app confirmation.
  const beforeDeleteCount = await profileRows.count();
  await profileRows.last().getByRole('button', { name: /Profilsicherung .* löschen/ }).click();
  await answerConfirmation('Sicherung löschen', false);
  assert.equal(await profileRows.count(), beforeDeleteCount);
  await profileRows.last().getByRole('button', { name: /Profilsicherung .* löschen/ }).click();
  await answerConfirmation('Sicherung löschen');
  assert.equal(await profileRows.count(), beforeDeleteCount - 1);

  // A storage quota failure prevents the merge entirely.
  const beforeQuota = await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1'));
  await page.locator('input[type=file]').setInputFiles({ name: 'syncmc-quota.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(incoming)) });
  await page.getByRole('radio', { name: 'Importiertes Profil QA Importiert übernehmen', exact: true }).check();
  await page.evaluate(() => { window.__testOriginalSetItem = Storage.prototype.setItem; Storage.prototype.setItem = function(key, value) { if (key === 'syncmc.profile-backups.v1') throw new DOMException('quota', 'QuotaExceededError'); return window.__testOriginalSetItem.call(this, key, value); }; });
  await page.getByRole('button', { name: 'Profilabgleich übernehmen', exact: true }).click();
  await page.getByRole('dialog').getByRole('alert').filter({ hasText: /Die Profilsicherung konnte nicht lokal gespeichert werden/ }).waitFor();
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1')), beforeQuota, 'failed snapshot means no imported mutation');
  await page.evaluate(() => { Storage.prototype.setItem = window.__testOriginalSetItem; });
  await page.getByRole('button', { name: 'Abbrechen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  // CurseForge stays honest without a key and never sends credentials through a browser proxy.
  await page.getByRole('button', { name: 'Entdecken', exact: true }).click();
  async function chooseSource(label) {
    await page.getByRole('combobox', { name: 'Inhaltsquelle', exact: true }).click();
    await page.getByRole('option', { name: new RegExp(`^${label}`) }).click();
  }
  assert.match(await page.getByRole('combobox', { name: 'Inhaltsquelle', exact: true }).innerText(), /Alle Anbieter/, 'combined catalog is the default');
  await page.getByRole('textbox', { name: 'Alle Anbieter durchsuchen', exact: true }).waitFor();
  await page.getByRole('heading', { name: 'MR Numeric QA', exact: true }).waitFor();
  const allWithoutKeyMr = page.locator('.search-result').filter({ has: page.getByRole('heading', { name: 'MR Numeric QA', exact: true }) });
  assert.equal((await allWithoutKeyMr.locator('.provider-badge').innerText()).trim(), 'Modrinth');
  await page.locator('.catalog-source-note').filter({ hasText: /CurseForge/ }).waitFor();
  assert.equal(await page.getByRole('heading', { name: 'CF QA Mod', exact: true }).count(), 0);
  assert.equal(await page.evaluate(() => window.__testCalls.filter(call => call.command === 'search_curseforge').length), 0, 'all providers keeps Modrinth usable without issuing an unconfigured CF request');
  await chooseSource('CurseForge');
  await page.getByText(/CurseForge ist noch nicht eingerichtet/).waitFor();
  assert.equal(await page.evaluate(() => window.__testCalls.filter(call => call.command === 'search_curseforge').length), 0, 'no unconfigured CurseForge request is made');
  await page.getByRole('button', { name: 'CurseForge einrichten', exact: true }).click();
  const cfKeyInput = page.getByLabel('CurseForge API-Schlüssel', { exact: true });
  await cfKeyInput.fill('qa-invalid-cf-key');
  await page.getByRole('button', { name: 'Schlüssel prüfen & speichern', exact: true }).click();
  await page.getByRole('alert').filter({ hasText: 'Der CurseForge-Schlüssel wurde abgelehnt.' }).waitFor();
  assert.equal(await cfKeyInput.inputValue(), '', 'rejected keys do not remain in the password field');
  await cfKeyInput.fill('qa-approved-cf-key');
  await page.getByRole('button', { name: 'Schlüssel prüfen & speichern', exact: true }).click();
  await page.getByText('Der CurseForge-Schlüssel wurde geprüft und im Schlüsselbund dieses Geräts gespeichert.', { exact: true }).waitFor();
  assert.equal(await cfKeyInput.inputValue(), '', 'saved keys do not remain in the password field');

  // Combined search keeps same-ID projects separate and exposes each provider on the result itself.
  await page.getByRole('button', { name: 'Entdecken', exact: true }).click();
  await chooseSource('Alle Anbieter');
  const allMrResult = page.locator('.search-result').filter({ has: page.getByRole('heading', { name: 'MR Numeric QA', exact: true }) });
  const allCfResult = page.locator('.search-result').filter({ has: page.getByRole('heading', { name: 'CF QA Mod', exact: true }) });
  await allMrResult.waitFor(); await allCfResult.waitFor();
  assert.equal(await page.locator('.search-result').count(), 2, 'same project_id from two providers produces two visible hits');
  assert.equal((await allMrResult.locator('.provider-badge').innerText()).trim(), 'Modrinth');
  assert.equal((await allCfResult.locator('.provider-badge').innerText()).trim(), 'CurseForge');
  const beforeCombinedMr = await page.evaluate(() => window.__testCalls.length);
  await allMrResult.getByRole('heading', { name: 'MR Numeric QA', exact: true }).click();
  await page.getByText('qa-mod-1.1.jar', { exact: true }).waitFor();
  const combinedMrCalls = await page.evaluate(index => window.__testCalls.slice(index).filter(call => call.command.endsWith('_versions')), beforeCombinedMr);
  assert.ok(combinedMrCalls.some(call => call.command === 'get_modrinth_versions' && call.args.projectId === '12345678'));
  assert.equal(combinedMrCalls.some(call => call.command === 'get_curseforge_versions'), false, 'MR hit never dispatches its equal ID to CF');
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  const beforeCombinedCf = await page.evaluate(() => window.__testCalls.length);
  await allCfResult.getByRole('heading', { name: 'CF QA Mod', exact: true }).click();
  await page.getByText('cf-qa-mod.jar', { exact: true }).waitFor();
  const combinedCfCalls = await page.evaluate(index => window.__testCalls.slice(index).filter(call => call.command.endsWith('_versions')), beforeCombinedCf);
  assert.ok(combinedCfCalls.some(call => call.command === 'get_curseforge_versions' && call.args.projectId === '12345678'));
  assert.equal(combinedCfCalls.some(call => call.command === 'get_modrinth_versions'), false, 'CF hit never dispatches its equal ID to MR');
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });

  // A failing provider is a warning, while results from the other provider remain usable.
  await page.evaluate(() => { window.__testCfSearchError = true; });
  await page.getByRole('textbox', { name: 'Alle Anbieter durchsuchen', exact: true }).press('Enter');
  await page.getByText(/Mock CurseForge-Suche nicht verfügbar/).waitFor();
  await allMrResult.waitFor();
  assert.equal(await page.getByRole('heading', { name: 'CF QA Mod', exact: true }).count(), 0);
  assert.match(await page.getByRole('combobox', { name: 'Inhaltsquelle', exact: true }).innerText(), /Alle Anbieter/);
  await page.evaluate(() => { window.__testCfSearchError = false; });
  await page.getByRole('textbox', { name: 'Alle Anbieter durchsuchen', exact: true }).press('Enter');
  await allCfResult.waitFor();

  // An obsolete combined query must not overwrite a newer single-provider filter/query.
  await page.evaluate(() => { window.__testDelayCfSearch = 'obsolete'; window.__testReleaseCfSearch = undefined; });
  const allSearchInput = page.getByRole('textbox', { name: 'Alle Anbieter durchsuchen', exact: true });
  await allSearchInput.fill('obsolete'); await allSearchInput.press('Enter');
  await page.waitForFunction(() => typeof window.__testReleaseCfSearch === 'function');
  await page.getByRole('heading', { name: 'MR Numeric QA · obsolete', exact: true }).waitFor();
  await page.getByRole('status').filter({ hasText: /Weitere Anbieter werden geladen/ }).waitFor();
  assert.equal(await page.getByRole('heading', { name: 'CF QA Mod · obsolete', exact: true }).count(), 0, 'MR results are usable while CF is still pending');
  await chooseSource('Modrinth');
  const mrSearchInput = page.getByRole('textbox', { name: 'Modrinth durchsuchen', exact: true });
  await mrSearchInput.fill('current'); await mrSearchInput.press('Enter');
  await page.getByRole('heading', { name: 'MR Numeric QA · current', exact: true }).waitFor();
  await page.evaluate(async () => { window.__testDelayCfSearch = false; window.__testReleaseCfSearch(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); });
  await page.getByRole('heading', { name: 'MR Numeric QA · current', exact: true }).waitFor();
  assert.equal(await page.getByRole('heading', { name: 'CF QA Mod · obsolete', exact: true }).count(), 0);
  assert.equal(await page.getByRole('heading', { name: 'MR Numeric QA · obsolete', exact: true }).count(), 0);
  assert.match(await page.getByRole('combobox', { name: 'Inhaltsquelle', exact: true }).innerText(), /Modrinth/);

  // Query freshness also applies when both old and new searches use all providers.
  await chooseSource('Alle Anbieter');
  await page.getByRole('heading', { name: 'CF QA Mod · current', exact: true }).waitFor();
  await page.evaluate(() => { window.__testDelayMrSearch = 'old-query'; window.__testReleaseMrSearch = undefined; });
  await allSearchInput.fill('old-query'); await allSearchInput.press('Enter');
  await page.waitForFunction(() => typeof window.__testReleaseMrSearch === 'function');
  await page.getByRole('heading', { name: 'CF QA Mod · old-query', exact: true }).waitFor();
  await page.getByRole('status').filter({ hasText: /Weitere Anbieter werden geladen/ }).waitFor();
  assert.equal(await page.getByRole('heading', { name: 'MR Numeric QA · old-query', exact: true }).count(), 0, 'CF results are usable while MR is still pending');
  await allSearchInput.fill('new-query'); await allSearchInput.press('Enter');
  await page.getByRole('heading', { name: 'MR Numeric QA · new-query', exact: true }).waitFor();
  await page.getByRole('heading', { name: 'CF QA Mod · new-query', exact: true }).waitFor();
  await page.evaluate(async () => { window.__testDelayMrSearch = false; window.__testReleaseMrSearch(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); });
  assert.equal(await page.getByRole('heading', { name: 'MR Numeric QA · old-query', exact: true }).count(), 0);
  assert.equal(await page.getByRole('heading', { name: 'CF QA Mod · old-query', exact: true }).count(), 0);
  await page.getByRole('heading', { name: 'MR Numeric QA · new-query', exact: true }).waitFor();
  await page.getByRole('heading', { name: 'CF QA Mod · new-query', exact: true }).waitFor();
  await allSearchInput.fill(''); await allSearchInput.press('Enter');
  await allMrResult.waitFor(); await allCfResult.waitFor();

  // Repository-linked Sodium becomes one stable row when the second provider arrives.
  const profileBeforeGrouping = await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1'));
  await page.evaluate(() => { window.__testGroupedSearch = true; window.__testDelayCfSearch = 'grouped-late'; window.__testReleaseCfSearch = undefined; });
  await allSearchInput.fill('grouped-late'); await allSearchInput.press('Enter');
  await page.waitForFunction(() => typeof window.__testReleaseCfSearch === 'function');
  const sodiumRow = page.locator('.search-result').filter({ has: page.getByRole('heading', { name: 'Sodium', exact: true }) });
  await sodiumRow.waitFor();
  assert.equal(await page.locator('.search-result').count(), 1);
  assert.equal(await sodiumRow.getAttribute('data-source'), 'modrinth');
  await sodiumRow.evaluate(element => { window.__testInitialGroupedRow = element; });
  const sodiumAdd = sodiumRow.getByRole('button', { name: 'Hinzufügen', exact: true });
  await sodiumAdd.focus();
  await page.evaluate(() => { window.__testGroupedAction = document.activeElement; });
  await page.evaluate(async () => { window.__testDelayCfSearch = false; window.__testReleaseCfSearch(); await new Promise(requestAnimationFrame); await new Promise(requestAnimationFrame); });
  const sodiumMrChoice = sodiumRow.getByRole('button', { name: 'Sodium über Modrinth auswählen', exact: true });
  const sodiumCfChoice = sodiumRow.getByRole('button', { name: 'Sodium über CurseForge auswählen', exact: true });
  async function waitForSodiumProvider(source) {
    await page.locator(`.search-result[data-source="${source}"]`).filter({ has: page.getByRole('heading', { name: 'Sodium', exact: true }) }).waitFor();
    await sodiumRow.locator(`.provider-choice[aria-pressed="true"]`).filter({ hasText: source === 'curseforge' ? 'CurseForge' : 'Modrinth' }).waitFor();
  }
  await sodiumMrChoice.waitFor(); await sodiumCfChoice.waitFor();
  assert.equal(await page.locator('.search-result').count(), 1, 'linked providers share one row');
  assert.equal(await sodiumRow.evaluate(element => element === window.__testInitialGroupedRow), true, 'late provider keeps the row');
  assert.equal(await page.evaluate(() => document.activeElement === window.__testGroupedAction), true, 'late provider keeps focus');
  assert.equal(await sodiumMrChoice.getAttribute('aria-pressed'), 'true');
  const linksRequest = await page.evaluate(() => window.__testCalls.filter(call => call.command === 'get_modrinth_project_links').at(-1));
  assert.deepEqual(linksRequest.args.projectIds, ['SodiumQA']);

  // Source toggles and reviewing exact versions must not silently migrate or create a pin.
  await sodiumCfChoice.click();
  await waitForSodiumProvider('curseforge');
  assert.equal(await sodiumRow.getAttribute('data-source'), 'curseforge');
  assert.equal(await sodiumCfChoice.getAttribute('aria-pressed'), 'true');
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1')), profileBeforeGrouping);
  const beforeSodiumCf = await page.evaluate(() => window.__testCalls.length);
  await sodiumAdd.click();
  await page.getByText('cf-qa-mod.jar', { exact: true }).waitFor();
  assert.ok(await page.evaluate(index => window.__testCalls.slice(index).some(call => call.command === 'get_curseforge_versions' && call.args.projectId === '34567890'), beforeSodiumCf));
  assert.equal(await page.evaluate(index => window.__testCalls.slice(index).some(call => call.command === 'get_modrinth_versions'), beforeSodiumCf), false);
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await sodiumMrChoice.click();
  await waitForSodiumProvider('modrinth');
  const beforeSodiumMr = await page.evaluate(() => window.__testCalls.length);
  await sodiumAdd.click();
  await page.getByText('sodium-qa.jar', { exact: true }).waitFor();
  assert.ok(await page.evaluate(index => window.__testCalls.slice(index).some(call => call.command === 'get_modrinth_versions' && call.args.projectId === 'SodiumQA'), beforeSodiumMr));
  assert.equal(await page.evaluate(index => window.__testCalls.slice(index).some(call => call.command === 'get_curseforge_versions'), beforeSodiumMr), false);
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1')), profileBeforeGrouping);

  // A source toggle and Add in one browser frame must use the current signal choice, not the old rendered hit.
  const beforeFastProviderAction = await page.evaluate(() => window.__testCalls.length);
  await sodiumRow.evaluate(element => {
    element.querySelector('[aria-label="Sodium über CurseForge auswählen"]').click();
    element.querySelector(':scope > button.button').click();
  });
  await page.getByText('cf-qa-mod.jar', { exact: true }).waitFor();
  const fastProviderCalls = await page.evaluate(index => window.__testCalls.slice(index), beforeFastProviderAction);
  assert.ok(fastProviderCalls.some(call => call.command === 'get_curseforge_versions' && call.args.projectId === '34567890'));
  assert.equal(fastProviderCalls.some(call => call.command === 'get_modrinth_versions'), false, 'same-frame Add resolves the current source choice');
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await waitForSodiumProvider('curseforge');

  // Provider display choices belong to the target instance; changing the target does not copy another instance's choice.
  const targetInstance = page.getByRole('combobox', { name: 'Zielinstanz auswählen', exact: true });
  await targetInstance.click();
  await page.getByRole('option', { name: /^Nur lokal/ }).click();
  await waitForSodiumProvider('modrinth');
  assert.match(await targetInstance.innerText(), /Nur lokal/);
  assert.equal(await sodiumCfChoice.getAttribute('aria-pressed'), 'false');
  assert.equal(await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances.find(instance => instance.id === 'qa-local-only').contents.length), 0);
  await targetInstance.click();
  await page.getByRole('option', { name: /^QA Fabric/ }).click();
  await sodiumMrChoice.waitFor();
  await sodiumMrChoice.click();
  await waitForSodiumProvider('modrinth');
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1')), profileBeforeGrouping, 'target changes and provider choices do not change pins');

  // An existing source pin blocks an accidental second provider pin for the linked project.
  await sodiumAdd.click();
  await page.getByRole('button', { name: 'Version vormerken', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await sodiumCfChoice.click();
  await waitForSodiumProvider('curseforge');
  await sodiumAdd.click();
  await page.getByText(/Dieses Projekt ist bereits über Modrinth im Profil/).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Version vormerken', exact: true }).isDisabled(), true);
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: 'Instanzen', exact: true }).click();
  await page.getByRole('button', { name: 'Sodium entfernen', exact: true }).click();
  assert.equal(await page.evaluate(() => localStorage.getItem('syncmc.workspace.v1')), profileBeforeGrouping, 'explicit removal is the only grouped-pin mutation');
  await page.evaluate(() => { window.__testGroupedSearch = false; });
  await page.getByRole('button', { name: 'Entdecken', exact: true }).click();
  await allSearchInput.fill(''); await allSearchInput.press('Enter');
  await allMrResult.waitFor(); await allCfResult.waitFor();

  // Existing single-source behavior remains explicit; delayed version results keep their captured provider.
  await chooseSource('CurseForge');
  await page.getByRole('heading', { name: 'CF QA Mod', exact: true }).waitFor();
  await page.evaluate(() => { window.__testDelayCfVersions = true; });
  await page.getByRole('heading', { name: 'CF QA Mod', exact: true }).click();
  await page.waitForFunction(() => typeof window.__testReleaseCfVersions === 'function');
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await chooseSource('Modrinth');
  await page.getByRole('heading', { name: 'MR Numeric QA', exact: true }).click();
  await page.getByText('qa-mod-1.1.jar', { exact: true }).waitFor();
  await page.evaluate(async () => { window.__testDelayCfVersions = false; window.__testReleaseCfVersions(); await new Promise(resolve => setTimeout(resolve, 0)); });
  await page.getByRole('dialog').getByRole('heading', { name: 'MR Numeric QA', exact: true }).waitFor();
  await page.getByRole('button', { name: 'Version vormerken', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await chooseSource('CurseForge');
  const cfResult = page.locator('.search-result').filter({ has: page.getByRole('heading', { name: 'CF QA Mod', exact: true }) });
  await cfResult.getByRole('button', { name: 'Hinzufügen', exact: true }).waitFor();
  await cfResult.getByRole('button', { name: 'Hinzufügen', exact: true }).click();
  await page.getByText('cf-qa-mod.jar', { exact: true }).waitFor();
  await page.getByRole('button', { name: 'Version vormerken', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  const sameNumericId = await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances[0].contents.filter(content => content.projectId === '12345678'));
  assert.equal(sameNumericId.length, 2, 'equal numeric project IDs from different sources remain separate');
  assert.deepEqual(sameNumericId.map(content => content.source).sort(), ['curseforge', 'modrinth']);
  assert.equal(sameNumericId.find(content => content.source === 'curseforge').sha1, 'b'.repeat(40));
  assert.match(sameNumericId.find(content => content.source === 'curseforge').id, /^cf-/);

  // Opted-out distribution is an explicit provider error; no guessed URL or automatic download is offered.
  await page.evaluate(() => { window.__testCFBlocked = true; });
  await page.getByRole('heading', { name: 'CF QA Mod', exact: true }).click();
  await page.getByRole('dialog').getByRole('alert').filter({ hasText: /keine automatischen Downloads/ }).waitFor();
  assert.equal(await page.getByRole('button', { name: 'Version vormerken', exact: true }).count(), 0);
  await page.getByRole('button', { name: 'Dialog schließen', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.evaluate(() => { window.__testCFBlocked = false; });

  // Update checks dispatch to each item's provider and preserve its official checksum and version pin.
  await page.getByRole('button', { name: 'Instanzen', exact: true }).click();
  await page.getByRole('button', { name: 'Mod-Updates prüfen', exact: true }).click();
  const cfUpdate = page.locator('.mod-update-row').filter({ hasText: 'CF QA Mod' });
  await cfUpdate.getByRole('button', { name: 'Version prüfen', exact: true }).click();
  await page.getByRole('dialog').getByText(/CurseForge/).first().waitFor();
  await page.getByRole('button', { name: 'Diese Version installieren', exact: true }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.getByText(/CF QA Mod wurde auf 1.1.0 aktualisiert/).waitFor();
  const updatedCF = await page.evaluate(() => JSON.parse(localStorage.getItem('syncmc.workspace.v1')).instances[0].contents.find(content => content.source === 'curseforge'));
  assert.equal(updatedCF.versionId, '2002'); assert.equal(updatedCF.sha1, 'c'.repeat(40));
  assert.equal(updatedCF.sha512, 'd'.repeat(128), 'locally computed SHA512 can accompany the provider SHA1');
  const providerCalls = await page.evaluate(() => window.__testCalls);
  assert.ok(providerCalls.some(call => call.command === 'get_modrinth_versions' && call.args.projectId === '12345678'));
  assert.ok(providerCalls.some(call => call.command === 'get_curseforge_versions' && call.args.projectId === '12345678'));
  const storedData = await page.evaluate(() => Object.keys(localStorage).map(key => localStorage.getItem(key)).join('\n'));
  assert.equal(storedData.includes('qa-approved-cf-key') || storedData.includes('qa-invalid-cf-key'), false, 'API keys never enter device settings, profiles, backups, or exports');
  await page.getByRole('button', { name: 'Geräte & Sync', exact: true }).click();
  await page.getByRole('button', { name: 'Profile exportieren', exact: true }).click();
  await page.locator('.export-feedback code').filter({ hasText: '/qa/exports/syncmc-instanzen.json' }).waitFor();
  const cfExport = await page.evaluate(() => window.__testCalls.filter(call => call.command === 'save_profile_export').at(-1).args.contents);
  assert.equal(cfExport.includes('qa-approved-cf-key') || cfExport.includes('qa-invalid-cf-key'), false, 'native profile export contains no API key');
  assert.equal(JSON.parse(cfExport).instances[0].contents.find(content => content.source === 'curseforge').sha1, 'c'.repeat(40));
  await page.getByRole('button', { name: 'Einstellungen', exact: true }).first().click();
  await page.getByRole('button', { name: 'Gespeicherten Schlüssel entfernen', exact: true }).click();
  await page.getByText('Der lokale CurseForge-Schlüssel wurde entfernt.', { exact: true }).waitFor();
  const saved = await page.evaluate(() => ({ workspace: JSON.parse(localStorage.getItem('syncmc.workspace.v1')), device: JSON.parse(localStorage.getItem('syncmc.device.v1')) }));
  assert.equal(saved.workspace.instances[0].loaderVersion, '0.18.0');
  assert.equal(saved.device.javaPath, '');
  assert.equal(saved.device.microsoftClientId, defaultMicrosoftClientId);
  assert.equal(/access_token|refresh_token|device_code|password/.test(JSON.stringify(saved)), false, 'no credentials enter localStorage');
  assert.equal(nativeDialogCount, 0, 'confirmations work without native window.confirm');
  assert.deepEqual(errors, []);
  const output = process.env.SYNCMC_TEST_SCREENSHOT;
  if (output) { await mkdir(output.slice(0, output.lastIndexOf('/')), { recursive: true }); await page.screenshot({ path: output, fullPage: true }); }
  console.log(JSON.stringify({ passed: true, checks: ['Fabric pinning', 'installation and required dependency', 'authentication gate and device-code polling', 'SyncMC app default and local override reset', 'managed Java setup and manual-path preservation', 'instance backup and rescue restore', 'compatible per-mod release update', 'conflict merge with local-only retention', 'native profile export success, folder open, and failure handling', 'persistent profile snapshot export and rescue restore', 'quota failure blocks profile mutation', 'Java validation', 'explicit launch and stop', 'accessible in-app confirmation cancel/accept, Escape, backdrop, and focus restoration', 'forced-stop confirmation', 'logs', 'disable reconciliation', 'local settings migration', 'all providers default with missing-key fallback', 'combined same-ID hits and provider badges', 'combined per-hit version dispatch', 'repository-linked grouping keeps progressive row and focus', 'explicit grouped provider selection preserves pins', 'same-frame source toggle and Add use current provider', 'grouped provider choices stay scoped to target instance', 'linked project prevents accidental duplicate provider pin', 'partial provider failure keeps other results', 'progressive results while either provider is pending', 'stale async query and filter protection', 'CurseForge missing-key and validation errors', 'source-aware identity and async version context', 'CurseForge SHA1 pins, provider update dispatch, distribution opt-out', 'no frontend credentials'], pageErrors: errors }, null, 2));
} catch (error) { console.error(JSON.stringify({ pageErrors: errors, text: await page.locator('body').innerText() })); await page.screenshot({path:'/private/tmp/syncmc-frontend-failure.png'}); throw error; } finally { await context.close(); await browser.close(); }
