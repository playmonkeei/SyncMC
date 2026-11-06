import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import type { ContentKind, Loader, ModVersion, SearchHit } from './models';

export type { ModVersion, SearchHit } from './models';

function isNative(): boolean {
  return '__TAURI_INTERNALS__' in globalThis;
}

@Injectable({ providedIn: 'root' })
export class ModrinthService {
  private readonly knownKinds = new Map<string, ContentKind>();

  async search(query: string, gameVersion: string, loader: Loader, kind: ContentKind): Promise<SearchHit[]> {
    const cleanQuery = query.trim().slice(0, 200);
    const facets: string[][] = [
      [kind === 'datapack' ? 'all_project_types:datapack' : `project_type:${kind}`],
      [`versions:${gameVersion}`],
    ];
    if (kind === 'mod') {
      // Vanilla has no mod loader and therefore cannot install Fabric/Forge mods.
      if (loader === 'vanilla') return [];
      facets.push([`categories:${loader}`]);
    }
    const params = new URLSearchParams({
      query: cleanQuery, facets: JSON.stringify(facets), limit: '24',
      index: cleanQuery ? 'relevance' : 'downloads',
    });
    const result = isNative()
      ? await invoke<{ hits: SearchHit[] }>('search_modrinth', {
        query: cleanQuery, gameVersion, loader, projectType: kind,
      })
      : await this.fetchJson<{ hits: SearchHit[] }>(`/api/modrinth/search?${params}`);
    if (!result || !Array.isArray(result.hits)) throw new Error('Modrinth hat keine gültige Suchantwort geliefert.');
    const hits = result.hits.filter((hit) => hit && typeof hit.project_id === 'string' && typeof hit.title === 'string');
    for (const hit of hits) this.knownKinds.set(hit.project_id, kind);
    return hits;
  }

  async versions(projectId: string, gameVersion: string, loader: Loader, explicitKind?: ContentKind): Promise<ModVersion[]> {
    if (!/^[a-zA-Z0-9_-]+$/u.test(projectId)) throw new Error('Die Modrinth-Projekt-ID ist ungültig.');
    const params = new URLSearchParams({ game_versions: JSON.stringify([gameVersion]), include_changelog: 'false' });
    // A dialog captures its kind when opened; later search responses must not change that context.
    const kind = explicitKind ?? this.knownKinds.get(projectId);
    if (kind === 'mod' && loader === 'vanilla') return [];
    // Official /v2/tag/loader maps these loaders to their supported project types.
    // A multi-type project can publish different artifact types for the same Minecraft version.
    const allowedLoaders = kind === 'mod' ? [loader]
      : kind === 'shader' ? ['canvas', 'iris', 'optifine', 'vanilla']
      : kind === 'resourcepack' ? ['minecraft']
      : kind === 'datapack' ? ['datapack'] : undefined;
    if (allowedLoaders) params.set('loaders', JSON.stringify(allowedLoaders));
    const result = isNative()
      ? await invoke<ModVersion[]>('get_modrinth_versions', { projectId, gameVersion, loader, projectType: kind ?? null })
      : await this.fetchJson<ModVersion[]>(`/api/modrinth/project/${encodeURIComponent(projectId)}/version?${params}`);
    if (!Array.isArray(result)) throw new Error('Modrinth hat keine gültige Versionsliste geliefert.');
    return result.filter((version) => version && typeof version.id === 'string'
      && Array.isArray(version.files) && version.files.length > 0)
      // Missing compatibility metadata cannot establish that a version fits this instance.
      .filter((version) => Array.isArray(version.game_versions) && version.game_versions.includes(gameVersion))
      .filter((version) => Array.isArray(version.loaders) && version.loaders.length > 0
        && (!allowedLoaders || version.loaders.some((versionLoader) => allowedLoaders.includes(versionLoader))));
  }

  async projectLinks(projectIds: string[]): Promise<{ project_id: string; source_url: string | null }[]> {
    const ids = [...new Set(projectIds)];
    if (!ids.length) return [];
    if (ids.length > 24 || ids.some(id => !/^[a-zA-Z0-9_-]{1,128}$/u.test(id))) {
      throw new Error('Die Projektliste für den Quellenvergleich ist ungültig.');
    }
    const projects = isNative()
      ? await invoke<{ project_id: string; source_url: string | null }[]>('get_modrinth_project_links', { projectIds: ids })
      : await this.fetchJson<{ id: string; source_url: string | null }[]>(`/api/modrinth/projects?${new URLSearchParams({ ids: JSON.stringify(ids) })}`)
        .then(projects => projects.map(project => ({ project_id: project.id, source_url: project.source_url })));
    return projects.filter(project => ids.includes(project.project_id))
      .map(project => ({ project_id: project.project_id, source_url: typeof project.source_url === 'string' && project.source_url.length <= 2048 ? project.source_url : null }));
  }

  private async fetchJson<T>(url: string): Promise<T> {
    let response: Response;
    try {
      response = await fetch(url, { headers: { Accept: 'application/json' }, signal: AbortSignal.timeout(20_000) });
    } catch {
      throw new Error('Modrinth ist gerade nicht erreichbar. Prüfe deine Internetverbindung und versuche es erneut.');
    }
    if (!response.ok) {
      if (response.status === 429) throw new Error('Modrinth erhält gerade zu viele Anfragen. Bitte gleich erneut versuchen.');
      throw new Error(`Modrinth-Anfrage fehlgeschlagen (HTTP ${response.status}).`);
    }
    try { return await response.json() as T; } catch {
      throw new Error('Modrinth hat eine ungültige Antwort geliefert. Ist der lokale API-Proxy gestartet?');
    }
  }
}
