import { Injectable, inject } from '@angular/core';
import { ModrinthService } from './modrinth.service';
import { CurseForgeService } from './curseforge.service';
import type { ContentKind, ContentSource, Loader, ModVersion, SearchHit } from './models';

export type CatalogHit = SearchHit & { source: ContentSource };

@Injectable({ providedIn: 'root' })
export class ContentService {
  private readonly modrinth = inject(ModrinthService);
  private readonly curseforge = inject(CurseForgeService);

  async search(source: ContentSource, query: string, gameVersion: string, loader: Loader, kind: ContentKind): Promise<CatalogHit[]> {
    const hits = source === 'curseforge' ? await this.curseforge.search(query, gameVersion, loader, kind)
      : await this.modrinth.search(query, gameVersion, loader, kind);
    return hits.map(hit => ({ ...hit, source }));
  }

  async versions(source: ContentSource, projectId: string, gameVersion: string, loader: Loader, kind: ContentKind): Promise<ModVersion[]> {
    const versions = source === 'curseforge' ? await this.curseforge.versions(projectId, gameVersion, loader, kind)
      : await this.modrinth.versions(projectId, gameVersion, loader, kind);
    return versions.map(version => ({ ...version, source }));
  }

  projectLinks(projectIds: string[]): Promise<{ project_id: string; source_url: string | null }[]> {
    return this.modrinth.projectLinks(projectIds);
  }
}
