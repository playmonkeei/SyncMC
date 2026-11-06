import { Component, HostListener, OnDestroy, computed, effect, inject, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { listen } from '@tauri-apps/api/event';
import { SyncmcStore, parseManifest } from './core/syncmc.store';
import { SYNCMC_APP_CONFIG } from './core/app-config';
import type { ProfileSyncPlan, ProfileSyncChoice, ProfileBackup } from './core/profile-sync';
import { ContentService, type CatalogHit } from './core/content.service';
import { candidateProjectIds, groupCatalogHits } from './core/catalog-groups';
import { CurseForgeService } from './core/curseforge.service';
import { MinecraftService, type MinecraftCatalog, type InstallationReport, type InstallationProgress,
  type RuntimeInfo, type FabricLoaderVersion, type MinecraftAccount, type LoginChallenge,
  type GameStatus, type GameLog, type ContentProgress, type JavaInstallProgress, type BackupSummary } from './core/minecraft.service';
import { groupMinecraftVersions, minecraftVersionLabel } from './core/minecraft-versions';
import { GlassSelect } from './shared/glass-select/glass-select.component';
import type { Page, ContentKind, ContentSource, Loader, SearchHit, ModVersion, ModVersionFile, Instance } from './core/models';

type Dialog = 'create' | 'server' | 'launch' | 'connect' | 'version' | 'import' | 'account' | 'logs' | 'mod-update' | null;
type CatalogSource = ContentSource | 'all';
interface ModUpdateCheck {
  instanceId: string;
  contentId: string;
  projectId: string;
  source: ContentSource;
  fromVersionId: string;
  name: string;
  currentVersion: string;
  status: 'available' | 'current' | 'error';
  candidate?: ModVersion;
  file?: ModVersionFile;
  message?: string;
}
interface AppConfirmation {
  title: string;
  message: string;
  actionLabel: string;
  destructive?: boolean;
}

@Component({
  selector: 'app-root', standalone: true, imports: [CommonModule, FormsModule, GlassSelect],
  templateUrl: './app.html', styleUrls: ['./app.scss', './chrome.scss', './catalog.scss'],
})
export class App implements OnDestroy {
  readonly s = inject(SyncmcStore);
  private readonly api = inject(ContentService);
  private readonly curseforge = inject(CurseForgeService);
  private readonly minecraft = inject(MinecraftService);
  readonly page = signal<Page>('start');
  readonly tab = signal('contents');
  readonly contentMode = signal<'library' | 'browse'>('library');
  readonly catalogVisible = computed(() => this.page() === 'discover' ||
    (this.page() === 'instances' && this.tab() === 'contents' && this.contentMode() === 'browse'));
  readonly kind = signal<ContentKind>('mod');
  readonly contentSource = signal<CatalogSource>('all');
  readonly sourceLabel = computed(() => this.contentSource() === 'all' ? 'Alle Anbieter'
    : this.contentSource() === 'curseforge' ? 'CurseForge' : 'Modrinth');
  readonly catalogAttribution = computed(() => this.contentSource() === 'all'
    ? 'Ergebnisse der verfügbaren Anbieter. Die Quelle steht bei jedem Treffer.' : `Ergebnisse von ${this.sourceLabel()}.`);
  readonly curseforgeConfigured = signal(false);
  readonly curseforgeChecking = signal(false);
  readonly curseforgeSaving = signal(false);
  readonly curseforgeError = signal('');
  readonly curseforgeNotice = signal('');
  readonly sourceOptions = [
    { value: 'all', label: 'Alle Anbieter', icon: 'stack', description: 'Modrinth und CurseForge' },
    { value: 'modrinth', label: 'Modrinth', icon: 'circles-three' },
    { value: 'curseforge', label: 'CurseForge', icon: 'cube', description: 'Eigener API-Schlüssel erforderlich' },
  ];
  readonly curseforgeUnavailable = computed(() => !this.native
    ? 'CurseForge steht in der Desktop-App zur Verfügung. Im Browser werden keine API-Schlüssel übertragen.'
    : this.curseforgeChecking() ? 'CurseForge-Verbindung wird geprüft …' : !this.curseforgeConfigured()
    ? 'CurseForge ist noch nicht eingerichtet. Hinterlege in den Einstellungen einen eigenen, für SyncMC freigegebenen API-Schlüssel.'
    : this.kind() === 'datapack' ? 'Die Suche nach Data Packs wird für CurseForge noch nicht unterstützt.' : '');
  readonly catalogUnavailable = computed(() => this.contentSource() === 'curseforge' ? this.curseforgeUnavailable() : '');
  readonly catalogSourceNote = computed(() => this.contentSource() === 'all' && this.curseforgeUnavailable()
    ? `${this.curseforgeUnavailable()} ${this.curseforgeChecking() ? 'Modrinth wird bereits durchsucht.' : 'Die Suche zeigt weiterhin Ergebnisse von Modrinth.'}` : '');
  readonly dialog = signal<Dialog>(null);
  readonly confirmation = signal<AppConfirmation | null>(null);
  readonly toast = signal('');
  readonly results = signal<CatalogHit[]>([]);
  private readonly catalogChoices = signal<Record<string, string>>({});
  readonly catalogGroups = computed(() => this.contentSource() === 'all' ? groupCatalogHits(this.results())
    : this.results().map(hit => ({ key: `${hit.source}:${hit.project_id}`, hits: [hit] })));
  readonly catalogRows = computed(() => this.catalogGroups().map(group => ({ ...group,
    hit: group.hits.find(hit => `${hit.source}:${hit.project_id}` === this.catalogChoices()[`${this.s.selected()?.id}:${group.key}`])
      ?? group.hits.find(hit => this.hasProject(hit.project_id, hit.source)) ?? group.hits[0],
  })));
  readonly loading = signal(false);
  readonly searchError = signal('');
  readonly searchWarning = signal('');
  readonly searched = signal(false);
  readonly versions = signal<ModVersion[]>([]);
  readonly versionHit = signal<SearchHit | null>(null);
  readonly selectedVersion = signal('');
  readonly versionSource = computed<ContentSource>(() => this.versionHit()?.source
    ?? (this.contentSource() === 'curseforge' ? 'curseforge' : 'modrinth'));
  readonly alternatePinnedSource = computed(() => {
    const hit = this.versionHit();
    if (!hit) return '';
    const group = this.catalogGroups().find(group => group.hits.some(candidate => candidate.source === hit.source && candidate.project_id === hit.project_id));
    const existing = group?.hits.find(candidate => candidate.source !== hit.source && this.hasProject(candidate.project_id, candidate.source));
    return existing ? this.providerLabel(existing.source) : '';
  });
  readonly versionLoading = signal(false);
  readonly versionError = signal('');
  readonly contentQuery = signal('');
  readonly showExamples = signal(false);
  readonly runtime = signal<RuntimeInfo | null>(null);
  readonly native = '__TAURI_INTERNALS__' in globalThis;
  readonly minecraftCatalog = signal<MinecraftCatalog | null>(null);
  readonly minecraftLoading = signal(false);
  readonly minecraftError = signal('');
  readonly includeSnapshots = signal(true);
  private readonly launchInstanceId = signal('');
  readonly launchInstance = computed(() => this.s.instances().find(instance => instance.id === this.launchInstanceId()) ?? null);
  readonly installation = signal<InstallationReport | null>(null);
  readonly installationLoading = signal(false);
  readonly installationError = signal('');
  readonly installingInstance = signal<Instance | null>(null);
  readonly installProgress = signal<InstallationProgress | null>(null);
  readonly fabricVersions = signal<FabricLoaderVersion[]>([]);
  readonly fabricLoading = signal(false);
  readonly fabricError = signal('');
  readonly fabricOptions = computed(() => {
    const loaders = this.fabricVersions().map(loader => ({ value: loader.version,
      label: loader.version, description: loader.stable ? 'Stabil' : 'Vorabversion', icon: 'puzzle-piece' }));
    const pinned = this.launchInstance()?.loaderVersion;
    if (pinned && !loaders.some(loader => loader.value === pinned)) {
      loaders.unshift({ value: pinned, label: pinned, description: 'Gespeicherte Version', icon: 'puzzle-piece' });
    }
    return loaders;
  });
  readonly account = signal<MinecraftAccount | null>(null);
  readonly accountLoading = signal(false);
  readonly accountError = signal('');
  readonly defaultMicrosoftClientId = SYNCMC_APP_CONFIG.microsoftClientId;
  readonly usingDefaultMicrosoftClient = computed(() => this.s.device().microsoftClientId === this.defaultMicrosoftClientId);
  readonly loginChallenge = signal<LoginChallenge | null>(null);
  readonly loginBusy = signal(false);
  readonly loginExpiresAt = signal(0);
  readonly loginSeconds = signal(0);
  readonly game = signal<GameStatus>({ running: false });
  readonly gameError = signal('');
  readonly gameLaunching = signal(false);
  readonly gameStopping = signal(false);
  readonly gameStopRequested = signal(false);
  readonly gameLogs = signal<GameLog[]>([]);
  readonly logsLoading = signal(false);
  readonly contentInstallingId = signal('');
  readonly contentProgress = signal<ContentProgress | null>(null);
  readonly javaChecking = signal(false);
  readonly javaError = signal('');
  readonly javaValidatedMajor = signal<number | null>(null);
  readonly javaInstalling = signal(false);
  readonly javaProgress = signal<JavaInstallProgress | null>(null);
  readonly javaRequirement = signal<{ instanceId: string; major: number } | null>(null);
  readonly javaRequirementLoading = signal(false);
  readonly managedJava = signal<RuntimeInfo | null>(null);
  readonly javaSetupNotice = signal('');
  readonly selectedJavaMajor = computed(() => this.javaRequirement()?.instanceId === this.s.selected()?.id
    ? this.javaRequirement()!.major : null);
  readonly javaStage = computed(() => ({ metadata: 'Passende Java-Version suchen', downloading: 'Java herunterladen',
    extracting: 'Java entpacken', verifying: 'Java prüfen', complete: 'Java eingerichtet' })[this.javaProgress()?.stage ?? 'metadata']);
  readonly javaProgressPercent = computed(() => this.javaProgress()?.totalBytes
    ? Math.min(100, Math.floor(this.javaProgress()!.downloadedBytes / this.javaProgress()!.totalBytes * 100)) : null);
  readonly backupList = signal<BackupSummary[]>([]);
  readonly backupListInstanceId = signal('');
  readonly backupsLoading = signal(false);
  readonly backupOperation = signal<'create' | 'restore' | null>(null);
  readonly backupError = signal('');
  readonly backupNotice = signal('');
  readonly lastContentBackup = signal<BackupSummary | null>(null);
  readonly updateChecks = signal<ModUpdateCheck[]>([]);
  readonly updateCheckKey = signal('');
  readonly updatesChecking = signal(false);
  readonly updatesChecked = signal(false);
  readonly updatesProgress = signal(0);
  readonly updateError = signal('');
  readonly updateNotice = signal('');
  readonly updateSelection = signal<ModUpdateCheck | null>(null);
  readonly updateApplying = signal(false);
  readonly updateApplyError = signal('');
  readonly profileSyncPlan = signal<ProfileSyncPlan | null>(null);
  readonly profileSyncChoices = signal<Record<string, ProfileSyncChoice>>({});
  readonly profileSyncError = signal('');
  readonly profileSyncApplying = signal(false);
  readonly profileBackupError = signal('');
  readonly profileBackupNotice = signal('');
  readonly exportBusy = signal(false);
  readonly exportError = signal('');
  readonly lastExportPath = signal('');
  readonly profileSyncResolved = computed(() => !!this.profileSyncPlan() && this.profileSyncPlan()!.entries
    .every(entry => entry.kind !== 'conflict' || (Object.hasOwn(this.profileSyncChoices(), entry.id)
      && (this.profileSyncChoices()[entry.id] === 'local' || this.profileSyncChoices()[entry.id] === 'incoming'))));
  readonly visibleUpdateChecks = computed(() => this.updateCheckKey() === this.modUpdateKey(this.s.selected()) ? this.updateChecks() : []);
  readonly updateResultsCurrent = computed(() => this.updateCheckKey() === this.modUpdateKey(this.s.selected()));
  readonly availableUpdates = computed(() => this.visibleUpdateChecks().filter(check => check.status === 'available'));
  readonly updateableMods = computed(() => this.s.selected()?.contents.filter(content => content.kind === 'mod'
    && !content.dependency && !content.versionId.startsWith('demo-')) ?? []);
  readonly operationBusy = computed(() => !!this.installingInstance() || !!this.contentInstallingId() || this.gameLaunching()
    || this.javaInstalling() || !!this.backupOperation() || this.updateApplying() || this.profileSyncApplying()
    || !!this.confirmation() || this.exportBusy() || this.curseforgeSaving());
  readonly selectedLocked = computed(() => this.operationBusy() || (this.game().running && this.game().instanceId === this.s.selected()?.id));
  readonly launchSupported = computed(() => ['vanilla', 'fabric'].includes(this.launchInstance()?.loader ?? ''));
  readonly launchHasExamples = computed(() => this.launchInstance()?.contents.some(content => content.enabled && content.versionId.startsWith('demo-')) ?? false);
  readonly launchInstallation = computed(() => {
    const instance = this.launchInstance(); const report = this.installation();
    return instance && report?.versionId === instance.mcVersion && (report.loader ?? 'vanilla') === instance.loader
      && (instance.loader !== 'fabric' || report.loaderVersion === instance.loaderVersion) ? report : null;
  });
  readonly newMinecraftFamily = signal('');
  readonly minecraftFamilies = computed(() => groupMinecraftVersions(this.minecraftCatalog()?.versions ?? [], this.includeSnapshots()));
  readonly minecraftFamilyOptions = computed(() => this.minecraftFamilies().map(family => ({
    value: family.id, label: family.label,
    description: family.versions.some(version => version.id === this.minecraftCatalog()?.latest.release)
      ? 'Aktuelle Version' : family.versions.some(version => version.type === 'release')
        ? `${family.versions.length} Versionen` : 'Snapshots & Vorabversionen',
    icon: family.versions.some(version => version.type === 'release') ? 'cube' : 'flask',
  })));
  readonly minecraftOptions = computed(() => {
    const catalog = this.minecraftCatalog();
    const family = this.minecraftFamilies().find(family => family.id === this.newMinecraftFamily());
    return (family?.versions ?? [])
      .map(version => ({ value: version.id, label: minecraftVersionLabel(version, family!.id),
        description: version.type === 'snapshot'
          ? `${version.id}${version.id === catalog?.latest.snapshot ? ' · Neuester Snapshot' : ''}`
          : `${version.id}${version.id === catalog?.latest.release ? ' · Aktuelle Version' : ' · Release'}`,
        icon: version.type === 'snapshot' ? 'flask' : 'cube',
      }));
  });
  readonly newMinecraftVersion = computed(() => this.minecraftCatalog()?.versions.find(version => version.id === this.newVersionValue()));
  readonly newVersionValue = signal('');
  readonly installProgressPercent = computed(() => {
    const progress = this.installProgress();
    return progress?.totalFiles ? Math.min(100, Math.floor(progress.completedFiles / progress.totalFiles * 100)) : 0;
  });
  readonly installStage = computed(() => ({ metadata: 'Versionsdaten laden', downloading: 'Spieldateien laden und prüfen',
    natives: 'Native Bibliotheken entpacken', loader: 'Fabric installieren', complete: 'Installation abgeschlossen' })[this.installProgress()?.stage ?? 'metadata']);
  readonly sceneImage = computed(() => {
    const instance = this.s.selected();
    return instance ? `url("${this.cover(instance)}")` : 'none';
  });
  readonly osLabel = computed(() => {
    const nativeOs = this.runtime()?.os;
    if (nativeOs) return ({ macos: 'macOS', windows: 'Windows', linux: 'Linux' } as Record<string, string>)[nativeOs] ?? nativeOs;
    return /Mac/i.test(navigator.platform) ? 'macOS' : /Win/i.test(navigator.platform) ? 'Windows' : 'Lokales Gerät';
  });
  readonly nav: { page: Page; label: string; icon: string }[] = [
    { page: 'start', label: 'Start', icon: 'house' },
    { page: 'instances', label: 'Instanzen', icon: 'stack' },
    { page: 'discover', label: 'Entdecken', icon: 'compass' },
    { page: 'sync', label: 'Geräte & Sync', icon: 'desktop' },
  ];
  readonly kinds: { id: ContentKind; label: string }[] = [
    { id: 'mod', label: 'Mods' }, { id: 'shader', label: 'Shader' },
    { id: 'resourcepack', label: 'Resource Packs' }, { id: 'datapack', label: 'Data Packs' },
  ];
  readonly filteredContents = computed(() => this.s.selected()?.contents.filter(c =>
    c.kind === this.kind() && c.name.toLowerCase().includes(this.contentQuery().toLowerCase())) ?? []);
  readonly title = computed(() => ({ start: 'Start', instances: 'Instanzen', discover: 'Entdecken', sync: 'Geräte & Sync', settings: 'Einstellungen' })[this.page()]);
  readonly version = computed(() => this.versions().find(v => v.id === this.selectedVersion()));
  readonly versionFile = computed(() => this.version()?.files.find(file => file.primary) ?? this.version()?.files[0]);
  readonly versionHashValid = computed(() => this.validProviderHash(this.versionSource(), this.versionFile()));
  readonly requiredDependencies = computed(() => this.version()?.dependencies.filter(d => d.dependency_type === 'required').length ?? 0);
  readonly addContentsTitle = computed(() => ({ mod: 'Mods hinzufügen', shader: 'Shader hinzufügen',
    resourcepack: 'Resource Packs hinzufügen', datapack: 'Data Packs hinzufügen' })[this.kind()]);
  readonly plannedContents = computed(() => this.s.selected()?.contents.filter(content => !content.versionId.startsWith('demo-')).length ?? 0);
  readonly instanceOptions = computed(() => this.s.instances().map(instance => ({
    value: instance.id, label: instance.name,
    description: `${instance.mcVersion} · ${this.loaderLabel(instance.loader)}`,
    image: this.thumb(instance),
  })));
  readonly loaderOptions = [
    { value: 'fabric', label: 'Fabric', icon: 'puzzle-piece' },
    { value: 'vanilla', label: 'Vanilla', icon: 'cube' },
  ];
  readonly versionOptions = computed(() => this.versions().map(version => ({
    value: version.id, label: version.version_number,
    description: `${version.version_type === 'release' ? 'Release' : version.version_type === 'beta' ? 'Beta' : 'Alpha'} · Minecraft ${this.s.selected()?.mcVersion ?? ''}`,
    icon: 'package',
  })));
  searchQuery = '';
  newName = ''; newLoader: Loader = 'fabric';
  serverName = ''; serverAddress = ''; formError = '';
  rename = '';
  importText = ''; importCount = 0;
  curseforgeKey = '';
  microsoftClientIdDraft = this.s.device().microsoftClientId;
  private request = 0;
  private versionRequest = 0;
  private installationRequest = 0;
  private versionContext?: { instanceId: string; kind: ContentKind; source: ContentSource; projectId: string; gameVersion: string; loader: Loader };
  private lastActive?: HTMLElement;
  private searchTimer?: ReturnType<typeof setTimeout>;
  private toastTimer?: ReturnType<typeof setTimeout>;
  private loginTimer?: ReturnType<typeof setTimeout>;
  private loginClock?: ReturnType<typeof setInterval>;
  private loginRequest = 0;
  private nativeUnlisteners: (() => void)[] = [];
  private destroyed = false;
  private javaRequirementRequest = 0;
  private backupListRequest = 0;
  private updateRequest = 0;
  private runtimeRequest = 0;
  private curseforgeStatusRequest = 0;
  private confirmationResolver?: (accepted: boolean) => void;
  private confirmationReturnFocus?: HTMLElement;

  constructor() {
    const initial = location.hash.slice(1) as Page;
    if (['start', 'instances', 'discover', 'sync', 'settings'].includes(initial)) this.page.set(initial);
    window.addEventListener('hashchange', () => {
      const next = location.hash.slice(1) as Page;
      if (['start', 'instances', 'discover', 'sync', 'settings'].includes(next)) {
        if (this.page() !== next) {
          this.activatePage(next);
        }
      }
    });
    if (this.page() === 'discover') void this.search();
    if (this.native) void this.initializeNative();
    effect(() => {
      if (this.dialog()) {
        this.lastActive = document.activeElement as HTMLElement;
        setTimeout(() => (document.querySelector<HTMLElement>('.modal input, .modal [role="combobox"]') ?? document.querySelector<HTMLElement>('.modal .button, .modal-close'))?.focus(), 0);
      }
    });
  }

  private async initializeNative(): Promise<void> {
    const listeners = await Promise.allSettled([
      listen<GameStatus>('minecraft-game-status', event => this.applyGameStatus(event.payload)),
      listen<GameLog>('minecraft-game-log', event => this.gameLogs.update(lines => [...lines.slice(-999), event.payload])),
      listen<string>('minecraft-close-blocked', event => this.notify(event.payload)),
    ]);
    for (const result of listeners) {
      if (result.status === 'fulfilled') {
        if (this.destroyed) result.value(); else this.nativeUnlisteners.push(result.value);
      }
    }
    await Promise.allSettled([
      this.refreshRuntime(), this.refreshAccount(), this.refreshCurseForgeStatus().then(() => {
        if (this.catalogVisible() && this.contentSource() !== 'modrinth') void this.search();
      }),
      this.minecraft.gameStatus().then(status => this.applyGameStatus(status)),
      this.minecraft.logs().then(lines => this.gameLogs.set(lines.slice(-1000))),
    ]);
    if (this.page() === 'settings') void this.loadJavaRequirement();
  }

  providerLabel(source: ContentSource): string { return source === 'curseforge' ? 'CurseForge' : 'Modrinth'; }

  private validProviderHash(source: ContentSource, file?: ModVersionFile): boolean {
    return source === 'curseforge' ? /^[a-fA-F0-9]{40}$/u.test(file?.hashes.sha1 ?? '')
      : /^[a-fA-F0-9]{128}$/u.test(file?.hashes.sha512 ?? '');
  }

  async setContentSource(value: string): Promise<void> {
    if (value !== 'all' && value !== 'modrinth' && value !== 'curseforge') return;
    if (this.dialog() === 'version') this.close();
    this.cancelSearch(); this.contentSource.set(value);
    if (value === 'curseforge' && this.native) await this.refreshCurseForgeStatus();
    if (this.contentSource() === value && this.catalogVisible()) void this.search();
  }

  async refreshCurseForgeStatus(): Promise<void> {
    if (!this.native || this.curseforgeSaving()) return;
    const ticket = ++this.curseforgeStatusRequest;
    this.curseforgeChecking.set(true);
    try {
      const status = await this.curseforge.status();
      if (ticket === this.curseforgeStatusRequest) this.curseforgeConfigured.set(status.configured);
    } catch (error) { if (ticket === this.curseforgeStatusRequest) this.curseforgeError.set(this.message(error)); }
    finally { if (ticket === this.curseforgeStatusRequest) this.curseforgeChecking.set(false); }
  }

  async saveCurseForgeKey(): Promise<void> {
    if (!this.native || this.operationBusy()) return;
    const key = this.curseforgeKey.trim(); this.curseforgeKey = '';
    if (!key) { this.curseforgeError.set('Bitte einen eigenen, für SyncMC freigegebenen API-Schlüssel eingeben.'); return; }
    this.curseforgeStatusRequest++; this.curseforgeChecking.set(false);
    this.curseforgeSaving.set(true); this.curseforgeError.set(''); this.curseforgeNotice.set('');
    try {
      const status = await this.curseforge.saveKey(key);
      this.curseforgeConfigured.set(status.configured);
      this.curseforgeNotice.set('Der CurseForge-Schlüssel wurde geprüft und im Schlüsselbund dieses Geräts gespeichert.');
    } catch (error) { this.curseforgeError.set(this.message(error).split(key).join('[entfernt]')); }
    finally { this.curseforgeKey = ''; this.curseforgeSaving.set(false); }
  }

  async clearCurseForgeKey(): Promise<void> {
    if (!this.native || this.operationBusy()) return;
    this.curseforgeStatusRequest++; this.curseforgeChecking.set(false); this.curseforgeKey = '';
    this.curseforgeSaving.set(true); this.curseforgeError.set(''); this.curseforgeNotice.set('');
    try {
      const status = await this.curseforge.clearKey();
      this.curseforgeConfigured.set(status.configured); this.curseforgeNotice.set('Der lokale CurseForge-Schlüssel wurde entfernt.');
    } catch (error) { this.curseforgeError.set(this.message(error)); }
    finally { this.curseforgeSaving.set(false); }
  }

  async openCurseForgeApplication(): Promise<void> {
    try { await this.curseforge.openApplication(); }
    catch (error) { this.curseforgeError.set(this.message(error)); }
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    this.resolveConfirmation(false);
    this.cancelSearch(); clearTimeout(this.toastTimer);
    this.cancelLogin();
    this.nativeUnlisteners.forEach(unlisten => unlisten());
  }

  private askConfirmation(confirmation: AppConfirmation): Promise<boolean> {
    if (this.destroyed || this.confirmation()) return Promise.resolve(false);
    this.confirmationReturnFocus = document.activeElement instanceof HTMLElement ? document.activeElement : undefined;
    const answer = new Promise<boolean>(resolve => { this.confirmationResolver = resolve; });
    this.confirmation.set(confirmation);
    setTimeout(() => {
      if (this.confirmation() === confirmation) document.querySelector<HTMLButtonElement>('.confirmation-cancel')?.focus();
    }, 0);
    return answer;
  }

  resolveConfirmation(accepted: boolean): void {
    const resolve = this.confirmationResolver;
    if (!resolve) return;
    const returnFocus = this.confirmationReturnFocus;
    this.confirmationResolver = undefined; this.confirmationReturnFocus = undefined;
    this.confirmation.set(null);
    resolve(accepted);
    if (!this.destroyed) setTimeout(() => {
      if (this.destroyed || this.confirmation()) return;
      if (returnFocus?.isConnected && !returnFocus.closest('[inert]')
        && !returnFocus.matches(':disabled')) returnFocus.focus();
      else (document.querySelector<HTMLElement>('.primary-modal .modal-close')
        ?? document.querySelector<HTMLElement>('.workspace button:not(:disabled)'))?.focus();
    }, 0);
  }

  async refreshRuntime(requiredMajor?: number): Promise<void> {
    if (!this.native || this.javaInstalling()) return;
    const ticket = ++this.runtimeRequest;
    const javaPath = this.s.device().javaPath;
    this.javaChecking.set(true); this.javaError.set(''); this.javaValidatedMajor.set(null);
    try {
      const info = requiredMajor
        ? await this.minecraft.validateJava(javaPath, requiredMajor)
        : await this.minecraft.runtime(javaPath);
      if (ticket !== this.runtimeRequest) return;
      this.runtime.set(info);
      if (requiredMajor) this.javaValidatedMajor.set(requiredMajor);
      if (!info.javaAvailable) this.javaError.set('Java wurde nicht gefunden. Trage den Pfad zur ausführbaren Java-Datei ein.');
    } catch (error) { if (ticket === this.runtimeRequest) this.javaError.set(this.message(error)); }
    finally { if (ticket === this.runtimeRequest) this.javaChecking.set(false); }
  }

  saveJavaPath(value: string): void {
    if (this.operationBusy() || this.game().running) return;
    try {
      this.s.updateDevice({ javaPath: value }); this.javaValidatedMajor.set(null);
      void this.refreshRuntime(this.dialog() === 'launch' ? this.launchInstallation()?.javaMajorVersion : this.selectedJavaMajor() ?? undefined);
    }
    catch (error) { this.javaError.set(this.message(error)); }
  }

  async loadJavaRequirement(): Promise<void> {
    const instance = this.s.selected(); const ticket = ++this.javaRequirementRequest;
    this.javaRequirement.set(null);
    if (!this.native || !instance) return;
    this.javaRequirementLoading.set(true);
    try {
      const report = await this.minecraft.installation(instance.id);
      if (ticket === this.javaRequirementRequest && report?.versionId === instance.mcVersion) {
        this.javaRequirement.set({ instanceId: instance.id, major: report.javaMajorVersion });
        void this.refreshRuntime(report.javaMajorVersion);
      }
    } catch (error) { if (ticket === this.javaRequirementRequest) this.javaError.set(this.message(error)); }
    finally { if (ticket === this.javaRequirementRequest) this.javaRequirementLoading.set(false); }
  }

  useAutomaticJava(): void {
    if (this.operationBusy() || this.game().running) return;
    this.s.updateDevice({ javaPath: '' });
    this.javaSetupNotice.set('Die automatische Java-Auswahl ist aktiv.');
    void this.refreshRuntime(this.dialog() === 'launch' ? this.launchInstallation()?.javaMajorVersion : this.selectedJavaMajor() ?? undefined);
  }

  async installJava(requiredMajor: number | null | undefined): Promise<void> {
    if (!this.native || !requiredMajor || this.operationBusy() || this.javaChecking() || this.game().running) return;
    this.javaInstalling.set(true); this.javaProgress.set(null); this.javaError.set(''); this.javaSetupNotice.set('');
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<JavaInstallProgress>('java-install-progress', event => {
        if (event.payload.major === requiredMajor) this.javaProgress.set(event.payload);
      });
      const info = await this.minecraft.installJava(requiredMajor);
      this.managedJava.set(info);
      if (!this.s.device().javaPath) {
        this.runtime.set(info); this.javaValidatedMajor.set(requiredMajor);
        this.javaSetupNotice.set(`Java ${requiredMajor} ist eingerichtet und wird automatisch verwendet.`);
      } else this.javaSetupNotice.set(`Java ${requiredMajor} ist eingerichtet. Dein manuell gewählter Java-Pfad bleibt aktiv; du kannst bewusst zur Automatik wechseln.`);
      this.notify(`Java ${requiredMajor} wurde eingerichtet und geprüft.`);
    } catch (error) { this.javaError.set(this.message(error)); }
    finally { unlisten?.(); this.javaInstalling.set(false); }
  }

  async loadBackups(instanceId = this.s.selected()?.id): Promise<void> {
    const ticket = ++this.backupListRequest;
    if (this.backupListInstanceId() !== instanceId) this.backupList.set([]);
    this.backupListInstanceId.set(instanceId ?? ''); this.backupError.set('');
    if (!this.native || !instanceId) return;
    this.backupsLoading.set(true);
    try {
      const backups = await this.minecraft.backups(instanceId);
      if (ticket === this.backupListRequest) this.backupList.set(backups.sort((a, b) => b.createdAt - a.createdAt));
    } catch (error) { if (ticket === this.backupListRequest) this.backupError.set(this.message(error)); }
    finally { if (ticket === this.backupListRequest) this.backupsLoading.set(false); }
  }

  async createBackup(): Promise<void> {
    const instance = this.s.selected();
    if (!this.native || !instance || this.operationBusy() || this.game().running) return;
    this.backupOperation.set('create'); this.backupError.set(''); this.backupNotice.set('');
    try {
      const backup = await this.minecraft.createBackup(instance);
      this.backupNotice.set(`„${backup.profileName}“ wurde mit ${backup.fileCount} Dateien gesichert.`);
      await this.loadBackups(instance.id);
    } catch (error) { this.backupError.set(this.message(error)); }
    finally { this.backupOperation.set(null); }
  }

  async restoreBackup(backup: BackupSummary): Promise<void> {
    const instance = this.s.selected();
    if (!this.native || !instance || instance.id !== backup.instanceId || this.operationBusy() || this.game().running) return;
    const expectedProfile = JSON.stringify(instance);
    const date = Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(backup.createdAt * 1000));
    if (!await this.askConfirmation({ title: 'Instanz wiederherstellen?', actionLabel: 'Instanz wiederherstellen', destructive: true,
      message: `„${backup.profileName}“ vom ${date} wiederherstellen? Der aktuelle lokale Instanzstand einschließlich Profil, Mods, Konfiguration und Welten wird ersetzt. Vorher werden eine Rettungssicherung des aktuellen Instanzordners und eine Profilsicherung angelegt.` })) return;
    if (this.operationBusy() || this.game().running || this.s.selected()?.id !== instance.id
      || JSON.stringify(this.s.instances().find(current => current.id === instance.id)) !== expectedProfile) {
      this.backupError.set('Die Instanz hat sich während der Bestätigung geändert. Bitte wähle die Sicherung erneut.'); return;
    }
    this.backupOperation.set('restore'); this.backupError.set(''); this.backupNotice.set('');
    try {
      this.s.backupProfiles('Vor Instanz-Wiederherstellung');
      const result = await this.minecraft.restoreBackup(instance.id, backup.id);
      const profile = parseManifest(JSON.stringify({ format: 'syncmc-manifest', version: 1, instances: [result.profile] })).instances[0];
      if (profile.id !== instance.id) throw new Error('Die Sicherung gehört zu einer anderen Instanz.');
      this.s.updateInstanceById(instance.id, { ...profile, contents: result.contents });
      if (this.s.persistenceError()) throw new Error('Die Dateien wurden wiederhergestellt, das Profil ist aber nur im Arbeitsspeicher gespeichert. Exportiere die Profile und prüfe den lokalen Speicher.');
      this.updateChecks.set([]); this.updateCheckKey.set(''); this.updatesChecked.set(false);
      if (this.launchInstance()?.id === instance.id) this.installation.set(result.requiresInstallation ? null : await this.minecraft.installation(instance.id));
      this.backupNotice.set(`Sicherung vom ${date} wiederhergestellt. Der vorherige Stand bleibt als Rettungssicherung erhalten.${result.requiresInstallation || result.requiresContentInstall ? ' Minecraft und Inhalte müssen vor dem nächsten Spielstart erneut installiert oder geprüft werden.' : ''}`);
      await this.loadBackups(instance.id);
    } catch (error) { this.backupError.set(this.message(error)); }
    finally { this.backupOperation.set(null); }
  }

  backupReason(reason: string): string {
    return ({ manual: 'Manuell erstellt', 'before-content-update': 'Vor Inhaltsänderung', 'before-content-change': 'Vor Inhaltsänderung',
      'before-restore': 'Rettungssicherung vor Wiederherstellung', 'rescue': 'Rettungssicherung' } as Record<string, string>)[reason] ?? reason;
  }

  backupSections(backup: BackupSummary): string {
    const names: Record<string, string> = { profile: 'Profil', mods: 'Mods', config: 'Konfiguration', worlds: 'Welten',
      saves: 'Welten', shaderpacks: 'Shader', resourcepacks: 'Resource Packs', settings: 'Einstellungen', options: 'Spieleinstellungen',
      servers: 'Serverdateien', 'instance-files': 'Instanzdateien' };
    return backup.containedSections.map(section => names[section] ?? section).join(' · ');
  }

  formatBytes(bytes: number): string {
    return bytes >= 1_000_000_000 ? `${(bytes / 1_000_000_000).toFixed(1)} GB`
      : bytes >= 1_000_000 ? `${(bytes / 1_000_000).toFixed(1)} MB` : `${Math.ceil(bytes / 1_000)} KB`;
  }

  private modUpdateKey(instance?: Instance): string {
    return instance ? JSON.stringify([instance.id, instance.mcVersion, instance.loader, instance.contents.filter(content =>
      content.kind === 'mod' && !content.dependency).map(content => [content.id, content.source, content.projectId, content.versionId]).sort()]) : '';
  }

  async checkModUpdates(): Promise<void> {
    const instance = this.s.selected();
    if (!instance || this.updatesChecking() || this.operationBusy()) return;
    const mods = this.updateableMods(); const ticket = ++this.updateRequest; const key = this.modUpdateKey(instance);
    this.updateCheckKey.set(key); this.updateChecks.set([]); this.updatesChecking.set(true); this.updatesChecked.set(false);
    this.updatesProgress.set(0); this.updateError.set(''); this.updateNotice.set('');
    try {
      for (let offset = 0; offset < mods.length; offset += 3) {
        const results = await Promise.all(mods.slice(offset, offset + 3).map(async (content): Promise<ModUpdateCheck> => {
          const base = { instanceId: instance.id, contentId: content.id, projectId: content.projectId, fromVersionId: content.versionId,
            source: content.source, name: content.name, currentVersion: content.version };
          try {
            if (content.source === 'curseforge' && !this.curseforgeConfigured()) throw new Error('Für CurseForge fehlt ein eigener, für SyncMC freigegebener API-Schlüssel.');
            const versions = await this.api.versions(content.source, content.projectId, instance.mcVersion, instance.loader, 'mod');
            const current = versions.find(version => version.id === content.versionId);
            const currentDate = Date.parse(current?.date_published ?? '');
            if (!Number.isFinite(currentDate)) return { ...base, status: 'error', message: 'Die bisherige Veröffentlichung lässt sich nicht sicher vergleichen. Wähle eine Version bei Bedarf manuell.' };
            const candidate = versions.filter(version => version.version_type === 'release' && version.id !== content.versionId
              && Date.parse(version.date_published ?? '') > currentDate)
              .sort((a, b) => Date.parse(b.date_published ?? '') - Date.parse(a.date_published ?? ''))[0];
            if (!candidate) return { ...base, status: 'current' };
            const installable = (file: ModVersionFile) => file.filename.toLowerCase().endsWith('.jar')
              && this.validProviderHash(content.source, file)
              && !['sources-jar', 'dev-jar', 'javadoc-jar', 'signature'].includes(file.file_type ?? '');
            const file = candidate.files.find(file => file.primary && installable(file)) ?? candidate.files.find(installable);
            return file ? { ...base, status: 'available', candidate, file }
              : { ...base, status: 'error', message: 'Die neue Veröffentlichung enthält keine verifizierbare Moddatei.' };
          } catch (error) { return { ...base, status: 'error', message: this.message(error) }; }
        }));
        if (ticket !== this.updateRequest || key !== this.modUpdateKey(this.s.selected())) return;
        this.updateChecks.update(checks => [...checks, ...results]); this.updatesProgress.set(offset + results.length);
      }
      this.updatesChecked.set(true);
    } finally { if (ticket === this.updateRequest) this.updatesChecking.set(false); }
  }

  reviewModUpdate(contentId: string): void {
    const update = this.visibleUpdateChecks().find(update => update.contentId === contentId);
    if (!update || update.status !== 'available' || this.selectedLocked()) return;
    this.updateSelection.set(update); this.updateApplyError.set(''); this.dialog.set('mod-update');
  }

  async applyModUpdate(): Promise<void> {
    const update = this.updateSelection(); const instance = this.s.selected();
    if (!this.native || !instance || !update?.candidate || !update.file || this.operationBusy() || this.game().running) return;
    const content = instance.contents.find(content => content.id === update.contentId);
    if (instance.id !== update.instanceId || content?.source !== update.source || content?.projectId !== update.projectId
      || content?.versionId !== update.fromVersionId) {
      this.updateApplyError.set('Das Profil hat sich geändert. Bitte prüfe die verfügbaren Updates erneut.'); return;
    }
    this.updateApplying.set(true); this.updateApplyError.set(''); this.lastContentBackup.set(null);
    try {
      const candidate: Instance = { ...instance, contents: instance.contents.map(content => content.id === update.contentId
        ? { ...content, versionId: update.candidate!.id, version: update.candidate!.version_number,
          filename: update.file!.filename, sha512: update.file!.hashes.sha512,
          sha1: update.source === 'curseforge' ? update.file!.hashes.sha1 : undefined, status: 'planned' as const } : content) };
      await this.syncInstanceContent(candidate, instance);
      this.updateChecks.set([]); this.updateCheckKey.set(''); this.updatesChecked.set(false);
      this.updateNotice.set(`${update.name} wurde auf ${update.candidate.version_number} aktualisiert.${this.lastContentBackup() ? ' Der vorherige Installationsstand ist gesichert.' : ''}`);
      if (this.dialog() === 'mod-update') this.close();
      this.notify(`${update.name} wurde aktualisiert.`);
    } catch (error) { this.updateApplyError.set(this.message(error)); }
    finally { this.updateApplying.set(false); }
  }

  saveClientId(value: string): void {
    try {
      this.s.updateDevice({ microsoftClientId: value });
      this.microsoftClientIdDraft = this.s.device().microsoftClientId;
      this.accountError.set('');
    }
    catch (error) { this.accountError.set(this.message(error)); }
  }

  async refreshAccount(): Promise<void> {
    if (!this.native) return;
    this.accountLoading.set(true);
    try { this.account.set(await this.minecraft.account()); }
    catch (error) { this.accountError.set(this.message(error)); }
    finally { this.accountLoading.set(false); }
  }

  openAccount(): void {
    this.microsoftClientIdDraft = this.s.device().microsoftClientId;
    this.dialog.set('account'); if (!this.loginChallenge()) void this.refreshAccount();
  }

  async beginLogin(): Promise<void> {
    if (!this.native || this.loginBusy() || this.loginChallenge()) return;
    this.loginBusy.set(true); this.accountError.set('');
    const ticket = ++this.loginRequest;
    try {
      const challenge = await this.minecraft.beginLogin(this.s.device().microsoftClientId);
      if (ticket !== this.loginRequest || this.destroyed) { await this.minecraft.cancelLogin(challenge.flowId); return; }
      this.loginChallenge.set(challenge);
      this.loginExpiresAt.set(Date.now() + challenge.expiresIn * 1000);
      this.loginSeconds.set(challenge.expiresIn);
      this.loginClock = setInterval(() => {
        this.loginSeconds.set(Math.max(0, Math.ceil((this.loginExpiresAt() - Date.now()) / 1000)));
      }, 1000);
      this.scheduleLoginPoll(ticket, Math.max(challenge.interval, 5));
      try { await this.minecraft.openLogin(); }
      catch { this.accountError.set(`Der Browser konnte nicht geöffnet werden. Öffne ${challenge.verificationUri} und gib den angezeigten Code ein.`); }
    } catch (error) { if (ticket === this.loginRequest) this.accountError.set(this.message(error)); }
    finally { if (ticket === this.loginRequest) this.loginBusy.set(false); }
  }

  private scheduleLoginPoll(ticket: number, interval: number): void {
    clearTimeout(this.loginTimer);
    this.loginTimer = setTimeout(() => void this.pollLogin(ticket, interval), interval * 1000);
  }

  private async pollLogin(ticket: number, interval: number): Promise<void> {
    const challenge = this.loginChallenge();
    if (!challenge || ticket !== this.loginRequest) return;
    if (Date.now() >= this.loginExpiresAt()) {
      this.cancelLogin(); this.accountError.set('Der Anmeldecode ist abgelaufen. Starte die Anmeldung erneut.'); return;
    }
    try {
      const result = await this.minecraft.pollLogin(challenge.flowId);
      if (ticket !== this.loginRequest) return;
      if (result.status === 'complete') {
        this.account.set(result.account ?? await this.minecraft.account());
        this.finishLogin(); this.accountError.set(''); this.notify('Dein Minecraft-Konto ist verbunden.');
      } else if (result.status === 'expired') {
        this.finishLogin(); this.accountError.set('Der Anmeldecode ist abgelaufen. Starte die Anmeldung erneut.');
      } else this.scheduleLoginPoll(ticket, result.status === 'slow_down' ? interval + 5 : interval);
    } catch (error) {
      if (ticket !== this.loginRequest) return;
      this.finishLogin(); this.accountError.set(this.message(error));
    }
  }

  private finishLogin(): void {
    clearTimeout(this.loginTimer); clearInterval(this.loginClock);
    this.loginChallenge.set(null); this.loginBusy.set(false); this.loginSeconds.set(0);
  }

  cancelLogin(): void {
    const challenge = this.loginChallenge();
    this.loginRequest++; this.finishLogin();
    if (challenge) void this.minecraft.cancelLogin(challenge.flowId).catch(() => {});
  }

  async reopenLogin(): Promise<void> {
    try { await this.minecraft.openLogin(); } catch (error) { this.accountError.set(this.message(error)); }
  }

  async openRegistration(): Promise<void> {
    try { await this.minecraft.openRegistration(); } catch (error) { this.accountError.set(this.message(error)); }
  }

  async logout(): Promise<void> {
    this.cancelLogin(); this.accountError.set(''); this.accountLoading.set(true);
    try { await this.minecraft.logout(); this.account.set(null); this.notify('Minecraft-Konto abgemeldet.'); }
    catch (error) { this.accountError.set(this.message(error)); }
    finally { this.accountLoading.set(false); }
  }

  async openLogs(): Promise<void> {
    this.dialog.set('logs');
    if (!this.native) return;
    this.logsLoading.set(true);
    try { this.gameLogs.set((await this.minecraft.logs()).slice(-1000)); }
    catch (error) { this.notify(this.message(error)); }
    finally { this.logsLoading.set(false); }
  }

  async copyLogs(): Promise<void> {
    try { await navigator.clipboard.writeText(this.gameLogs().map(log => `[${log.stream}] ${log.line}`).join('\n')); this.notify('Spielprotokoll kopiert.'); }
    catch { this.notify('Das Spielprotokoll konnte nicht kopiert werden.'); }
  }

  navigate(page: Page): void {
    this.activatePage(page); location.hash = page;
  }
  private activatePage(page: Page): void {
    this.cancelSearch();
    if (this.confirmation()) this.resolveConfirmation(false);
    if (this.dialog()) this.close();
    this.contentMode.set('library');
    this.page.set(page);
    if (page === 'settings') void this.loadJavaRequirement();
    if (this.catalogVisible()) void this.search();
  }
  select(id: string): void {
    if (this.dialog() === 'version') this.close();
    this.s.selectInstance(id);
    this.updateRequest++; this.updatesChecking.set(false); this.updatesChecked.set(false); this.updateChecks.set([]);
    this.updateNotice.set(''); this.backupNotice.set('');
    if (this.tab() === 'settings') this.rename = this.s.selected()?.name ?? '';
    if (this.tab() === 'backups') void this.loadBackups(id);
    if (this.page() === 'settings') void this.loadJavaRequirement();
    if (this.catalogVisible()) void this.search();
  }
  openInstance(id?: string): void { if (id) this.select(id); this.tab.set('contents'); this.navigate('instances'); }
  cover(instance: Instance): string { return `/assets/${instance.cover}.png`; }
  thumb(instance: Instance): string { return `/assets/${instance.cover}-thumb.png`; }
  loaderLabel(loader: string): string { return ({ fabric: 'Fabric', vanilla: 'Vanilla', forge: 'Forge', neoforge: 'NeoForge' } as Record<string, string>)[loader] ?? loader; }
  openCreate(): void {
    this.newName = ''; this.formError = ''; this.dialog.set('create');
    if (!this.minecraftCatalog()) void this.loadMinecraftVersions();
  }
  async loadMinecraftVersions(): Promise<void> {
    if (this.minecraftLoading()) return;
    this.minecraftLoading.set(true); this.minecraftError.set('');
    try {
      const catalog = await this.minecraft.catalog();
      this.minecraftCatalog.set(catalog);
      if (!catalog.versions.some(version => version.id === this.newVersionValue())) {
        this.newVersionValue.set(catalog.latest.release);
      }
      const family = this.minecraftFamilies().find(group => group.versions.some(version => version.id === this.newVersionValue()));
      this.newMinecraftFamily.set(family?.id ?? '');
    } catch (error) { this.minecraftError.set(this.message(error)); }
    finally { this.minecraftLoading.set(false); }
  }
  setSnapshots(include: boolean): void {
    this.includeSnapshots.set(include);
    const families = this.minecraftFamilies();
    const family = families.find(group => group.id === this.newMinecraftFamily())
      ?? families.find(group => group.versions.some(version => version.id === this.minecraftCatalog()?.latest.release));
    if (family) {
      this.newMinecraftFamily.set(family.id);
      if (!family.versions.some(version => version.id === this.newVersionValue())) this.setMinecraftFamily(family.id);
    }
  }
  setMinecraftFamily(id: string): void {
    const family = this.minecraftFamilies().find(group => group.id === id);
    if (!family) return;
    this.newMinecraftFamily.set(id);
    this.newVersionValue.set((family.versions.find(version => version.type === 'release') ?? family.versions[0]).id);
    this.formError = '';
  }
  create(): void {
    if (!this.minecraftOptions().some(version => version.value === this.newVersionValue())) {
      this.formError = 'Bitte eine verfügbare Minecraft-Version auswählen.'; return;
    }
    try { this.s.createInstance(this.newName, this.newVersionValue(), this.newLoader); this.close(); this.openInstance(); this.notify('Deine neue Instanz ist angelegt.'); }
    catch (e) { this.formError = this.message(e); }
  }
  async openLaunch(instance = this.s.selected()): Promise<void> {
    if (!instance) return;
    this.launchInstanceId.set(instance.id); this.installation.set(null); this.installationError.set('');
    this.gameError.set(''); this.fabricError.set(''); this.fabricVersions.set([]); this.fabricLoading.set(false);
    this.dialog.set('launch');
    const ticket = ++this.installationRequest;
    if (!this.native) return;
    this.installationLoading.set(true);
    try {
      const report = await this.minecraft.installation(instance.id);
      if (ticket !== this.installationRequest) return;
      if (report?.versionId === instance.mcVersion) {
        if (instance.loader === 'fabric' && !instance.loaderVersion && report.loaderVersion) {
          this.s.updateInstanceById(instance.id, { loaderVersion: report.loaderVersion });
        }
        this.installation.set(report);
        this.javaRequirement.set({ instanceId: instance.id, major: report.javaMajorVersion });
      }
      if (instance.loader === 'fabric') await this.loadFabricVersions(instance, ticket);
      void this.refreshAccount();
      void this.refreshRuntime(this.launchInstallation()?.javaMajorVersion);
    } catch (error) { if (ticket === this.installationRequest) this.installationError.set(this.message(error)); }
    finally { if (ticket === this.installationRequest) this.installationLoading.set(false); }
  }

  private async loadFabricVersions(instance: Instance, ticket: number): Promise<void> {
    this.fabricLoading.set(true);
    try {
      const loaders = await this.minecraft.fabricLoaders(instance.mcVersion);
      if (ticket !== this.installationRequest) return;
      this.fabricVersions.set(loaders);
      const current = this.launchInstance();
      if (current?.id === instance.id && !current.loaderVersion) {
        const preferred = loaders.find(loader => loader.stable) ?? loaders[0];
        if (preferred) this.s.updateInstanceById(instance.id, { loaderVersion: preferred.version });
      }
      if (!loaders.length) this.fabricError.set('Fabric bietet für diese Minecraft-Version noch keinen passenden Loader an.');
    } catch (error) { if (ticket === this.installationRequest) this.fabricError.set(this.message(error)); }
    finally { if (ticket === this.installationRequest) this.fabricLoading.set(false); }
  }

  setFabricVersion(version: string): void {
    const instance = this.launchInstance();
    if (!instance || this.operationBusy() || this.game().running) return;
    this.s.updateInstanceById(instance.id, { loaderVersion: version });
    this.installationError.set(''); this.javaValidatedMajor.set(null);
  }

  async installMinecraft(): Promise<void> {
    const instance = this.launchInstance();
    if (!this.native || !instance || !this.launchSupported() || this.operationBusy() || this.game().running) return;
    if (instance.loader === 'fabric' && !instance.loaderVersion) {
      this.installationError.set('Wähle zuerst eine verfügbare Fabric-Version.'); return;
    }
    this.installationRequest++; this.installationLoading.set(false);
    this.installingInstance.set(instance); this.installationError.set(''); this.installProgress.set(null);
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<InstallationProgress>('minecraft-install-progress', event => {
        if (event.payload.instanceId === instance.id) this.installProgress.set(event.payload);
      });
      const report = await this.minecraft.install(instance.id, instance.mcVersion, instance.loader, instance.loaderVersion);
      if (report.loaderVersion) this.s.updateInstanceById(instance.id, { loaderVersion: report.loaderVersion });
      if (this.launchInstance()?.id === instance.id) this.installation.set(report);
      this.javaRequirement.set({ instanceId: instance.id, major: report.javaMajorVersion });
      const current = this.s.instances().find(item => item.id === instance.id);
      if (current) await this.syncInstanceContent(current);
      await this.refreshRuntime(report.javaMajorVersion);
      this.notify(`Minecraft ${instance.mcVersion} und deine Inhalte wurden installiert und geprüft.`);
    } catch (error) {
      if (this.launchInstance()?.id === instance.id) this.installationError.set(this.message(error));
      this.notify(`Installation fehlgeschlagen: ${this.message(error)}`);
    } finally { unlisten?.(); this.installingInstance.set(null); }
  }

  private async syncInstanceContent(instance: Instance, expectedBefore = instance): Promise<Instance> {
    this.contentInstallingId.set(instance.id); this.contentProgress.set(null);
    let unlisten: (() => void) | undefined;
    try {
      unlisten = await listen<ContentProgress>('content-install-progress', event => {
        if (event.payload.instanceId === instance.id) this.contentProgress.set(event.payload);
      });
      const report = await this.minecraft.installContent(instance);
      const current = this.s.instances().find(item => item.id === instance.id);
      if (!current || JSON.stringify(current.contents) !== JSON.stringify(expectedBefore.contents)
        || current.mcVersion !== expectedBefore.mcVersion || current.loader !== expectedBefore.loader
        || current.loaderVersion !== expectedBefore.loaderVersion) {
        throw new Error('Das Instanzprofil hat sich während der Installation geändert. Bitte erneut abgleichen.');
      }
      this.s.updateInstanceById(instance.id, { contents: report.contents });
      if (report.backup) this.lastContentBackup.set(report.backup);
      if (this.s.persistenceError()) throw new Error('Die Dateien wurden aktualisiert, das Profil ist aber nur im Arbeitsspeicher gespeichert. Exportiere die Profile und prüfe den lokalen Speicher.');
      return this.s.instances().find(item => item.id === instance.id)!;
    } finally { unlisten?.(); this.contentInstallingId.set(''); }
  }

  async syncContents(): Promise<void> {
    const instance = this.dialog() === 'launch' ? this.launchInstance() : this.s.selected();
    if (!this.native || !instance || this.operationBusy() || this.game().running) return;
    this.installationError.set('');
    try { await this.syncInstanceContent(instance); this.notify('Inhalte und Pflichtabhängigkeiten wurden abgeglichen.'); }
    catch (error) { this.installationError.set(this.message(error)); this.notify(this.message(error)); }
  }

  async startGame(): Promise<void> {
    const instance = this.launchInstance(); const report = this.launchInstallation();
    if (!this.native || !instance || !report || !this.account() || this.operationBusy() || this.game().running) return;
    this.gameLaunching.set(true); this.gameError.set(''); this.javaError.set('');
    try {
      const info = await this.minecraft.validateJava(this.s.device().javaPath, report.javaMajorVersion);
      this.runtime.set(info); this.javaValidatedMajor.set(report.javaMajorVersion);
      const current = await this.syncInstanceContent(instance);
      this.gameLogs.set([]);
      this.applyGameStatus(await this.minecraft.launch(current, this.s.device()));
      this.notify(`${current.name} wird gestartet.`);
    } catch (error) { this.gameError.set(this.message(error)); }
    finally { this.gameLaunching.set(false); }
  }

  private applyGameStatus(status: GameStatus): void {
    if (!status.running || status.pid !== this.game().pid) this.gameStopRequested.set(false);
    this.game.set(status);
  }

  async stopGame(force = false): Promise<void> {
    if (!this.native || !this.game().running || this.gameStopping() || this.operationBusy()) return;
    const expectedGame = JSON.stringify([this.game().pid, this.game().instanceId, this.game().startedAt]);
    if (force && !await this.askConfirmation({ title: 'Minecraft sofort beenden?', actionLabel: 'Minecraft sofort beenden', destructive: true,
      message: 'Nicht gespeicherte Weltänderungen können verloren gehen. Beende die Welt nach Möglichkeit zuerst im Spiel.' })) return;
    if (this.operationBusy() || !this.game().running
      || JSON.stringify([this.game().pid, this.game().instanceId, this.game().startedAt]) !== expectedGame) {
      this.gameError.set('Der Spielprozess hat sich während der Bestätigung geändert. Bitte prüfe den aktuellen Spielstatus.'); return;
    }
    this.gameStopping.set(true); this.gameError.set('');
    try {
      await this.minecraft.stop(force);
      this.applyGameStatus(await this.minecraft.gameStatus());
      if (this.game().running) this.gameStopRequested.set(true);
    }
    catch (error) { this.gameError.set(this.message(error)); }
    finally { this.gameStopping.set(false); }
  }

  chooseExample(contentId: string): void {
    const instance = this.s.selected(); const content = instance?.contents.find(item => item.id === contentId);
    if (!content || this.selectedLocked()) return;
    this.kind.set(content.kind);
    void this.chooseVersion({ source: content.source, project_id: content.projectId, title: content.name, description: '',
      icon_url: content.iconUrl ?? '', author: '', downloads: 0, project_type: content.kind });
  }

  runningInstanceName(): string {
    return this.s.instances().find(instance => instance.id === this.game().instanceId)?.name ?? 'Minecraft';
  }

  editLaunchContents(): void {
    const instance = this.launchInstance();
    this.close();
    if (instance) this.openInstance(instance.id);
  }

  openServer(): void { this.serverName = ''; this.serverAddress = ''; this.formError = ''; this.dialog.set('server'); }
  addServer(): void {
    if (!this.s.addServer(this.serverName, this.serverAddress)) { this.formError = 'Bitte einen Namen und eine gültige, noch nicht gespeicherte Serveradresse eingeben.'; return; }
    this.close(); this.notify('Serverfavorit gespeichert.');
  }
  async copyAddress(address: string, label = 'Serveradresse'): Promise<void> {
    try { await navigator.clipboard.writeText(address); this.notify(`${label} kopiert.`); }
    catch { this.notify('Die Zwischenablage ist nicht verfügbar.'); }
  }
  saveName(): void {
    try { this.s.updateInstance({ name: this.rename }); this.notify('Instanz umbenannt.'); }
    catch (e) { this.notify(this.message(e)); }
  }
  setTab(tab: string): void {
    if (tab !== 'contents') { this.contentMode.set('library'); this.cancelSearch(); }
    this.tab.set(tab);
    if (tab === 'settings') this.rename = this.s.selected()?.name ?? '';
    if (tab === 'backups') void this.loadBackups();
  }
  setKind(kind: ContentKind): void {
    clearTimeout(this.searchTimer);
    this.kind.set(kind);
    if (this.catalogVisible()) void this.search();
  }
  openContentBrowser(): void {
    if (!this.s.selected()) { this.openCreate(); return; }
    if (this.page() !== 'instances') this.navigate('instances');
    this.tab.set('contents');
    this.contentMode.set('browse');
    void this.search();
    setTimeout(() => document.querySelector<HTMLInputElement>('#catalog-search')?.focus(), 0);
  }
  showContentLibrary(): void {
    this.cancelSearch();
    this.contentMode.set('library');
    setTimeout(() => document.querySelector<HTMLButtonElement>('[data-add-contents]')?.focus(), 0);
  }
  private cancelSearch(): void {
    clearTimeout(this.searchTimer);
    this.request++;
    this.loading.set(false);
    this.results.set([]);
    this.searchError.set('');
    this.searchWarning.set('');
    this.catalogChoices.set({});
  }
  queueSearch(): void {
    clearTimeout(this.searchTimer);
    this.request++;
    this.searchTimer = setTimeout(() => void this.search(), 300);
  }
  async search(): Promise<void> {
    if (!this.catalogVisible()) return;
    clearTimeout(this.searchTimer);
    const ticket = ++this.request;
    const instance = this.s.selected();
    if (!instance) { this.results.set([]); this.loading.set(false); return; }
    if (this.catalogUnavailable()) { this.results.set([]); this.loading.set(false); this.searchError.set(''); this.searchWarning.set(''); return; }
    const selection = this.contentSource();
    const sources: ContentSource[] = selection === 'all'
      ? this.curseforgeUnavailable() ? ['modrinth'] : ['modrinth', 'curseforge'] : [selection];
    this.loading.set(true); this.searchError.set(''); this.searchWarning.set(''); this.results.set([]); this.searched.set(true);
    const groups: (CatalogHit[] | undefined)[] = sources.map(() => undefined);
    const failures: string[] = [];
    let pending = sources.length;
    const publish = () => {
      if (ticket !== this.request) return;
      // Preserve each provider's ranking and publish fast results before slower sources finish.
      // Source + project ID remains the identity, including projects offered on both sites.
      const hits: CatalogHit[] = [];
      for (let index = 0; index < Math.max(0, ...groups.map(group => group?.length ?? 0)); index++) {
        for (const group of groups) if (group?.[index]) hits.push(group[index]);
      }
      this.results.set(hits);
      const succeeded = groups.some(group => group !== undefined);
      if (!pending && !succeeded) this.searchError.set(failures.join(' '));
      else if (failures.length) this.searchWarning.set(`Ein Teil der Suche ist nicht verfügbar. ${failures.join(' ')} ${pending ? 'Die übrigen Anbieter werden weiter durchsucht.' : 'Die Ergebnisse der übrigen Anbieter bleiben verfügbar.'}`);
      this.loading.set(pending > 0);
    };
    await Promise.allSettled(sources.map(async (source, index) => {
      try { groups[index] = await this.api.search(source, this.searchQuery, instance.mcVersion, instance.loader, this.kind()); }
      catch (error) { failures.push(`${this.providerLabel(source)}: ${this.message(error)}`); }
      finally { pending--; publish(); }
    }));
    if (ticket === this.request && selection === 'all') {
      const ids = candidateProjectIds(this.results());
      if (!ids.length) return;
      try {
        const links = await this.api.projectLinks(ids);
        if (ticket !== this.request) return;
        const byId = new Map(links.map(link => [link.project_id, link.source_url]));
        this.results.update(hits => hits.map(hit => hit.source === 'modrinth' && byId.has(hit.project_id)
          ? { ...hit, source_url: byId.get(hit.project_id) } : hit));
      } catch {
        // Optional display grouping must never hide an otherwise valid search result.
      }
    }
  }
  selectCatalogProvider(key: string, hit: CatalogHit): void {
    this.catalogChoices.update(choices => ({ ...choices, [`${this.s.selected()?.id}:${key}`]: `${hit.source}:${hit.project_id}` }));
  }
  chooseCatalogVersion(key: string): void {
    // Resolve the selection now, including a source change made before the next render frame.
    const row = this.catalogRows().find(candidate => candidate.key === key);
    if (row) void this.chooseVersion(row.hit);
  }
  hasProject(id: string, source: ContentSource): boolean {
    return this.s.selected()?.contents.some(content => content.source === source && content.projectId === id
      && !content.versionId.startsWith('demo-')) ?? false;
  }
  async chooseVersion(hit: CatalogHit): Promise<void> {
    const instance = this.s.selected(); if (!instance) return;
    const ticket = ++this.versionRequest;
    const kind = this.kind();
    const source = hit.source;
    this.versionContext = { instanceId: instance.id, kind, source, projectId: hit.project_id, gameVersion: instance.mcVersion, loader: instance.loader };
    this.versionHit.set({ ...hit, source }); this.versions.set([]); this.selectedVersion.set('');
    this.versionError.set(''); this.versionLoading.set(true); this.dialog.set('version');
    try {
      const versions = await this.api.versions(source, hit.project_id, instance.mcVersion, instance.loader, kind);
      if (ticket !== this.versionRequest) return;
      this.versions.set(versions); this.selectedVersion.set(versions[0]?.id ?? '');
    } catch (e) { if (ticket === this.versionRequest) this.versionError.set(this.message(e)); }
    finally { if (ticket === this.versionRequest) this.versionLoading.set(false); }
  }
  addVersion(): void {
    if (this.selectedLocked()) { this.versionError.set('Bitte warte, bis die Installation oder das Spiel beendet ist.'); return; }
    if (this.alternatePinnedSource()) { this.versionError.set('Diese Mod ist bereits über eine andere Quelle vorgemerkt. Entferne diesen Eintrag zuerst, wenn du die Quelle wechseln möchtest.'); return; }
    const hit = this.versionHit(); const version = this.version();
    const context = this.versionContext;
    if (!context || context.instanceId !== this.s.selected()?.id || context.kind !== this.kind()
      || context.gameVersion !== this.s.selected()?.mcVersion || context.loader !== this.s.selected()?.loader
      || context.source !== hit?.source || context.projectId !== hit?.project_id || context.source !== version?.source) {
      this.versionError.set('Die Instanz hat sich geändert. Bitte wähle die Version erneut.'); return;
    }
    const file = this.versionFile();
    if (!hit || !version || !file) return;
    if (!this.validProviderHash(context.source, file)) { this.versionError.set('Die Quelldatei enthält keine gültige Anbieter-Prüfsumme.'); return; }
    try {
      this.s.addContent({ id: `${context.source === 'curseforge' ? 'cf' : 'modrinth'}-${crypto.randomUUID()}`, projectId: hit.project_id, versionId: version.id,
        name: hit.title, version: version.version_number, filename: file.filename, iconUrl: hit.icon_url,
        kind: context.kind, enabled: true, sha512: file.hashes.sha512,
        sha1: context.source === 'curseforge' ? file.hashes.sha1 : undefined, source: context.source, status: 'planned' });
      this.close(); this.notify(`${hit.title} ist mit genauer Version vorgemerkt.`);
    } catch (e) { this.versionError.set(this.message(e)); }
  }
  private async downloadJsonFile(value: string, filename: string): Promise<string | null> {
    if (this.confirmation()) throw new Error('Beende zuerst die ausstehende Bestätigung.');
    if (this.exportBusy()) throw new Error('Es wird bereits eine Datei exportiert. Bitte kurz warten.');
    this.exportBusy.set(true); this.exportError.set(''); this.lastExportPath.set('');
    try {
      if (this.native) {
        const result = await this.minecraft.saveProfileExport(value, filename);
        this.lastExportPath.set(result.path);
        return result.path;
      }
      const blob = new Blob([value], { type: 'application/json' });
      const url = URL.createObjectURL(blob); const a = document.createElement('a');
      a.href = url; a.download = filename; a.click();
      setTimeout(() => URL.revokeObjectURL(url), 1000);
      return null;
    } catch (error) { this.exportError.set(this.message(error)); throw error; }
    finally { this.exportBusy.set(false); }
  }

  async downloadManifest(): Promise<void> {
    try {
      const path = await this.downloadJsonFile(this.s.exportManifest(), 'syncmc-instanzen.json');
      this.notify(path ? `Profile gespeichert: ${path}` : 'Instanzen als Manifest exportiert.');
    } catch (error) { this.notify(`Export fehlgeschlagen: ${this.message(error)}`); }
  }

  async downloadWorkspaceRecovery(): Promise<void> {
    const raw = this.s.workspaceRecoveryText();
    if (raw === null) return;
    try {
      const path = await this.downloadJsonFile(raw, 'syncmc-wiederherstellung.json');
      this.notify(path ? `Wiederherstellungsdaten gespeichert: ${path}` : 'Wiederherstellungsdaten exportiert.');
    } catch (error) { this.notify(`Export fehlgeschlagen: ${this.message(error)}`); }
  }

  async openExportFolder(): Promise<void> {
    if (!this.native) return;
    try { await this.minecraft.openProfileExports(); }
    catch (error) { this.exportError.set(this.message(error)); this.notify(this.message(error)); }
  }

  createProfileBackup(): void {
    if (this.operationBusy() || this.game().running) return;
    this.profileBackupError.set(''); this.profileBackupNotice.set('');
    try {
      this.s.backupProfiles('Manuelle Profilsicherung');
      this.profileBackupNotice.set('Die aktuellen Instanzprofile wurden auf diesem Gerät gesichert.');
    } catch (error) { this.profileBackupError.set(this.message(error)); }
  }

  async downloadProfileBackup(backup: ProfileBackup): Promise<void> {
    try {
      const path = await this.downloadJsonFile(this.s.exportProfileBackup(backup.id), `syncmc-profilsicherung-${backup.id}.json`);
      this.notify(path ? `Profilsicherung gespeichert: ${path}` : 'Profilsicherung als Datei exportiert.');
    } catch (error) { this.profileBackupError.set(this.message(error)); }
  }

  async restoreProfileBackup(backup: ProfileBackup): Promise<void> {
    if (this.operationBusy() || this.game().running) return;
    const expectedProfiles = this.s.exportManifest();
    if (!await this.askConfirmation({ title: 'Instanzprofile wiederherstellen?', actionLabel: 'Profile wiederherstellen', destructive: true,
      message: `Die gesamte aktuelle Profilliste durch die Sicherung vom ${Intl.DateTimeFormat('de-DE', { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(backup.createdAt))} ersetzen? Zuerst wird eine Rettungssicherung der aktuellen Profile angelegt. Minecraft-Dateien, Welten und Geräteeinstellungen werden dabei nicht wiederhergestellt.` })) return;
    if (this.operationBusy() || this.game().running || this.s.exportManifest() !== expectedProfiles
      || !this.s.profileBackups().some(current => current.id === backup.id && current.createdAt === backup.createdAt)) {
      this.profileBackupError.set('Die Profile oder die Sicherung haben sich während der Bestätigung geändert. Bitte wähle die Sicherung erneut.'); return;
    }
    this.profileBackupError.set(''); this.profileBackupNotice.set('');
    try {
      this.s.restoreProfileBackup(backup.id);
      this.installation.set(null); this.updateChecks.set([]); this.updatesChecked.set(false);
      this.profileBackupNotice.set('Instanzprofile wiederhergestellt. Eine Rettungssicherung der vorherigen Profile bleibt erhalten.');
    } catch (error) { this.profileBackupError.set(this.message(error)); }
  }

  async removeProfileBackup(backup: ProfileBackup): Promise<void> {
    if (this.operationBusy() || this.game().running) return;
    if (!await this.askConfirmation({ title: 'Profilsicherung löschen?', actionLabel: 'Sicherung löschen', destructive: true,
      message: 'Diese lokale Profilsicherung löschen? Bereits exportierte Dateien und deine aktuellen Instanzprofile bleiben erhalten.' })) return;
    if (this.operationBusy() || this.game().running
      || !this.s.profileBackups().some(current => current.id === backup.id && current.createdAt === backup.createdAt)) {
      this.profileBackupError.set('Die Sicherung oder der Spielstatus hat sich geändert. Bitte wähle die Sicherung erneut.'); return;
    }
    try { this.s.removeProfileBackup(backup.id); this.profileBackupError.set(''); }
    catch (error) { this.profileBackupError.set(this.message(error)); }
  }

  async readImport(event: Event): Promise<void> {
    const input = event.target as HTMLInputElement; const file = input.files?.[0]; if (!file) return;
    if (this.operationBusy() || this.game().running) { input.value = ''; this.notify('Bitte beende zuerst das Spiel oder die laufende Installation.'); return; }
    try {
      if (file.size > 5_000_000) throw new Error('Die Datei darf höchstens 5 MB groß sein.');
      this.importText = await file.text();
      const plan = this.s.prepareProfileSync(this.importText);
      this.profileSyncPlan.set(plan); this.profileSyncChoices.set({}); this.profileSyncError.set('');
      this.importCount = plan.incoming.length; this.dialog.set('import');
    } catch (error) { this.notify(this.message(error)); }
    input.value = '';
  }

  chooseProfileSync(instanceId: string, choice: ProfileSyncChoice): void {
    this.profileSyncChoices.update(choices => ({ ...choices, [instanceId]: choice }));
  }

  confirmImport(): void {
    const plan = this.profileSyncPlan();
    if (!plan || !this.profileSyncResolved() || this.operationBusy() || this.game().running) return;
    this.profileSyncApplying.set(true); this.profileSyncError.set('');
    try {
      const previousBackupCount = this.s.profileBackups().length;
      this.s.applyProfileSync(plan, this.profileSyncChoices());
      this.profileSyncPlan.set(null); this.close(); this.navigate('sync'); this.searched.set(false);
      this.profileBackupNotice.set(this.s.profileBackups().length > previousBackupCount
        ? 'Profile aus der Datei wurden abgeglichen. Der vorherige Profilstand ist lokal gesichert.'
        : 'Die gewählten Profile entsprechen bereits dem lokalen Stand. Es wurden keine Änderungen übernommen.');
      this.notify('Dateiabgleich abgeschlossen. Geräteeinstellungen bleiben lokal.');
    } catch (error) { this.profileSyncError.set(this.message(error)); }
    finally { this.profileSyncApplying.set(false); }
  }
  close(): void {
    if (this.confirmation()) { this.resolveConfirmation(false); return; }
    if (this.dialog() === 'account') this.cancelLogin();
    this.dialog.set(null); this.formError = ''; this.versionRequest++; this.versionContext = undefined;
    if (this.lastActive?.isConnected) this.lastActive.focus();
  }
  onEscape(event: Event): void {
    if (!event.defaultPrevented && this.confirmation()) {
      event.preventDefault(); event.stopPropagation(); this.resolveConfirmation(false); return;
    }
    if (!event.defaultPrevented && this.dialog()) this.close();
  }
  @HostListener('document:keydown', ['$event'])
  trapFocus(event: KeyboardEvent): void {
    if ((!this.dialog() && !this.confirmation()) || event.key !== 'Tab' || event.defaultPrevented) return;
    const activeDialog = document.querySelector<HTMLElement>(this.confirmation() ? '.confirmation-modal' : '.primary-modal');
    const controls = [...activeDialog?.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), a[href], summary, [tabindex]:not([tabindex="-1"])') ?? []]
      .filter(element => element.tabIndex >= 0 && element.getClientRects().length > 0);
    const first = controls[0]; const last = controls.at(-1);
    if (event.shiftKey && document.activeElement === first) { last?.focus(); event.preventDefault(); }
    else if (!event.shiftKey && document.activeElement === last) { first?.focus(); event.preventDefault(); }
    else if (!controls.includes(document.activeElement as HTMLElement)) { first?.focus(); event.preventDefault(); }
  }
  notify(text: string): void { clearTimeout(this.toastTimer); this.toast.set(text); this.toastTimer = setTimeout(() => this.toast.set(''), 4500); }
  message(error: unknown): string { return error instanceof Error ? error.message : String(error); }
  iconFor(name: string): string {
    if (/iris|shader/i.test(name)) return 'sun-horizon';
    if (/sodium|lithium|fast/i.test(name)) return 'lightning';
    if (/fabric|api/i.test(name)) return 'puzzle-piece';
    if (/menu/i.test(name)) return 'list-bullets';
    return 'cube';
  }
  formatDownloads(count: number): string { return Intl.NumberFormat('de', { notation: 'compact', maximumFractionDigits: 1 }).format(count); }
}
