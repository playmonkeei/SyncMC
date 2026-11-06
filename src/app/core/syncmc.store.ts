import { computed, Injectable, signal } from '@angular/core';
import { SYNCMC_APP_CONFIG } from './app-config';
import { mergeProfileSync, prepareProfileSync as compareProfiles, profileFingerprint,
  type ProfileBackup, type ProfileSyncChoice, type ProfileSyncPlan } from './profile-sync';
import type {
  ContentKind,
  DeviceSettings,
  InstalledContent,
  Instance,
  Loader,
  ServerFavorite,
  SyncmcManifest,
} from './models';

const WORKSPACE_KEY = 'syncmc.workspace.v1';
const DEVICE_KEY = 'syncmc.device.v1';
const PROFILE_BACKUPS_KEY = 'syncmc.profile-backups.v1';
const WORKSPACE_RECOVERY_KEY = 'syncmc.workspace-recovery.v1';
const MAX_PROFILE_BACKUPS = 100;
const MAX_MANIFEST_LENGTH = 5_000_000;
const MAX_INSTANCES = 100;
const MAX_CONTENTS = 1_000;
const MAX_SERVERS = 200;
const LOADERS: Loader[] = ['fabric', 'vanilla', 'forge', 'neoforge'];
const KINDS: ContentKind[] = ['mod', 'shader', 'resourcepack', 'datapack'];

interface SavedProfileBackup extends ProfileBackup { manifest: string }

function record(value: unknown): Record<string, unknown> {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error('Die Manifestdatei enthält ungültige Daten.');
  }
  return value as Record<string, unknown>;
}

function text(value: unknown, field: string, maxLength = 160): string {
  if (typeof value !== 'string') throw new Error(`${field} fehlt oder ist ungültig.`);
  const clean = value.trim();
  if (!clean || clean.length > maxLength || /[\u0000-\u001f\u007f]/u.test(clean)) {
    throw new Error(`${field} ist leer, zu lang oder enthält ungültige Zeichen.`);
  }
  return clean;
}

function identifier(value: unknown, field: string): string {
  const result = text(value, field, 100);
  if (!/^[a-zA-Z0-9_-]+$/u.test(result)) throw new Error(`${field} ist ungültig.`);
  return result;
}

function list(value: unknown, field: string, maximum: number): unknown[] {
  if (!Array.isArray(value) || value.length > maximum) {
    throw new Error(`${field} muss eine Liste mit höchstens ${maximum} Einträgen sein.`);
  }
  return value;
}

function unique<T extends { id: string }>(items: T[], field: string): T[] {
  if (new Set(items.map((item) => item.id)).size !== items.length) {
    throw new Error(`${field} enthält doppelte IDs.`);
  }
  return items;
}

function safeIcon(value: unknown): string | undefined {
  if (typeof value !== 'string' || value.length > 2_048) return undefined;
  try {
    const url = new URL(value);
    if (url.protocol !== 'https:' || url.username || url.password) return undefined;
    // Icons are public assets. Drop query strings so manifests cannot carry access tokens.
    url.search = '';
    url.hash = '';
    return url.href;
  } catch {
    return undefined;
  }
}

