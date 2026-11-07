import assert from 'node:assert/strict';
import test from 'node:test';
import { filterGameLogs, formatGameLogs, gameLogLevel, gameLogSource } from '../src/app/core/game-logs.ts';
import type { GameLog } from '../src/app/core/minecraft.service.ts';

const timestamp = 1794043253;
function log(line: string, patch: Partial<GameLog> = {}): GameLog {
  return { timestamp, stream: 'stdout', line, ...patch };
}

test('recognizes explicit severity while keeping unclassified stderr out of error results', () => {
  assert.equal(gameLogLevel(log('warning', { level: 'warning' })), 'WARN');
  assert.equal(gameLogLevel(log('error', { level: 'error' })), 'ERROR');
  for (const level of ['INFO', 'WARN', 'ERROR', 'FATAL', 'DEBUG', 'TRACE']) {
    assert.equal(gameLogLevel(log('message', { level })), level);
  }
  const stderr = log('UNSUPPORTED (log once): gldCopyBufferSubData', { stream: 'stderr' });
  assert.equal(gameLogLevel(stderr), 'UNKNOWN');
  assert.equal(gameLogLevel(log('custom', { level: 'CUSTOM' })), 'UNKNOWN');
  assert.deepEqual(filterGameLogs([stderr], 'errors', ''), []);
  assert.deepEqual(filterGameLogs([stderr], 'all', ''), [stderr]);
});

test('combines severity with case-insensitive message, source, thread and stream searches', () => {
  const info = log('Loading Minecraft', { level: 'INFO', logger: 'FabricLoader/GameProvider', thread: 'main' });
  const warning = log('Missing sound', { level: 'WARN', logger: 'Sodium', thread: 'Render thread' });
  const error = log('Unable to load shader\n\tat Game.render(Game.java:12)', { level: 'ERROR', logger: 'Minecraft', thread: 'Render thread' });
  const fatal = log('Could not start', { level: 'FATAL', logger: 'Engine' });
  const stderr = log('Native diagnostic', { stream: 'stderr', level: 'UNKNOWN' });
  const logs = [info, warning, error, fatal, stderr];
  const original = structuredClone(logs);
  assert.deepEqual(filterGameLogs(logs, 'all', '  FABRICLOADER  '), [info]);
  assert.deepEqual(filterGameLogs(logs, 'all', 'game.java:12'), [error]);
  assert.deepEqual(filterGameLogs(logs, 'warnings', 'render THREAD'), [warning]);
  assert.deepEqual(filterGameLogs(logs, 'errors', ''), [error, fatal]);
  assert.deepEqual(filterGameLogs(logs, 'errors', 'render'), [error]);
  assert.deepEqual(filterGameLogs(logs, 'all', 'stderr'), [stderr]);
  assert.deepEqual(filterGameLogs(logs, 'all', 'fatal'), [fatal]);
  assert.deepEqual(filterGameLogs(logs, 'warnings', 'shader'), []);
  assert.deepEqual(filterGameLogs(logs, 'all', '   '), logs);
  assert.deepEqual(logs, original, 'filtering preserves the original entries and order');
});

test('provides useful sources for launcher and unstructured game output', () => {
  assert.equal(gameLogSource(log('started', { stream: 'launcher' })), 'SyncMC');
  assert.equal(gameLogSource(log('native output', { stream: 'stderr' })), 'Standardfehler');
  assert.equal(gameLogSource(log('plain output')), 'Minecraft');
  assert.equal(gameLogSource(log('message', { logger: 'FabricLoader/Mixin' })), 'FabricLoader/Mixin');
});

test('copies UTC timestamps, level, source and thread with readable multiline details', () => {
  const logs = [
    log('Loading 2 mods:\n\t- fabric-api\n\t- sodium', { level: 'INFO', logger: 'FabricLoader', thread: 'main' }),
    log('Missing sound', { level: 'WARN', logger: 'Minecraft', thread: 'Render thread' }),
    log('Native diagnostic', { stream: 'stderr', level: 'UNKNOWN' }),
    log('Plain diagnostic', { level: 'UNKNOWN' }),
  ];
  const original = structuredClone(logs);
  const copied = formatGameLogs(logs);
  const isoTime = new Date(timestamp * 1000).toISOString();
  assert.ok(copied.includes(isoTime), 'the copied timestamp includes its date and UTC timezone');
  assert.match(copied, /\[INFO\].*\[FabricLoader\].*\[main\].*Loading 2 mods:/);
  assert.ok(copied.includes('\n    \t- fabric-api\n    \t- sodium'), 'continuations retain their original indentation');
  assert.match(copied, /\[WARN\].*\[Minecraft\].*\[Render thread\].*Missing sound/);
  assert.match(copied, /\[STDERR\].*\[Standardfehler\].*Native diagnostic/);
  assert.match(copied, /\[LOG\].*\[Minecraft\].*Plain diagnostic/);
  assert.doesNotMatch(copied, /log4j:|<!\[CDATA\[/);
  assert.deepEqual(logs, original);
  assert.equal(formatGameLogs([]), '');
});

test('copying a filtered view exports only matching messages and preserves their order', () => {
  const logs = [
    log('routine startup', { level: 'INFO' }),
    log('first failure', { level: 'ERROR' }),
    log('unrelated warning', { level: 'WARN' }),
    log('second failure', { level: 'FATAL' }),
  ];
  const copied = formatGameLogs(filterGameLogs(logs, 'errors', 'failure'));
  assert.ok(copied.indexOf('first failure') < copied.indexOf('second failure'));
  assert.doesNotMatch(copied, /routine startup|unrelated warning/);
});
