import type { GameLog } from './minecraft.service';

export type GameLogLevel = 'INFO' | 'WARN' | 'ERROR' | 'FATAL' | 'DEBUG' | 'TRACE' | 'UNKNOWN';
export type GameLogFilter = 'all' | 'warnings' | 'errors';

export function gameLogLevel(log: GameLog): GameLogLevel {
  const level = log.level?.toUpperCase();
  if (level === 'WARNING') return 'WARN';
  return level === 'INFO' || level === 'WARN' || level === 'ERROR' || level === 'FATAL'
    || level === 'DEBUG' || level === 'TRACE' ? level : 'UNKNOWN';
}

export function gameLogSource(log: GameLog): string {
  return log.logger || (log.stream === 'launcher' ? 'SyncMC' : log.stream === 'stderr' ? 'Standardfehler' : 'Minecraft');
}

export function gameLogLabel(log: GameLog): string {
  const level = gameLogLevel(log);
  return level === 'UNKNOWN' ? (log.stream === 'stderr' ? 'STDERR' : 'LOG') : level;
}

export function filterGameLogs(logs: GameLog[], filter: GameLogFilter, query: string): GameLog[] {
  const search = query.trim().toLocaleLowerCase('de');
  return logs.filter(log => {
    const level = gameLogLevel(log);
    if (filter === 'warnings' && level !== 'WARN') return false;
    if (filter === 'errors' && level !== 'ERROR' && level !== 'FATAL') return false;
    return !search || [log.line, gameLogSource(log), log.thread, log.stream, gameLogLabel(log)]
      .some(value => value?.toLocaleLowerCase('de').includes(search));
  });
}

export function formatGameLogs(logs: GameLog[]): string {
  return logs.map(log => {
    const time = new Date(log.timestamp * 1000).toISOString();
    const thread = log.thread ? ` [${log.thread}]` : '';
    return `${time} [${gameLogLabel(log)}] [${gameLogSource(log)}]${thread} ${log.line.replace(/\n/g, '\n    ')}`;
  }).join('\n');
}
