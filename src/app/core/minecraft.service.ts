import { Injectable } from '@angular/core';
import { invoke } from '@tauri-apps/api/core';
import type { InstalledContent, Instance, DeviceSettings } from './models';

export interface MinecraftVersion {
  id: string;
  type: 'release' | 'snapshot';
  releaseTime: string;
}

export interface MinecraftCatalog {
  latest: { release: string; snapshot: string };
  versions: MinecraftVersion[];
}

export interface InstallationReport {
  instanceId: string;
  versionId: string;
  directory: string;
  javaMajorVersion: number;
  filesVerified: number;
  bytesVerified: number;
  nativeArchives: number;
  installedAt: number;
  loader: string;
  loaderVersion?: string;
}

export interface RuntimeInfo {
  os: string;
  arch: string;
  javaAvailable: boolean;
  javaVersion: string | null;
  javaMajor?: number | null;
  javaArch?: string | null;
  javaPath: string;
}

export interface FabricLoaderVersion { version: string; stable: boolean }
export interface MinecraftAccount { name: string; uuid: string }
export interface LoginChallenge { flowId: string; userCode: string; verificationUri: string; expiresIn: number; interval: number }
export interface LoginPoll { status: 'pending' | 'slow_down' | 'complete' | 'expired'; account?: MinecraftAccount }
export interface GameStatus { running: boolean; instanceId?: string | null; pid?: number | null; exitCode?: number | null; startedAt?: number | null }
export interface GameLog {
  timestamp: number;
  stream: string;
  line: string;
  level?: string;
  logger?: string;
  thread?: string;
}
export interface BackupSummary {
  id: string;
  instanceId: string;
  createdAt: number;
  reason: string;
  fileCount: number;
  totalBytes: number;
  mcVersion: string;
  loader: string;
  profileName: string;
  contentsCount: number;
  verifiedContent: boolean;
  containedSections: string[];
}
export interface BackupRestoreReport {
  backup: BackupSummary;
  rescueBackup: BackupSummary;
  profile: Instance;
  contents: InstalledContent[];
  requiresInstallation: boolean;
  requiresContentInstall: boolean;
}
export interface ContentReport { contents: InstalledContent[]; filesVerified: number; dependencyCount: number; backup?: BackupSummary | null }
export interface ContentProgress { instanceId: string; completedFiles: number; totalFiles: number }
export interface ProfileExportReport { path: string; bytes: number }
export interface JavaInstallProgress {
  major: number;
  stage: 'metadata' | 'downloading' | 'extracting' | 'verifying' | 'complete';
  downloadedBytes: number;
  totalBytes: number;
}

export interface InstallationProgress {
  instanceId: string;
  stage: 'metadata' | 'downloading' | 'natives' | 'loader' | 'complete';
  completedFiles: number;
  totalFiles: number;
  downloadedBytes: number;
}

@Injectable({ providedIn: 'root' })
export class MinecraftService {
  async catalog(): Promise<MinecraftCatalog> {
    let data: unknown;
    if ('__TAURI_INTERNALS__' in globalThis) {
      data = await invoke<unknown>('get_minecraft_versions');
    } else {
      const response = await fetch('/api/minecraft/version_manifest_v2.json', {
        signal: AbortSignal.timeout(20_000),
      });
      if (!response.ok) throw new Error('Die Minecraft-Versionen konnten nicht von Mojang geladen werden.');
      data = await response.json();
    }
    if (!data || typeof data !== 'object') throw new Error('Mojang hat keine gültige Versionsliste geliefert.');
    const manifest = data as { latest?: { release?: unknown; snapshot?: unknown }; versions?: unknown };
    if (!Array.isArray(manifest.versions) || typeof manifest.latest?.release !== 'string'
      || typeof manifest.latest?.snapshot !== 'string') {
      throw new Error('Mojang hat keine gültige Versionsliste geliefert.');
    }
    const versions: MinecraftVersion[] = [];
    const seen = new Set<string>();
    for (const item of manifest.versions) {
      if (!item || typeof item !== 'object') continue;
      const version = item as Record<string, unknown>;
      if ((version['type'] !== 'release' && version['type'] !== 'snapshot')
        || typeof version['id'] !== 'string' || !/^[a-zA-Z0-9][a-zA-Z0-9 ._+-]{0,49}$/u.test(version['id'])
        || version['id'] !== version['id'].trim()
        || typeof version['releaseTime'] !== 'string' || !Number.isFinite(Date.parse(version['releaseTime']))
        || seen.has(version['id'])) continue;
      seen.add(version['id']);
      versions.push({ id: version['id'], type: version['type'], releaseTime: version['releaseTime'] });
    }
    versions.sort((a, b) => Date.parse(b.releaseTime) - Date.parse(a.releaseTime));
    if (!versions.some(version => version.id === manifest.latest!.release && version.type === 'release')) {
      throw new Error('Die aktuelle Minecraft-Version fehlt in der Versionsliste.');
    }
    return { latest: { release: manifest.latest.release, snapshot: manifest.latest.snapshot }, versions };
  }

