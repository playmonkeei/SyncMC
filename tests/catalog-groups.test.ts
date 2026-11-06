import assert from 'node:assert/strict';
import test from 'node:test';
import { canonicalRepository, candidateProjectIds, groupCatalogHits } from '../src/app/core/catalog-groups.ts';
import type { CatalogHit } from '../src/app/core/content.service.ts';

function hit(source: CatalogHit['source'], id: string, patch: Partial<CatalogHit> = {}): CatalogHit {
  return { source, project_id: id, title: 'Sodium', project_type: 'mod', description: '', icon_url: '',
    author: 'Author', downloads: 1, ...patch };
}

test('canonicalizes supported repository roots, including GitHub case, .git, and GitLab subgroups', () => {
  assert.equal(canonicalRepository('https://GitHub.com/CaffeineMC/Sodium.GIT/'), 'https://github.com/caffeinemc/sodium');
  assert.equal(canonicalRepository('https://gitlab.com/group/nested/project.git/'), 'https://gitlab.com/group/nested/project');
  assert.equal(canonicalRepository('https://codeberg.org/owner/project.git'), 'https://codeberg.org/owner/project');
  assert.equal(canonicalRepository('https://bitbucket.org/owner/project/'), 'https://bitbucket.org/owner/project');
  assert.equal(canonicalRepository(' https://github.com/owner/project '), 'https://github.com/owner/project');
});

test('rejects untrusted hosts, authentication, query/fragment, encoded paths, and repository subpages', () => {
  for (const value of [null, undefined, 1, {}, '', 'http://github.com/owner/project', 'https://github.com.evil.test/owner/project',
    'https://www.github.com/owner/project', 'https://user:secret@github.com/owner/project', 'https://@github.com/owner/project',
    'https://github.com:8443/owner/project', 'https://github.com/owner/project?', 'https://github.com/owner/project#',
    'https://github.com/owner/project?tab=readme', 'https://github.com/owner/project/issues',
    'https://github.com/owner/project/tree/main', 'https://github.com/a/../owner/project',
    'https://github.com/owner/%70roject', 'https://github.com/owner/project//', 'https://github.com/owner/.git',
    'https://gitlab.com/group/project/-/tree/main', 'https://gitlab.com/group/project/-/issues',
    'https://codeberg.org/owner/project/src/branch/main', 'https://bitbucket.org/owner/project/src/main',
    'https://github.com\\owner\\project', 'https://github.com/owner/pro ject']) {
    assert.equal(canonicalRepository(value), null, String(value));
  }
});

test('groups only exact normalized title/kind with a common canonical repository', () => {
  const mr = hit('modrinth', 'mr', { title: '  ＳＯＤＩＵＭ\t', source_url: 'https://github.com/CaffeineMC/Sodium.git' });
  const cf = hit('curseforge', '1', { title: 'sodium', source_url: 'https://github.com/caffeinemc/sodium/' });
  const original = structuredClone([mr, cf]);
  assert.deepEqual(groupCatalogHits([mr, cf]), [{ key: 'modrinth:mr', hits: [mr, cf] }]);
  assert.deepEqual([mr, cf], original, 'input hits and their metadata stay unchanged');
  const spaced = hit('modrinth', 'spaces', { title: 'A\t  Mod', source_url: 'https://github.com/owner/repo' });
  const other = hit('curseforge', '2', { title: ' a mod ', source_url: 'https://github.com/owner/repo.git' });
  assert.equal(groupCatalogHits([spaced, other]).length, 1);
});

test('same names or numeric IDs alone never merge closed-source, unknown, or differently linked projects', () => {
  for (const [left, right] of [
    [hit('modrinth', '123'), hit('curseforge', '123')],
    [hit('modrinth', 'mr', { source_url: null }), hit('curseforge', '1', { source_url: null })],
    [hit('modrinth', 'mr', { source_url: 'https://github.com/a/one' }), hit('curseforge', '1', { source_url: 'https://github.com/a/two' })],
    [hit('modrinth', 'mr', { source_url: 'https://example.test/project' }), hit('curseforge', '1', { source_url: 'https://example.test/project' })],
    [hit('modrinth', 'mr', { source_url: 'https://github.com/owner/project/issues' }), hit('curseforge', '1', { source_url: 'https://github.com/owner/project/issues' })],
  ]) assert.equal(groupCatalogHits([left!, right!]).length, 2);
});

