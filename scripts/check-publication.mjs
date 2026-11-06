// Inspect the Git index locally. Never print matching values or send file contents away.
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';

function git(args) {
  const result = spawnSync('git', args, { encoding: 'buffer', maxBuffer: 32 * 1024 * 1024 });
  if (result.error || result.status !== 0) {
    console.error('Git-Prüfung fehlgeschlagen. Im Repository ausführen und Dateien zuerst stagen.');
    process.exit(2);
  }
  return result.stdout;
}

// By default check changes staged for the next commit. --all checks the full index.
const all = process.argv.includes('--all');
const paths = git(all
  ? ['ls-files', '--cached', '-z']
  : ['diff', '--cached', '--name-only', '--diff-filter=ACMR', '-z'])
  .toString('utf8').split('\0').filter(Boolean);
const findings = [];
const report = (path, line, type) => findings.push({ path, line, type });
const forbidden = /(?:^|\/)(?:\.env(?:\..+)?|\.npmrc|\.netrc|\.git-credentials|credentials\.json|secrets\.json|AGENTS\.md|design-qa\.md)$|(?:^|\/)(?:node_modules|dist|\.angular|target|gen|\.aws|\.azure|\.ssh|\.gnupg|\.agents|\.codex|private|\.private|backups|exports)(?:\/|$)|^docs\/(?:qa|design)\/|\.(?:pem|key|p12|pfx|jks|keystore|mobileprovision|log|secret|syncmc\.json)$|\.local(?:\.|$)/i;
const rules = [
  ['Privater Schlüssel', /-----BEGIN (?:[A-Z0-9]+ )?PRIVATE KEY-----/],
  ['GitHub-Token', /\b(?:gh[pousr]_[A-Za-z0-9]{20,}|github_pat_[A-Za-z0-9_]{30,})\b/],
  ['AWS-Zugriffsschlüssel', /\b(?:AKIA|ASIA)[A-Z0-9]{16}\b/],
  ['Slack-Token', /\bxox[baprs]-[A-Za-z0-9-]{20,}\b/],
  ['Google-API-Key', /\bAIza[A-Za-z0-9_-]{35}\b/],
  ['JWT', /\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\b/],
  ['CurseForge-Key oder Passwort-Hash', /\$2[aby]\$\d{2}\$[./A-Za-z0-9]{53}/],
  ['Persönlicher Home-Pfad', /(?:\/Users\/|\/home\/|[A-Za-z]:\\Users\\)[A-Za-z0-9._-]+[\/\\]/],
  ['Zugangsdaten in URL', /https?:\/\/[^\s/@:'"<>]+:[^\s/@'"<>]+@/i],
];
const allowedEmail = /^(?:[^@]+@users\.noreply\.github\.com|[^@]+@(?:example\.(?:com|org|net|test)|localhost))$/i;

// Reviewed synthetic fixtures only: exact file + full-line SHA-256 + finding type.
// Changed lines must be reviewed again; other credential rules remain active.
const reviewedFixtures = {
  // Negative device-code redirect validation.
  'src-tauri/src/auth.rs': {
    b93a290285ef5a7a6c693e1e0082fd1ac3204d7a6ffad75e1636060a60425f0e: ['E-Mail-Adresse manuell prüfen'],
  },
  // Negative content download-host validation.
  'src-tauri/src/content.rs': {
    '61833163e369b47357437abb80281a5b192cae14327ca33272b854ad64f1943b': ['Zugangsdaten in URL', 'E-Mail-Adresse manuell prüfen'],
  },
  // Negative repository URL and authenticated-download host validation.
  'src-tauri/src/curseforge.rs': {
    '0414f8f107aaf67484487e1777ef1c83ad05e411f95dc0d7cd0297df2efb31aa': ['Zugangsdaten in URL', 'E-Mail-Adresse manuell prüfen'],
    '43617e2d0c97adec1001fb34d40c326cf020cca35628b9957bef73a225eb105b': ['E-Mail-Adresse manuell prüfen'],
  },
  // Synthetic credentials in ignored launch-plan tests; no game is started.
  'src-tauri/src/launch.rs': {
    '2a1e5ac32f0508338f8f342a120c89448f7ff260ad88072d8ad04f58c42b8702': ['Mögliches Credential-Literal'],
    '1b8ec2aee5c8a31e414a156a9161201bec884bc78371ad8fffe45daff03fe223': ['Mögliches Credential-Literal'],
  },
  // Negative managed-runtime redirect validation.
  'src-tauri/src/managed_java.rs': {
    f79136a9d0cc65ff7dfd692af3b2f47fda06ce08a981392bf570712bccd2fdb8: ['Zugangsdaten in URL', 'E-Mail-Adresse manuell prüfen'],
  },
  // Negative Minecraft download-host validation.
  'src-tauri/src/minecraft.rs': {
    a0ed9c705fd84807fed75b0e4d41a64a7e7523e22033ac111b95a75649ad33bb: ['E-Mail-Adresse manuell prüfen'],
  },
  // Negative Modrinth repository URL validation.
  'src-tauri/src/modrinth.rs': {
    baf473182d103bcfe103cf511a33e8c7e57f943fbaa1f72feb79d6bba8ac7371: ['Zugangsdaten in URL', 'E-Mail-Adresse manuell prüfen'],
  },
  // Negative catalog repository URL normalization.
  'tests/catalog-groups.test.ts': {
    aa6311382c01f8ef0cc3a9fb0d2ed482853dc34a0ae125b3166020442217be17: ['Zugangsdaten in URL', 'E-Mail-Adresse manuell prüfen'],
  },
};

for (const path of paths) {
  if (forbidden.test(path) && !/(?:^|\/)\.env\.example$/.test(path)) {
    report(path, 0, 'Private oder generierte Datei');
  }
  const entry = git(['ls-files', '--stage', '--', path]).toString('utf8');
  if (/^(?:120000|160000) /.test(entry)) {
    report(path, 0, 'Symlink oder Submodul manuell prüfen');
    continue;
  }
  const bytes = git(['show', `:${path}`]);
  // Images are inspected visually; unknown binary files require an explicit review.
  if (bytes.includes(0)) {
    if (!/\.png$/i.test(path) || !bytes.subarray(0, 8).equals(Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))) {
      report(path, 0, 'Ungeprüfter Binärdateityp');
    }
    continue;
  }
  for (const [index, line] of bytes.toString('utf8').split(/\r?\n/).entries()) {
    const reviewedTypes = reviewedFixtures[path]?.[createHash('sha256').update(line).digest('hex')] ?? [];
    const reportLine = type => {
      if (!reviewedTypes.includes(type)) report(path, index + 1, type);
    };
    for (const [type, pattern] of rules) {
      if (pattern.test(line)) reportLine(type);
    }
    // Case-insensitive to catch mixed-case addresses too; print no address values.
    for (const match of line.matchAll(/\b[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}\b/gi)) {
      if (!allowedEmail.test(match[0])) reportLine('E-Mail-Adresse manuell prüfen');
    }
    if (/\b(?:api[_-]?key|client[_-]?secret|refresh[_-]?token|access[_-]?token|password)\b["']?\s*[:=]\s*["'][A-Za-z0-9_+/.=$-]{24,}["']/i.test(line)
      && !/\b(?:test|fake|example|mock|dummy)\b/i.test(line)) {
      reportLine('Mögliches Credential-Literal');
    }
  }
}

if (findings.length) {
  console.error(`Veröffentlichungsprüfung: ${findings.length} Fundstelle(n). Inhalte werden nicht ausgegeben.`);
  for (const { path, line, type } of findings) console.error(`${path}${line ? `:${line}` : ''} — ${type}`);
  process.exitCode = 1;
} else {
  console.log(`${paths.length} ${all ? 'versionierte' : 'gestagte'} Dateien geprüft: keine erkannten Veröffentlichungsrisiken.`);
  console.log('Bildinhalte, Metadaten und Git-Autor separat prüfen. Dies ist keine Garantie für Geheimnisfreiheit.');
}
