import type { CatalogHit } from './content.service';

export interface CatalogGroup {
  key: string;
  hits: CatalogHit[];
}

/** Compare provider-supplied repository references locally; never navigate or fetch them. */
export function canonicalRepository(value: unknown): string | null {
  if (typeof value !== 'string') return null;
  const raw = value.trim();
  if (!raw || raw.length > 2_048 || /[\s\\%?#\u0000-\u001f\u007f]/u.test(raw)) return null;
  const parts = /^https:\/\/([^/]+)(\/.*)?$/iu.exec(raw);
  if (!parts || parts[1]!.includes('@')) return null;
  let url: URL;
  try { url = new URL(raw); } catch { return null; }
  if (url.protocol !== 'https:' || url.username || url.password || url.port || url.search || url.hash) return null;
  const host = url.hostname;
  if (!['github.com', 'gitlab.com', 'codeberg.org', 'bitbucket.org'].includes(host)) return null;
  // Inspect the original path so URL's dot-segment normalization cannot turn a subpath into a repository.
  const path = (parts[2] ?? '').replace(/\/$/u, '');
  const segments = path.slice(1).split('/');
  if (segments.some(segment => !/^[A-Za-z0-9_.-]+$/u.test(segment) || segment === '.' || segment === '..')) return null;
  if (host === 'gitlab.com' ? segments.length < 2 || segments.includes('-') : segments.length !== 2) return null;
  segments[segments.length - 1] = segments.at(-1)!.replace(/\.git$/iu, '');
  if (!segments.at(-1)) return null;
  // GitHub repository paths are case-insensitive. Other hosts retain path casing conservatively.
  const repository = host === 'github.com' ? segments.join('/').toLowerCase() : segments.join('/');
  return `https://${host}/${repository}`;
}

function titleAndKind(hit: CatalogHit): string | null {
  const title = hit.title.normalize('NFKC').trim().replace(/\s+/gu, ' ').toLowerCase();
  return title && hit.project_type ? JSON.stringify([title, hit.project_type]) : null;
}

function repositoryIdentity(hit: CatalogHit): string | null {
  const titleKind = titleAndKind(hit);
  const repository = canonicalRepository(hit.source_url);
  return titleKind && repository ? JSON.stringify([titleKind, repository]) : null;
}

/** Preserve first appearance and source-specific pins; only the display row is grouped. */
export function groupCatalogHits(hits: CatalogHit[]): CatalogGroup[] {
  const groups: CatalogGroup[] = [];
  const matches = new Map<string, CatalogGroup[]>();
  const projects = new Map<string, { modrinth: Set<string>; curseforge: Set<string> }>();
  for (const hit of hits) {
    const identity = repositoryIdentity(hit);
    if (!identity) continue;
    const sources = projects.get(identity) ?? { modrinth: new Set<string>(), curseforge: new Set<string>() };
    sources[hit.source].add(hit.project_id);
    projects.set(identity, sources);
  }
  for (const hit of hits) {
    const candidate = repositoryIdentity(hit);
    const sources = candidate ? projects.get(candidate) : undefined;
    const identity = sources && sources.modrinth.size <= 1 && sources.curseforge.size <= 1 ? candidate : null;
    const matching = identity ? matches.get(identity) ?? [] : [];
    const existing = matching.find(group => group.hits.every(previous => previous.source !== hit.source));
    if (existing) existing.hits.push(hit);
    else {
      const group = { key: `${hit.source}:${hit.project_id}`, hits: [hit] };
      groups.push(group);
      if (identity) matches.set(identity, [...matching, group]);
    }
  }
  return groups;
}

/** Enrich only MR projects with a title/kind peer that already has a usable CF repository reference. */
export function candidateProjectIds(hits: CatalogHit[]): string[] {
  const curseforgeTitles = new Set(hits.filter(hit => hit.source === 'curseforge' && canonicalRepository(hit.source_url))
    .map(titleAndKind).filter((identity): identity is string => identity !== null));
  const ids = new Set<string>();
  for (const hit of hits) {
    const identity = titleAndKind(hit);
    if (hit.source === 'modrinth' && hit.project_id && identity && curseforgeTitles.has(identity)) ids.add(hit.project_id);
    if (ids.size === 24) break;
  }
  return [...ids];
}