test('monorepo modules with different titles or different kinds remain separate', () => {
  const shared = { source_url: 'https://github.com/owner/monorepo' };
  const a = hit('modrinth', 'a', { ...shared, title: 'Module A' });
  assert.equal(groupCatalogHits([a, hit('curseforge', '1', { ...shared, title: 'Module B' })]).length, 2);
  assert.equal(groupCatalogHits([a, hit('curseforge', '1', { ...shared, title: 'Module A', project_type: 'resourcepack' })]).length, 2);
});

test('never collapses same-provider projects and retains interleaved first-hit group order', () => {
  const repo = 'https://github.com/owner/repo';
  const mr1 = hit('modrinth', 'first', { source_url: repo });
  const mr2 = hit('modrinth', 'second', { source_url: repo });
  assert.deepEqual(groupCatalogHits([mr1, mr2]).map(group => group.key), ['modrinth:first', 'modrinth:second']);
  const cfOther = hit('curseforge', '2', { title: 'Other', source_url: 'https://github.com/owner/other' });
  const mrUnlinked = hit('modrinth', 'unlinked', { title: 'Unlinked' });
  const cfFirst = hit('curseforge', '1', { source_url: repo });
  const mrOther = hit('modrinth', 'other', { title: 'Other', source_url: cfOther.source_url });
  const groups = groupCatalogHits([mr1, cfOther, mrUnlinked, cfFirst, mrOther]);
  assert.deepEqual(groups.map(group => group.key), ['modrinth:first', 'curseforge:2', 'modrinth:unlinked']);
  assert.deepEqual(groups.map(group => group.hits.map(item => item.project_id)), [['first', '1'], ['2', 'other'], ['unlinked']]);
});

test('requests only matching MR project references, deduplicates IDs, and caps enrichment at 24', () => {
  const cf = hit('curseforge', '1', { source_url: 'https://github.com/owner/project' });
  const candidates = Array.from({ length: 30 }, (_, index) => hit('modrinth', `mr-${index}`, { title: ' ＳＯＤＩＵＭ ' }));
  assert.deepEqual(candidateProjectIds([hit('modrinth', 'wrong-title', { title: 'Other' }),
    hit('modrinth', 'wrong-kind', { project_type: 'shader' }), cf, ...candidates, candidates[0]!]), candidates.slice(0, 24).map(item => item.project_id));
  assert.deepEqual(candidateProjectIds([hit('modrinth', 'mr'), hit('curseforge', '2', { source_url: null })]), []);
  assert.deepEqual(candidateProjectIds([hit('modrinth', 'mr'), hit('curseforge', '2', { source_url: 'https://github.com/owner/project/issues' })]), []);
  assert.deepEqual(candidateProjectIds([cf, candidates[0]!, candidates[0]!]), ['mr-0']);
});

test('ambiguous multiple projects on either provider never select an arbitrary cross-provider partner', () => {
  const shared = { source_url: 'https://github.com/owner/project' };
  const mr1 = hit('modrinth', 'mr-one', shared);
  const mr2 = hit('modrinth', 'mr-two', shared);
  const cf1 = hit('curseforge', '1', shared);
  const cf2 = hit('curseforge', '2', shared);
  for (const values of [[mr1, mr2, cf1], [mr1, cf1, mr2], [cf1, cf2, mr1], [cf1, mr1, cf2]]) {
    const groups = groupCatalogHits(values);
    assert.equal(groups.length, 3);
    assert.ok(groups.every(group => group.hits.length === 1));
    assert.deepEqual(groups.map(group => group.key), values.map(item => `${item.source}:${item.project_id}`));
  }
});