  installation(instanceId: string): Promise<InstallationReport | null> {
    return invoke<InstallationReport | null>('get_minecraft_installation', { instanceId });
  }

  install(instanceId: string, versionId: string, loader: string, loaderVersion?: string): Promise<InstallationReport> {
    return invoke<InstallationReport>('install_minecraft', { instanceId, versionId, loader, loaderVersion });
  }

  fabricLoaders(gameVersion: string): Promise<FabricLoaderVersion[]> {
    return invoke<FabricLoaderVersion[]>('get_fabric_loaders', { gameVersion });
  }

  runtime(javaPath = ''): Promise<RuntimeInfo> {
    return invoke<RuntimeInfo>('runtime_info', { javaPath });
  }

  validateJava(javaPath: string, requiredMajor: number): Promise<RuntimeInfo> {
    return invoke<RuntimeInfo>('validate_java', { javaPath, requiredMajor });
  }

  installJava(requiredMajor: number): Promise<RuntimeInfo> {
    return invoke<RuntimeInfo>('install_java_runtime', { requiredMajor });
  }

  installContent(instance: Instance): Promise<ContentReport> {
    return invoke<ContentReport>('install_instance_content', { instanceId: instance.id, gameVersion: instance.mcVersion,
      loader: instance.loader, contents: instance.contents, profile: instance });
  }

  backups(instanceId: string): Promise<BackupSummary[]> {
    return invoke<BackupSummary[]>('list_instance_backups', { instanceId });
  }

  createBackup(instance: Instance, reason = 'manual'): Promise<BackupSummary> {
    return invoke<BackupSummary>('create_instance_backup', { instanceId: instance.id, profile: instance, reason });
  }

  restoreBackup(instanceId: string, backupId: string): Promise<BackupRestoreReport> {
    return invoke<BackupRestoreReport>('restore_instance_backup', { instanceId, backupId });
  }

  saveProfileExport(contents: string, filename: string): Promise<ProfileExportReport> {
    return invoke<ProfileExportReport>('save_profile_export', { contents, filename });
  }

  openProfileExports(): Promise<void> {
    return invoke<void>('open_profile_exports');
  }

  launch(instance: Instance, device: DeviceSettings): Promise<GameStatus> {
    return invoke<GameStatus>('launch_minecraft', { instanceId: instance.id, versionId: instance.mcVersion,
      loader: instance.loader, loaderVersion: instance.loaderVersion, javaPath: device.javaPath,
      memoryMb: device.memoryMb, fullscreen: device.fullscreen, contents: instance.contents });
  }

  gameStatus(): Promise<GameStatus> { return invoke<GameStatus>('game_status'); }
  stop(force = false): Promise<void> { return invoke<void>('stop_minecraft', { force }); }
  logs(): Promise<GameLog[]> { return invoke<GameLog[]>('get_game_logs'); }
  account(): Promise<MinecraftAccount | null> { return invoke<MinecraftAccount | null>('get_minecraft_account'); }
  beginLogin(clientId: string): Promise<LoginChallenge> { return invoke<LoginChallenge>('begin_microsoft_login', { clientId }); }
  pollLogin(flowId: string): Promise<LoginPoll> { return invoke<LoginPoll>('poll_microsoft_login', { flowId }); }
  cancelLogin(flowId: string): Promise<void> { return invoke<void>('cancel_microsoft_login', { flowId }); }
  logout(): Promise<void> { return invoke<void>('logout_minecraft_account'); }
  openLogin(): Promise<void> { return invoke<void>('open_microsoft_login'); }
  openRegistration(): Promise<void> { return invoke<void>('open_microsoft_registration'); }
}
