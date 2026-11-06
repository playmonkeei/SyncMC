import assert from 'node:assert/strict';
import test from 'node:test';
import { groupMinecraftVersions, minecraftVersionLabel } from '../src/app/core/minecraft-versions.ts';
import type { MinecraftVersion } from '../src/app/core/minecraft.service.ts';

function version(id: string, type: MinecraftVersion['type'] = 'snapshot'): MinecraftVersion {
  return { id, type, releaseTime: '2026-11-03T13:08:56Z' };
}

test('groups releases, patches, and previews by their numeric release family', () => {
  const versions = [
    version('26.4-snapshot-7'), version('26.4-snapshot-6'), version('26.3', 'release'),
    version('26.3-rc-3'), version('26.3-pre-1'), version('1.21.11', 'release'),
    version('1.21.11-pre1'), version('1.21', 'release'), version('1.14 Pre-Release 5'),
    version('1.14.1 Pre-Release 1'),
  ];
  const groups = groupMinecraftVersions(versions, true);
  assert.deepEqual(groups.map(group => [group.id, group.label, group.versions.map(item => item.id)]), [
    ['26.4', '26.4', ['26.4-snapshot-7', '26.4-snapshot-6']],
    ['26.3', '26.3', ['26.3', '26.3-rc-3', '26.3-pre-1']],
    ['1.21', '1.21', ['1.21.11', '1.21.11-pre1', '1.21']],
    ['1.14', '1.14', ['1.14 Pre-Release 5', '1.14.1 Pre-Release 1']],
  ]);
});

test('weekly snapshots remain explicit year collections after numeric families', () => {
  const groups = groupMinecraftVersions([
    version('25w46a'), version('26.4-snapshot-7'), version('25w45a'),
    version('1.RV-Pre1'), version('26.3', 'release'), version('24w14potato'),
    version('24w40a'), version('1.21.1', 'release'),
  ], true);
  assert.deepEqual(groups.map(group => [group.id, group.label]), [
    ['26.4', '26.4'], ['26.3', '26.3'], ['1.21', '1.21'],
    ['snapshots-2025', 'Snapshots 2025'], ['special', 'Sonderversionen'],
    ['snapshots-2024', 'Snapshots 2024'],
  ]);
  assert.deepEqual(groups.find(group => group.id === 'snapshots-2025')?.versions.map(item => item.id),
    ['25w46a', '25w45a']);
  assert.deepEqual(groups.find(group => group.id === 'special')?.versions.map(item => item.id),
    ['1.RV-Pre1', '24w14potato']);
});

test('hiding snapshots removes preview-only groups and keeps stable patches in order', () => {
  const groups = groupMinecraftVersions([
    version('26.4-snapshot-7'), version('26.3', 'release'), version('26.3-rc-3'),
    version('25w46a'), version('1.RV-Pre1'), version('1.21.11', 'release'),
    version('1.21', 'release'),
  ], false);
  assert.deepEqual(groups.map(group => [group.id, group.versions.map(item => item.id)]), [
    ['26.3', ['26.3']], ['1.21', ['1.21.11', '1.21']],
  ]);
});

test('every supplied item is preserved exactly once without mutating input order', () => {
  const repeated = version('26.4-snapshot-7');
  const versions = [version('25w46a'), repeated, version('26.3', 'release'), repeated,
    version('1.14 Pre-Release 5'), version('3D Shareware v1.34')];
  const original = [...versions];
  const grouped = groupMinecraftVersions(versions, true).flatMap(group => group.versions);
  assert.equal(grouped.length, versions.length);
  for (const item of new Set(versions)) {
    assert.equal(grouped.filter(candidate => candidate === item).length,
      versions.filter(candidate => candidate === item).length);
  }
  assert.deepEqual(versions, original);
});

test('labels shorten modern previews while keeping patch versions unambiguous', () => {
  const cases: [MinecraftVersion, string, string][] = [
    [version('26.4-snapshot-7'), '26.4', 'Snapshot 7'],
    [version('26.4-pre-2'), '26.4', 'Pre-Release 2'],
    [version('26.4-rc-1'), '26.4', 'Release Candidate 1'],
    [version('26.4', 'release'), '26.4', 'Release'],
    [version('1.21.11', 'release'), '1.21', '1.21.11'],
    [version('1.21.11-pre1'), '1.21', '1.21.11 Pre-Release 1'],
    [version('1.21.11-rc1'), '1.21', '1.21.11 Release Candidate 1'],
    [version('26.4.1-snapshot-2'), '26.4', '26.4.1 Snapshot 2'],
    [version('1.14 Pre-Release 5'), '1.14', 'Pre-Release 5'],
    [version('1.14.1 Pre-Release 1'), '1.14', '1.14.1 Pre-Release 1'],
    [version('1.14 Release Candidate 2'), '1.14', 'Release Candidate 2'],
    [version('25w46a'), 'snapshots-2025', '25w46a'],
    [version('1.RV-Pre1'), 'special', '1.RV-Pre1'],
  ];
  for (const [item, familyId, expected] of cases) {
    assert.equal(minecraftVersionLabel(item, familyId), expected, item.id);
  }
});

test('an empty catalog or a snapshot-only catalog with snapshots hidden has no groups', () => {
  assert.deepEqual(groupMinecraftVersions([], true), []);
  assert.deepEqual(groupMinecraftVersions([version('26.4-snapshot-7')], false), []);
});
