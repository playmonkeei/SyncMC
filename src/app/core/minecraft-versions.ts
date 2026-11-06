import type { MinecraftVersion } from './minecraft.service';

export interface MinecraftVersionGroup {
  id: string;
  label: string;
  versions: MinecraftVersion[];
}

function versionFamily(id: string): { id: string; label: string; numeric: boolean } {
  const numeric = /^(\d+\.\d+)(?=$|[.\s-])/u.exec(id);
  if (numeric) return { id: numeric[1], label: numeric[1], numeric: true };

  // Weekly IDs describe their publication week, not the release they target.
  const weekly = /^(\d{2})w\d{2}[a-z]$/iu.exec(id);
  if (weekly) {
    const year = `20${weekly[1]}`;
    return { id: `snapshots-${year}`, label: `Snapshots ${year}`, numeric: false };
  }

  return { id: 'special', label: 'Sonderversionen', numeric: false };
}

/** Keep Mojang's chronological order, with weekly/special collections after release families. */
export function groupMinecraftVersions(
  versions: MinecraftVersion[],
  includeSnapshots: boolean,
): MinecraftVersionGroup[] {
  const numericGroups = new Map<string, MinecraftVersionGroup>();
  const otherGroups = new Map<string, MinecraftVersionGroup>();

  for (const version of versions) {
    if (!includeSnapshots && version.type === 'snapshot') continue;
    const family = versionFamily(version.id);
    const groups = family.numeric ? numericGroups : otherGroups;
    let group = groups.get(family.id);
    if (!group) {
      group = { id: family.id, label: family.label, versions: [] };
      groups.set(family.id, group);
    }
    group.versions.push(version);
  }

  return [...numericGroups.values(), ...otherGroups.values()];
}

/** Shorten only the selected family itself; retain patch numbers to avoid ambiguous choices. */
export function minecraftVersionLabel(version: MinecraftVersion, familyId: string): string {
  if (version.type === 'release') return version.id === familyId ? 'Release' : version.id;

  const preview = /^(\d+\.\d+(?:\.\d+)*)(?:-|\s+)(?:snapshot-?(\d+)|pre(?:[ -]release)?[ -]?(\d+)|rc-?(\d+)|release[ -]candidate[ -]?(\d+))$/iu.exec(version.id);
  if (!preview) return version.id;
  const prefix = preview[1] === familyId ? '' : `${preview[1]} `;
  if (preview[2]) return `${prefix}Snapshot ${preview[2]}`;
  if (preview[3]) return `${prefix}Pre-Release ${preview[3]}`;
  return `${prefix}Release Candidate ${preview[4] ?? preview[5]}`;
}
