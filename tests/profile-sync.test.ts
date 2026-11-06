import assert from 'node:assert/strict';
import test from 'node:test';
import { mergeProfileSync, prepareProfileSync, profileFingerprint } from '../src/app/core/profile-sync.ts';
import type { Instance } from '../src/app/core/models.ts';

function profile(id = 'one'): Instance {
  return { id, name: `Profil ${id}`, mcVersion: '1.21.1', loader: 'fabric', loaderVersion: '0.20.1',
    cover: 'redwood', servers: [], contents: [{ id: 'sodium', projectId: 'AANobbMI', versionId: 'oldpin',
      name: 'Sodium', version: '0.8', filename: 'sodium.jar', kind: 'mod', enabled: true,
      source: 'modrinth', status: 'downloaded' }] };
}

test('comparison distinguishes additions and differences without removing absent local profiles', () => {
  const local = [profile(), profile('local-only')];
  const incoming = [profile(), profile('new')];
  incoming[0]!.name = 'Geändert';
  const plan = prepareProfileSync(local, incoming);
  assert.deepEqual(plan.entries.map(entry => [entry.kind, entry.changes]), [['conflict', ['Name']], ['added', []]]);
  assert.equal(plan.keptLocalCount, 1);
  assert.throws(() => mergeProfileSync(local, plan, {}), /Bitte entscheide/);
  const merged = mergeProfileSync(local, plan, { one: 'incoming' });
  assert.deepEqual(merged.map(instance => instance.id), ['one', 'local-only', 'new']);
  assert.equal(merged[0]!.name, 'Geändert');
  assert.equal(merged[0]!.contents[0]!.status, 'downloaded');
  assert.equal(merged[2]!.contents[0]!.status, 'planned');
  assert.equal(local[0]!.name, 'Profil one');
});

test('a changed version pin loses imported download status and local choice keeps current profile', () => {
  const local = [profile()];
  const incoming = [profile()];
  incoming[0]!.contents[0]!.versionId = 'newpin';
  const plan = prepareProfileSync(local, incoming);
  assert.deepEqual(plan.entries[0]!.changes, ['Inhalte und Versionspins']);
  assert.equal(mergeProfileSync(local, plan, { one: 'incoming' })[0]!.contents[0]!.status, 'planned');
  assert.equal(mergeProfileSync(local, plan, { one: 'local' })[0]!.contents[0]!.versionId, 'oldpin');
});

test('download states and list order never create portable conflicts', () => {
  const first = profile();
  first.servers = [{ id: 'a', name: 'A', address: 'a.example' }, { id: 'b', name: 'B', address: 'b.example' }];
  first.contents.push({ ...first.contents[0]!, id: 'other', projectId: 'other' });
  const incoming = structuredClone(first);
  incoming.servers.reverse(); incoming.contents.reverse();
  incoming.contents.forEach(item => item.status = 'planned');
  incoming.contents.forEach(item => item.sha1 = 'a'.repeat(40));
  assert.equal(profileFingerprint(first), profileFingerprint(incoming));
  const plan = prepareProfileSync([first], [incoming]);
  assert.equal(plan.entries[0]!.kind, 'identical');
  assert.equal(mergeProfileSync([first], plan, {})[0]!.contents[0]!.status, 'downloaded');
});

test('stale previews cannot silently overwrite a subsequent local edit', () => {
  const local = [profile()];
  const plan = prepareProfileSync(local, [profile('remote')]);
  local[0]!.servers.push({ id: 'new', name: 'Neuer Server', address: 'server.example' });
  assert.throws(() => mergeProfileSync(local, plan, {}), /inzwischen geändert/);
});

test('IDs that match object prototype names still require explicit decisions', () => {
  const local = [profile('__proto__'), profile('constructor')];
  const incoming = structuredClone(local);
  incoming.forEach(instance => instance.name = 'Neu');
  const plan = prepareProfileSync(local, incoming);
  assert.throws(() => mergeProfileSync(local, plan, {}), /Bitte entscheide/);
  const choices = Object.fromEntries([['__proto__', 'incoming'], ['constructor', 'local']]) as Record<string, 'incoming' | 'local'>;
  assert.deepEqual(mergeProfileSync(local, plan, choices).map(instance => instance.name), ['Neu', 'Profil constructor']);
});

test('provider and authoritative SHA-1 distinguish CurseForge pins from local byte receipts', () => {
  const local = profile();
  local.contents[0] = { ...local.contents[0]!, source: 'curseforge', projectId: '123', versionId: '456',
    sha1: 'a'.repeat(40), sha512: 'b'.repeat(128) };
  const incoming = structuredClone(local);
  delete incoming.contents[0]!.sha512;
  incoming.contents[0]!.status = 'planned';
  assert.equal(prepareProfileSync([local], [incoming]).entries[0]!.kind, 'identical');
  incoming.name = 'Neuer Anzeigename';
  const plan = prepareProfileSync([local], [incoming]);
  const merged = mergeProfileSync([local], plan, { one: 'incoming' });
  assert.equal(merged[0]!.contents[0]!.sha512, 'b'.repeat(128));
  assert.equal(merged[0]!.contents[0]!.status, 'downloaded');
  incoming.contents[0]!.source = 'modrinth';
  assert.notEqual(profileFingerprint(local), profileFingerprint(incoming));
  incoming.contents[0]!.source = 'curseforge'; incoming.contents[0]!.sha1 = 'c'.repeat(40);
  const newHashPlan = prepareProfileSync([local], [incoming]);
  assert.equal(mergeProfileSync([local], newHashPlan, { one: 'incoming' })[0]!.contents[0]!.status, 'planned');
});
