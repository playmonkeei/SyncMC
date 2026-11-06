import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import type { ContentKind, Loader, ModVersion, SearchHit } from './models';

export interface CurseForgeStatus { configured: boolean }

/** CurseForge credentials and all authenticated requests stay in the native core. */
@Injectable({ providedIn: 'root' })
export class CurseForgeService {
  private requireNative(): void {
    if (!('__TAURI_INTERNALS__' in globalThis)) {
      throw new Error('CurseForge steht in der Desktop-App zur Verfügung. Im Browser werden keine API-Schlüssel übertragen.');
    }
  }

  status(): Promise<CurseForgeStatus> {
    this.requireNative();
    return invoke<CurseForgeStatus>('curseforge_status');
  }

  saveKey(key: string): Promise<CurseForgeStatus> {
    this.requireNative();
    return invoke<CurseForgeStatus>('set_curseforge_key', { key });
  }

  clearKey(): Promise<CurseForgeStatus> {
    this.requireNative();
    return invoke<CurseForgeStatus>('clear_curseforge_key');
  }

  openApplication(): Promise<void> {
    this.requireNative();
    return invoke<void>('open_curseforge_application');
  }

  async search(query: string, gameVersion: string, loader: Loader, kind: ContentKind): Promise<SearchHit[]> {
    this.requireNative();
    if (kind === 'datapack') throw new Error('Die Suche nach Data Packs wird für CurseForge noch nicht unterstützt.');
    if (kind === 'mod' && loader === 'vanilla') return [];
    const result = await invoke<{ hits: SearchHit[] }>('search_curseforge', {
      query: query.trim().slice(0, 200), gameVersion, loader, projectType: kind,
    });
    if (!result || !Array.isArray(result.hits)) throw new Error('CurseForge hat keine gültigen Suchergebnisse geliefert.');
    return result.hits.filter(hit => hit && typeof hit.project_id === 'string' && /^[1-9][0-9]*$/u.test(hit.project_id)
      && typeof hit.title === 'string' && hit.project_type === kind).map(hit => ({ ...hit, source: 'curseforge' }));
  }

  async versions(projectId: string, gameVersion: string, loader: Loader, kind: ContentKind): Promise<ModVersion[]> {
    this.requireNative();
    if (!/^[1-9][0-9]*$/u.test(projectId)) throw new Error('Die CurseForge-Projekt-ID ist ungültig.');
    if (kind === 'datapack') throw new Error('Data Packs werden für CurseForge noch nicht unterstützt.');
    if (kind === 'mod' && loader === 'vanilla') return [];
    const result = await invoke<ModVersion[]>('get_curseforge_versions', { projectId, gameVersion, loader, projectType: kind });
    if (!Array.isArray(result)) throw new Error('CurseForge hat keine gültigen Dateiversionen geliefert.');
    return result.filter(version => version && typeof version.id === 'string' && /^[1-9][0-9]*$/u.test(version.id)
      && (!version.project_id || version.project_id === projectId)
      && version.project_type === kind
      && Array.isArray(version.files) && version.files.length > 0
      && Array.isArray(version.game_versions) && version.game_versions.includes(gameVersion)
      // CurseForge's class and exact game version identify packs; it does not attest a shader engine.
      && (kind !== 'mod' || (Array.isArray(version.loaders) && version.loaders.includes(loader))))
      .map(version => ({ ...version, source: 'curseforge' }));
  }
}
