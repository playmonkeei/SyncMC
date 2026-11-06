import type { InstalledContent, Instance } from './models';

export type ProfileSyncChoice = 'local' | 'incoming';
export interface ProfileSyncEntry {
  id: string;
  name: string;
  kind: 'added' | 'identical' | 'conflict';
  local?: Instance;
  incoming: Instance;
  changes: string[];
}
export interface ProfileSyncPlan {
  localSnapshot: string;
  incoming: Instance[];
  entries: ProfileSyncEntry[];
  keptLocalCount: number;
}
export interface ProfileBackup {
  id: string;
  createdAt: string;
  reason: string;
  instanceCount: number;
}

// Compare only portable desired state. A download receipt and device settings
// never prove that files exist on another computer and never belong in a sync.
function portableContent(item: InstalledContent): object {
  return {
    id: item.id, projectId: item.projectId, versionId: item.versionId,
    name: item.name, version: item.version, filename: item.filename,
    kind: item.kind, enabled: item.enabled, source: item.source,
    // CurseForge publishes SHA-1. Its computed local SHA-512 is a byte receipt,
    // rather than another remote pin; installing the same file adds no conflict.
    sha512: item.source === 'curseforge' ? null : item.sha512 ?? null,
    sha1: item.source === 'curseforge' ? item.sha1 ?? null : null,
    dependency: item.dependency === true,
    iconUrl: item.iconUrl ?? null,
  };
}

export function profileFingerprint(instance: Instance): string {
  return JSON.stringify({
    id: instance.id, name: instance.name, mcVersion: instance.mcVersion,
    loader: instance.loader, loaderVersion: instance.loaderVersion ?? null,
    cover: instance.cover,
    contents: [...instance.contents].sort((a, b) => a.id.localeCompare(b.id)).map(portableContent),
    servers: [...instance.servers].sort((a, b) => a.id.localeCompare(b.id))
      .map(server => ({ id: server.id, name: server.name, address: server.address })),
  });
}

function workspaceFingerprint(instances: Instance[]): string {
  return JSON.stringify([...instances].sort((a, b) => a.id.localeCompare(b.id)).map(profileFingerprint));
}

function changedFields(local: Instance, incoming: Instance): string[] {
  const fields: string[] = [];
  if (local.name !== incoming.name) fields.push('Name');
  if (local.mcVersion !== incoming.mcVersion) fields.push('Minecraft-Version');
  if (local.loader !== incoming.loader || local.loaderVersion !== incoming.loaderVersion) fields.push('Modloader');
  if (local.cover !== incoming.cover) fields.push('Cover');
  const contentKey = (instance: Instance) => JSON.stringify([...instance.contents]
    .sort((a, b) => a.id.localeCompare(b.id)).map(portableContent));
  if (contentKey(local) !== contentKey(incoming)) fields.push('Inhalte und Versionspins');
  const serverKey = (instance: Instance) => JSON.stringify([...instance.servers]
    .sort((a, b) => a.id.localeCompare(b.id)).map(server => [server.id, server.name, server.address]));
  if (serverKey(local) !== serverKey(incoming)) fields.push('Serverfavoriten');
  return fields;
}

/** Inputs are validated by the manifest parser before entering this pure comparison. */
export function prepareProfileSync(local: Instance[], incoming: Instance[]): ProfileSyncPlan {
  const localById = new Map(local.map(instance => [instance.id, instance]));
  const incomingIds = new Set(incoming.map(instance => instance.id));
  return structuredClone({
    localSnapshot: workspaceFingerprint(local), incoming,
    keptLocalCount: local.filter(instance => !incomingIds.has(instance.id)).length,
    entries: incoming.map(instance => {
      const previous = localById.get(instance.id);
      const equal = previous && profileFingerprint(previous) === profileFingerprint(instance);
      return {
        id: instance.id, name: instance.name,
        kind: !previous ? 'added' : equal ? 'identical' : 'conflict',
        ...(previous ? { local: previous } : {}), incoming: instance,
        changes: previous ? changedFields(previous, instance) : [],
      };
    }),
  });
}

export function mergeProfileSync(local: Instance[], plan: ProfileSyncPlan,
  choices: Record<string, ProfileSyncChoice>): Instance[] {
  if (workspaceFingerprint(local) !== plan.localSnapshot) {
    throw new Error('Die lokalen Profile wurden inzwischen geändert. Bitte die Datei erneut vergleichen.');
  }
  // Recalculate every decision from the inputs, rather than trusting UI labels.
  const current = prepareProfileSync(local, plan.incoming);
  const decisions = new Map<string, Instance>();
  for (const entry of current.entries) {
    if (entry.kind === 'identical') continue;
    if (entry.kind === 'conflict') {
      const choice = Object.hasOwn(choices, entry.id) ? choices[entry.id] : undefined;
      if (choice !== 'local' && choice !== 'incoming') {
        throw new Error(`Bitte entscheide für „${entry.name}“, welches Profil übernommen werden soll.`);
      }
      if (choice === 'local') continue;
    }
    const incoming = structuredClone(entry.incoming);
    // Identical local bytes may retain their local display state. A changed pin
    // always becomes planned and still needs native download/hash verification.
    incoming.contents = incoming.contents.map(item => {
      const previous = entry.local?.contents.find(candidate => candidate.id === item.id);
      const unchanged = previous && JSON.stringify(portableContent(previous)) === JSON.stringify(portableContent(item));
      return { ...item,
        ...(unchanged && item.source === 'curseforge' && previous.sha512 ? { sha512: previous.sha512 } : {}),
        status: unchanged ? previous.status : 'planned' };
    });
    decisions.set(entry.id, incoming);
  }
  const result = local.map(instance => decisions.get(instance.id) ?? structuredClone(instance));
  const localIds = new Set(local.map(instance => instance.id));
  for (const entry of current.entries) {
    if (!localIds.has(entry.id)) result.push(decisions.get(entry.id)!);
  }
  return result;
}