function sanitizeContent(value: unknown, imported = false): InstalledContent {
  const item = record(value);
  const kind = item['kind'] as ContentKind;
  const source = item['source'];
  if (!KINDS.includes(kind) || (source !== 'modrinth' && source !== 'curseforge')) {
    throw new Error('Der Inhaltstyp oder die Inhaltsquelle wird nicht unterstützt.');
  }
  const filename = text(item['filename'], 'Dateiname', 240);
  if (/[<>:"/\\|?*]/u.test(filename) || filename === '.' || filename === '..' || /[. ]$/u.test(filename)
    || /^(?:con|prn|aux|nul|com[1-9]|lpt[1-9])(?:\.|$)/iu.test(filename)) {
    throw new Error('Ein Inhaltsdateiname ist für macOS und Windows ungültig.');
  }
  if (typeof item['enabled'] !== 'boolean') throw new Error('Der Aktivierungsstatus eines Inhalts ist ungültig.');
  if (item['status'] !== 'planned' && item['status'] !== 'downloaded') {
    throw new Error('Der Downloadstatus eines Inhalts ist ungültig.');
  }
  const sha512 = typeof item['sha512'] === 'string' && /^[a-fA-F0-9]{128}$/u.test(item['sha512'])
    ? item['sha512'].toLowerCase()
    : undefined;
  if (item['sha512'] !== undefined && !sha512) throw new Error('Die SHA-512-Prüfsumme ist ungültig.');
  const sha1 = typeof item['sha1'] === 'string' && /^[a-fA-F0-9]{40}$/u.test(item['sha1'])
    ? item['sha1'].toLowerCase() : undefined;
  if (item['sha1'] !== undefined && !sha1) throw new Error('Die SHA-1-Prüfsumme ist ungültig.');
  const projectId = identifier(item['projectId'], 'Projekt-ID');
  const versionId = identifier(item['versionId'], 'Versions-ID');
  if (source === 'curseforge' && [projectId, versionId].some(id => !/^[1-9][0-9]{0,9}$/u.test(id) || Number(id) > 4_294_967_295)) {
    throw new Error('CurseForge-Projekt- und Datei-IDs müssen gültige positive Zahlen sein.');
  }
  return {
    id: identifier(item['id'], 'Inhalts-ID'),
    projectId,
    versionId,
    name: text(item['name'], 'Inhaltsname'),
    version: text(item['version'], 'Inhaltsversion'),
    filename,
    ...(safeIcon(item['iconUrl']) ? { iconUrl: safeIcon(item['iconUrl']) } : {}),
    kind,
    enabled: item['enabled'],
    ...(sha512 ? { sha512 } : {}),
    ...(sha1 ? { sha1 } : {}),
    source,
    // A manifest describes desired files, never proves that bytes exist on this device.
    status: imported ? 'planned' : item['status'],
    ...(item['dependency'] === true ? { dependency: true } : {}),
  };
}

function sanitizeAddress(value: unknown): string {
  const address = text(value, 'Serveradresse', 253);
  if (/[\s/@?#\\]/u.test(address)) throw new Error('Bitte eine Serveradresse ohne URL oder Zugangsdaten eingeben.');
  const match = /^(?:\[[a-fA-F0-9:]+\]|[a-zA-Z0-9](?:[a-zA-Z0-9.-]*[a-zA-Z0-9])?)(?::(\d{1,5}))?$/u.exec(address);
  if (!match || (match[1] && (Number(match[1]) < 1 || Number(match[1]) > 65_535))) {
    throw new Error('Die Serveradresse oder der Port ist ungültig.');
  }
  return address.toLowerCase();
}

function sanitizeServer(value: unknown): ServerFavorite {
  const server = record(value);
  return {
    id: identifier(server['id'], 'Server-ID'),
    name: text(server['name'], 'Servername'),
    address: sanitizeAddress(server['address']),
  };
}

function sanitizeInstance(value: unknown, imported = false): Instance {
  const instance = record(value);
  const loader = instance['loader'] as Loader;
  if (!LOADERS.includes(loader)) throw new Error('Dieser Mod-Loader wird nicht unterstützt.');
  const mcVersion = text(instance['mcVersion'], 'Minecraft-Version', 50);
  if (!/^[a-zA-Z0-9][a-zA-Z0-9 ._+-]*$/u.test(mcVersion)) throw new Error('Die Minecraft-Version ist ungültig.');
  const cover = instance['cover'];
  const loaderVersion = instance['loaderVersion'] === undefined ? undefined
    : text(instance['loaderVersion'], 'Loader-Version', 100);
  if (loaderVersion && !/^[a-zA-Z0-9][a-zA-Z0-9._+-]*$/u.test(loaderVersion)) {
    throw new Error('Die Loader-Version ist ungültig.');
  }
  if (cover !== 'redwood' && cover !== 'vanilla' && cover !== 'create') {
    throw new Error('Das Instanz-Cover ist ungültig.');
  }
  const contents = unique(list(instance['contents'], 'Inhalte', MAX_CONTENTS)
    .map((content) => sanitizeContent(content, imported)), 'Die Inhaltsliste');
  if (new Set(contents.map((content) => `${content.source}:${content.projectId}`)).size !== contents.length) {
    throw new Error('Eine Instanz darf jedes Projekt derselben Quelle nur einmal enthalten.');
  }
  if (new Set(contents.map((content) => content.filename.toLowerCase())).size !== contents.length) {
    throw new Error('Die Inhaltsliste enthält kollidierende Dateinamen.');
  }
  const servers = unique(list(instance['servers'], 'Server', MAX_SERVERS).map(sanitizeServer), 'Die Serverliste');
  return {
    id: identifier(instance['id'], 'Instanz-ID'),
    name: text(instance['name'], 'Instanzname', 100),
    mcVersion,
    loader,
    ...(loader !== 'vanilla' && loaderVersion ? { loaderVersion } : {}),
    cover,
    contents,
    servers,
  };
}

/** Validate and copy only known shared fields; imported download state is always local/planned. */
export function parseManifest(manifestText: string): SyncmcManifest {
  if (typeof manifestText !== 'string' || manifestText.length > MAX_MANIFEST_LENGTH
    || new TextEncoder().encode(manifestText).byteLength > MAX_MANIFEST_LENGTH) {
    throw new Error('Die Manifestdatei ist zu groß (maximal 5 MB).');
  }
  let parsed: unknown;
  try { parsed = JSON.parse(manifestText); } catch { throw new Error('Die Datei enthält kein gültiges JSON.'); }
  const manifest = record(parsed);
  if (manifest['format'] !== 'syncmc-manifest' || manifest['version'] !== 1) {
    throw new Error('Dieses Manifestformat wird nicht unterstützt. Erwartet wird SyncMC Version 1.');
  }
  return {
    format: 'syncmc-manifest',
    version: 1,
    instances: unique(list(manifest['instances'], 'Instanzen', MAX_INSTANCES)
      .map((instance) => sanitizeInstance(instance, true)), 'Die Instanzliste'),
  };
}

function id(prefix: string): string {
  const uuid = globalThis.crypto?.randomUUID?.();
  return `${prefix}-${uuid ?? `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`}`;
}

function demoContent(projectId: string, name: string, kind: ContentKind = 'mod'): InstalledContent {
  return {
    id: `demo-${projectId}`,
    projectId,
    versionId: `demo-${projectId}`,
    name,
    version: 'Beispiel · noch nicht installiert',
    filename: `${name.toLowerCase().replace(/[^a-z0-9]+/gu, '-')}.${kind === 'mod' ? 'jar' : 'zip'}`,
    kind,
    enabled: true,
    source: 'modrinth',
    status: 'planned',
  };
}

function defaultInstances(): Instance[] {
  return [
    {
      id: 'redwood', name: 'Redwood SMP', mcVersion: '1.21.1', loader: 'fabric', cover: 'redwood',
      contents: [
        demoContent('AANobbMI', 'Sodium'),
        demoContent('YL57xq9U', 'Iris Shaders'),
        demoContent('P7dR8mSH', 'Fabric API'),
        demoContent('mOgUt4GM', 'Mod Menu'),
        demoContent('HVnmMxH1', 'Complementary Reimagined', 'shader'),
      ],
      servers: [{ id: 'redwood-example', name: 'Redwood · Beispielserver', address: 'play.redwood.example' }],
    },
    {
      id: 'vanilla', name: 'Vanilla', mcVersion: '1.21.1', loader: 'vanilla', cover: 'vanilla',
      contents: [], servers: [],
    },
    {
      id: 'create', name: 'Create & Explore', mcVersion: '1.21.1', loader: 'fabric', cover: 'create',
      contents: [demoContent('LNytGWDc', 'Create'), demoContent('P7dR8mSH', 'Fabric API')],
      servers: [{ id: 'create-example', name: 'Create · Beispielserver', address: 'create.example' }],
    },
  ];
}

function defaultDevice(): DeviceSettings {
  return { memoryMb: 4_096, fullscreen: false, deviceName: 'Dieser Computer', javaPath: '',
    microsoftClientId: SYNCMC_APP_CONFIG.microsoftClientId };
}

function readStorage(key: string): string | null {
  try { return globalThis.localStorage?.getItem(key) ?? null; } catch { return null; }
}

@Injectable({ providedIn: 'root' })
export class SyncmcStore {
  readonly instances = signal<Instance[]>(defaultInstances());
  readonly selectedId = signal('redwood');
  readonly selected = computed<Instance | undefined>(() => this.instances().find((instance) => instance.id === this.selectedId()) ?? this.instances()[0]);
  readonly device = signal<DeviceSettings>(defaultDevice());
  readonly persistenceError = signal<string | null>(null);
  private unreadableWorkspace: string | null = null;
  readonly workspaceRecoveryText = signal<string | null>(readStorage(WORKSPACE_RECOVERY_KEY));
  private profileBackupLoadError = false;
  private readonly profileSnapshots = signal<SavedProfileBackup[]>([]);
  readonly profileBackups = computed<ProfileBackup[]>(() => this.profileSnapshots()
    .map(({ id, createdAt, reason, instanceCount }) => ({ id, createdAt, reason, instanceCount })));

  constructor() {
    const saved = readStorage(WORKSPACE_KEY);
    if (saved) {
      try {
        const state = record(JSON.parse(saved));
        const restored = unique(list(state['instances'], 'Instanzen', MAX_INSTANCES)
          .map((instance) => sanitizeInstance(instance)), 'Die Instanzliste');
        this.instances.set(restored);
        const selectedId = state['selectedId'];
        this.selectedId.set(typeof selectedId === 'string' && restored.some((instance) => instance.id === selectedId)
          ? selectedId : restored[0]?.id ?? '');
      } catch {
        this.unreadableWorkspace = saved;
        this.workspaceRecoveryText.set(saved);
        this.persistenceError.set('Gespeicherte Instanzen konnten nicht geladen werden. Der unlesbare Rohstand bleibt für eine Wiederherstellung erhalten; vorerst werden Beispiele angezeigt.');
      }
    }
    const savedDevice = readStorage(DEVICE_KEY);
    if (savedDevice) {
      try { this.device.set(this.sanitizeDevice(record(JSON.parse(savedDevice)))); } catch { /* Keep safe device defaults. */ }
    }
    const savedBackups = readStorage(PROFILE_BACKUPS_KEY);
    if (savedBackups) {
      try {
        const backups = unique(list(JSON.parse(savedBackups), 'Profilsicherungen', MAX_PROFILE_BACKUPS).map(value => {
          const item = record(value);
          if (typeof item['manifest'] !== 'string') throw new Error('Ungültige Profilsicherung.');
          const manifest = parseManifest(item['manifest']);
          const createdAt = text(item['createdAt'], 'Sicherungsdatum', 40);
          if (!Number.isFinite(Date.parse(createdAt))) throw new Error('Ungültiges Sicherungsdatum.');
          return { id: identifier(item['id'], 'Sicherungs-ID'), createdAt,
            reason: text(item['reason'], 'Sicherungsgrund', 100),
            instanceCount: manifest.instances.length, manifest: JSON.stringify(manifest) };
        }), 'Profilsicherungen');
        this.profileSnapshots.set(backups);
      } catch {
        this.profileBackupLoadError = true;
        this.persistenceError.set('Lokale Profilsicherungen konnten nicht gelesen werden. Die aktuellen Profile bleiben erhalten.');
      }
    }
  }

  createInstance(name: string, mcVersion: string, loader: Loader): Instance {
    if (this.instances().length >= MAX_INSTANCES) throw new Error('Es sind maximal 100 Instanzen möglich.');
    const instance = sanitizeInstance({
      id: id('instance'), name, mcVersion, loader,
      cover: loader === 'vanilla' ? 'vanilla' : 'redwood', contents: [], servers: [],
    });
    this.instances.update((instances) => [...instances, instance]);
    this.selectedId.set(instance.id);
    this.persist();
    return instance;
  }

  selectInstance(instanceId: string): void {
    if (!this.instances().some((instance) => instance.id === instanceId)) return;
    this.selectedId.set(instanceId);
    this.persist();
  }

  updateInstance(patch: Partial<Instance>): void {
    const current = this.selected();
    if (!current) return;
    this.updateInstanceById(current.id, patch);
  }

  updateInstanceById(instanceId: string, patch: Partial<Instance>): void {
    const current = this.instances().find(instance => instance.id === instanceId);
    if (!current) return;
    const updated = sanitizeInstance({ ...current, ...patch, id: current.id });
    this.instances.update((instances) => instances.map((instance) => instance.id === current.id ? updated : instance));
    this.persist();
  }

  toggleContent(contentId: string): void {
    const current = this.selected();
    if (!current) return;
    this.updateInstance({ contents: current.contents.map((content) => content.id === contentId
      ? { ...content, enabled: !content.enabled } : content) });
  }

  addContent(value: InstalledContent): void {
    const current = this.selected();
    if (!current) throw new Error('Bitte zuerst eine Instanz anlegen.');
    const content = sanitizeContent(value);
    const existing = current.contents.find((item) => item.source === content.source && item.projectId === content.projectId);
    const contents = existing
      ? current.contents.map((item) => item.source === content.source && item.projectId === content.projectId ? { ...content, id: item.id } : item)
      : [...current.contents, content];
    this.updateInstance({ contents });
  }

  removeContent(contentId: string): void {
    const current = this.selected();
    if (current) this.updateInstance({ contents: current.contents.filter((content) => content.id !== contentId) });
  }

  addServer(name: string, address: string): boolean {
    const current = this.selected();
    if (!current) return false;
    let server: ServerFavorite;
    try { server = sanitizeServer({ id: id('server'), name, address }); } catch { return false; }
    if (current.servers.length >= MAX_SERVERS || current.servers.some((item) => item.address === server.address)) return false;
    this.updateInstance({ servers: [...current.servers, server] });
    return true;
  }

  removeServer(serverId: string): void {
    const current = this.selected();
    if (current) this.updateInstance({ servers: current.servers.filter((server) => server.id !== serverId) });
  }

  updateDevice(patch: Partial<DeviceSettings>): void {
    this.device.set(this.sanitizeDevice({ ...this.device(), ...patch }));
    this.persist();
  }

  exportManifest(): string {
    const manifest: SyncmcManifest = {
      format: 'syncmc-manifest', version: 1,
      instances: this.instances().map((instance) => sanitizeInstance(instance, true)),
    };
    const serialized = JSON.stringify(manifest, null, 2);
    if (serialized.length > MAX_MANIFEST_LENGTH || new TextEncoder().encode(serialized).byteLength > MAX_MANIFEST_LENGTH) {
      throw new Error('Die Profilliste überschreitet die zulässige Dateigröße von 5 MB. Es wurde keine unlesbare Sicherung erstellt.');
    }
    return serialized;
  }

  importManifest(manifestText: string): void {
    // Validate the complete document before changing any current state.
    const manifest = parseManifest(manifestText);
    this.backupProfiles('Vor vollständigem Profilimport');
    this.commitProfiles(manifest.instances, manifest.instances[0]?.id ?? '');
  }

  prepareProfileSync(manifestText: string): ProfileSyncPlan {
    return compareProfiles(this.instances(), parseManifest(manifestText).instances);
  }

  applyProfileSync(plan: ProfileSyncPlan, choices: Record<string, ProfileSyncChoice>): void {
    // Parse again at commit time: a preview is not an installation or validation receipt.
    const incoming = parseManifest(JSON.stringify({ format: 'syncmc-manifest', version: 1, instances: plan.incoming })).instances;
    const merged = mergeProfileSync(this.instances(), { ...plan, incoming }, choices);
    const checked = unique(list(merged, 'Instanzen', MAX_INSTANCES).map(value => sanitizeInstance(value)), 'Die Instanzliste');
    if (checked.length === this.instances().length && checked.every((instance, index) =>
      profileFingerprint(instance) === profileFingerprint(this.instances()[index]!))) return;
    this.backupProfiles('Vor Profilabgleich');
    this.commitProfiles(checked, this.selectedId());
  }

  backupProfiles(reason: string): ProfileBackup {
    this.protectUnreadableWorkspace();
    if (this.profileSnapshots().length >= MAX_PROFILE_BACKUPS) {
      throw new Error('Es sind bereits 100 Profilsicherungen vorhanden. Exportiere und entferne eine alte Sicherung, bevor du fortfährst.');
    }
    const backup: SavedProfileBackup = { id: id('profiles'), createdAt: new Date().toISOString(),
      reason: text(reason, 'Sicherungsgrund', 100), instanceCount: this.instances().length,
      manifest: this.exportManifest() };
    const snapshots = [backup, ...this.profileSnapshots()];
    this.writeProfileBackups(snapshots);
    const { manifest: _manifest, ...summary } = backup;
    return summary;
  }

  exportProfileBackup(backupId: string): string {
    const backup = this.profileSnapshots().find(item => item.id === backupId);
    if (!backup) throw new Error('Diese Profilsicherung ist nicht mehr vorhanden.');
    return backup.manifest;
  }

  restoreProfileBackup(backupId: string): void {
    const manifest = parseManifest(this.exportProfileBackup(backupId));
    this.backupProfiles('Vor Wiederherstellung der Profile');
    this.commitProfiles(manifest.instances, this.selectedId());
  }

  removeProfileBackup(backupId: string): void {
    if (!this.profileSnapshots().some(item => item.id === backupId)) return;
    this.writeProfileBackups(this.profileSnapshots().filter(item => item.id !== backupId));
  }

  private writeProfileBackups(backups: SavedProfileBackup[]): void {
    if (this.profileBackupLoadError) {
      throw new Error('Die gespeicherten Profilsicherungen sind beschädigt und werden nicht überschrieben. Exportiere den aktuellen Profilstand und prüfe den lokalen Anwendungsspeicher.');
    }
    try {
      if (!globalThis.localStorage) throw new Error('Kein lokaler Speicher');
      globalThis.localStorage.setItem(PROFILE_BACKUPS_KEY, JSON.stringify(backups));
    } catch {
      throw new Error('Die Profilsicherung konnte nicht lokal gespeichert werden. Es wurde nichts übernommen. Exportiere deine Profile und prüfe den verfügbaren Speicher.');
    }
    this.profileSnapshots.set(backups);
  }

  private commitProfiles(instances: Instance[], preferredId: string): void {
    const selectedId = instances.some(instance => instance.id === preferredId) ? preferredId : instances[0]?.id ?? '';
    try {
      if (!globalThis.localStorage) throw new Error('Kein lokaler Speicher');
      this.protectUnreadableWorkspace();
      globalThis.localStorage.setItem(WORKSPACE_KEY, JSON.stringify({ version: 1, instances, selectedId }));
    } catch {
      throw new Error('Die Profile konnten nicht lokal gespeichert werden. Der bisherige Stand bleibt erhalten.');
    }
    this.instances.set(instances);
    this.selectedId.set(selectedId);
    this.persistenceError.set(null);
  }

  private sanitizeDevice(value: Record<string, unknown>): DeviceSettings {
    const memoryMb = value['memoryMb'];
    if (typeof memoryMb !== 'number' || !Number.isInteger(memoryMb) || memoryMb < 1_024 || memoryMb > 65_536) {
      throw new Error('Der Arbeitsspeicher muss zwischen 1.024 und 65.536 MB liegen.');
    }
    if (typeof value['fullscreen'] !== 'boolean') throw new Error('Die Vollbildeinstellung ist ungültig.');
    const javaPath = typeof value['javaPath'] === 'string' ? value['javaPath'].trim() : '';
    const microsoftClientId = (typeof value['microsoftClientId'] === 'string' ? value['microsoftClientId'].trim() : '')
      || SYNCMC_APP_CONFIG.microsoftClientId;
    if (javaPath.length > 2_048 || /[\u0000-\u001f\u007f]/u.test(javaPath)) throw new Error('Der Java-Pfad ist ungültig.');
    if (microsoftClientId && !/^[a-zA-Z0-9-]{1,100}$/u.test(microsoftClientId)) throw new Error('Die Microsoft-Client-ID ist ungültig.');
    return { memoryMb, fullscreen: value['fullscreen'], deviceName: text(value['deviceName'], 'Gerätename', 80), javaPath, microsoftClientId };
  }

  private persist(): void {
    try {
      if (!globalThis.localStorage) return;
      this.protectUnreadableWorkspace();
      globalThis.localStorage.setItem(WORKSPACE_KEY, JSON.stringify({
        version: 1, instances: this.instances(), selectedId: this.selectedId(),
      }));
      globalThis.localStorage.setItem(DEVICE_KEY, JSON.stringify(this.device()));
      this.persistenceError.set(null);
    } catch {
      this.persistenceError.set('Änderungen sind im Arbeitsspeicher. Der lokale Speicher ist nicht verfügbar oder voll.');
    }
  }

  private protectUnreadableWorkspace(): void {
    if (this.unreadableWorkspace === null) return;
    try {
      if (!globalThis.localStorage) throw new Error('Kein lokaler Speicher');
      const previous = globalThis.localStorage.getItem(WORKSPACE_RECOVERY_KEY);
      if (previous !== null && previous !== this.unreadableWorkspace) {
        throw new Error('Ein anderer Rohstand wurde bereits gesichert');
      }
      globalThis.localStorage.setItem(WORKSPACE_RECOVERY_KEY, this.unreadableWorkspace);
    } catch {
      throw new Error('Der unlesbare Profil-Rohstand konnte nicht zusätzlich gesichert werden. Er wird nicht überschrieben. Exportiere die Wiederherstellungsdaten und prüfe den lokalen Speicher.');
    }
    this.unreadableWorkspace = null;
  }
}
