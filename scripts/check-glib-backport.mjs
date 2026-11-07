// Keep the reviewed upstream backport intact, including its license and metadata.
import { createHash } from 'node:crypto';
import { lstatSync, readdirSync, readFileSync } from 'node:fs';

const source = new URL('../src-tauri/vendor/glib-0.18.5/', import.meta.url);
const expectedTreeHash = '2f7ef2019fd0cb8cb2b95f3a3a4d08b63c4ea546362eae332643a6a6de0e67bf';
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex');
const files = [];

function collect(relative = '') {
  const directory = new URL(relative, source);
  if (!lstatSync(directory).isDirectory()) throw new Error('Vendor-Verzeichnis ist ungültig.');
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const name = `${relative}${entry.name}`;
    if (entry.isDirectory()) collect(`${name}/`);
    else if (entry.isFile()) files.push(name);
    else throw new Error(`Unerwarteter Vendor-Dateityp: ${name}`);
  }
}

try {
  collect();
  const tree = createHash('sha256');
  for (const name of files.sort()) {
    tree.update(`${name}\0${sha256(readFileSync(new URL(name, source)))}\n`);
  }
  if (files.length !== 121 || tree.digest('hex') !== expectedTreeHash) {
    throw new Error('Die GLib-Quelle weicht vom geprüften Upstream-Backport ab.');
  }

  const lock = readFileSync(new URL('../src-tauri/Cargo.lock', import.meta.url), 'utf8');
  const packages = lock.split('[[package]]').filter(block => /^name = "glib"$/m.test(block));
  if (packages.length !== 1 || !/^version = "0\.18\.5"$/m.test(packages[0])
    || /^(?:source|checksum) =/m.test(packages[0])) {
    throw new Error('Cargo.lock muss genau eine lokale GLib-0.18.5-Quelle verwenden.');
  }
  const manifest = readFileSync(new URL('../src-tauri/Cargo.toml', import.meta.url), 'utf8');
  const patch = manifest.split('[patch.crates-io]')[1]?.split(/^\[/m)[0] ?? '';
  if (!/^glib = \{ path = "vendor\/glib-0\.18\.5" \}$/m.test(patch)) {
    throw new Error('Der Cargo-Patch für die geprüfte GLib-Quelle fehlt.');
  }
  console.log('GLib-Backport geprüft: 121 Archivdateien entsprechen dem geprüften Upstream-Backport; Cargo verwendet die lokale Quelle.');
} catch (error) {
  console.error(`GLib-Backport-Prüfung fehlgeschlagen: ${error.message}`);
  process.exitCode = 1;
}
