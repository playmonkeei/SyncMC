export type Page = 'start' | 'instances' | 'discover' | 'sync' | 'settings';
export type ContentKind = 'mod' | 'shader' | 'resourcepack' | 'datapack';
export type ContentSource = 'modrinth' | 'curseforge';
export type Loader = 'fabric' | 'vanilla' | 'forge' | 'neoforge';

export interface InstalledContent {
  id: string;
  projectId: string;
  versionId: string;
  name: string;
  version: string;
  filename: string;
  iconUrl?: string;
  kind: ContentKind;
  enabled: boolean;
  sha512?: string;
  sha1?: string;
  source: ContentSource;
  status: 'planned' | 'downloaded';
  dependency?: boolean;
}

export interface ServerFavorite {
  id: string;
  name: string;
  address: string;
}

export interface Instance {
  id: string;
  name: string;
  mcVersion: string;
  loader: Loader;
  loaderVersion?: string;
  cover: 'redwood' | 'vanilla' | 'create';
  contents: InstalledContent[];
  servers: ServerFavorite[];
}

export interface DeviceSettings {
  memoryMb: number;
  fullscreen: boolean;
  deviceName: string;
  javaPath: string;
  microsoftClientId: string;
}

export interface SyncmcManifest {
  format: 'syncmc-manifest';
  version: 1;
  instances: Instance[];
}

export interface SearchHit {
  source?: ContentSource;
  source_url?: string | null;
  project_id: string;
  slug?: string;
  title: string;
  description: string;
  icon_url: string;
  author: string;
  downloads: number;
  project_type: string;
  all_project_types?: string[];
  categories?: string[];
  versions?: string[];
  follows?: number;
  license?: string;
  gallery?: string[];
  featured_gallery?: string | null;
  latest_version?: string;
}

export interface ModVersionFile {
  filename: string;
  url: string;
  hashes: { sha512?: string; sha1?: string; [algorithm: string]: string | undefined };
  primary: boolean;
  size?: number;
  file_type?: string | null;
}

export interface ModVersionDependency {
  version_id: string | null;
  project_id: string | null;
  dependency_type: 'required' | 'optional' | 'incompatible' | 'embedded';
  file_name?: string | null;
}

export interface ModVersion {
  source?: ContentSource;
  project_type?: ContentKind;
  id: string;
  project_id?: string;
  name: string;
  version_number: string;
  version_type?: 'release' | 'beta' | 'alpha';
  game_versions?: string[];
  loaders?: string[];
  date_published?: string;
  files: ModVersionFile[];
  dependencies: ModVersionDependency[];
}
