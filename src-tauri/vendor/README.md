# GLib 0.18.5 security backport

Tauri's Linux GTK3 dependencies require the GLib 0.18 API. SyncMC uses a local
copy of GLib 0.18.5 with the upstream fix for
[RUSTSEC-2024-0429 / GHSA-wrw7-89jp-8q8g](https://rustsec.org/advisories/RUSTSEC-2024-0429.html).
The Cargo workspace applies this copy through `[patch.crates-io]`.

Source: [the original crates.io archive](https://static.crates.io/crates/glib/glib-0.18.5.crate).
Its SHA-256, verified against the original Cargo.lock entry, is
`233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`.
The upstream MIT license and copyright notice are included unchanged.

The sole source change is the two-line fix from
[gtk-rs-core commit b5a4071e439bef2b5eea76c3aa25e5ae84839e34](https://github.com/gtk-rs/gtk-rs-core/pull/1343/commits/b5a4071e439bef2b5eea76c3aa25e5ae84839e34):
`VariantStrIter::impl_get` declares its out-pointer mutable and passes `&mut p`
to `g_variant_get_child`. All 121 archive files are retained, and the package
keeps its actual version, 0.18.5.

On a system with GLib development libraries, run the optimized regression tests:

```sh
cargo test --locked --release --manifest-path src-tauri/Cargo.toml -p syncmc-glib-regression
```

Linux CI runs these tests in addition to the native application tests. They cover
forward and reverse string iteration, indexed iteration, exhaustion, and Unicode.
`npm run check:glib-backport` verifies the complete reviewed source tree and the
Cargo patch without network access. CI runs it on every change; the repository's
Git attributes preserve LF line endings on Windows as well.
When updating this copy, compare it against the checksum-verified archive and
review every difference. Remove the backport, workspace test package, and patch
entry and their integrity checks once Tauri's complete GTK dependency graph can
use GLib 0.20 or newer.
