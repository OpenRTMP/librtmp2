# Publishing to an Ubuntu PPA

`.github/workflows/publish-ppa.yml` builds a signed Debian **source**
package and uploads it to a Launchpad PPA with `dput`. It runs as a job of
`release.yml` (`publish-ppa`), so every tag push or manual `release.yml`
run publishes to the PPA automatically alongside the GitHub Release and
crates.io publish — there's nothing extra to trigger. Launchpad's own
build farm then compiles the binary `.deb` for each targeted Ubuntu series
(default `resolute` for Ubuntu 26.04 LTS and `stonking` for Ubuntu
26.10, configurable via the workflow's `series` input when running it
standalone). The workflow never builds or uploads a binary
package itself.

Because Rust crate downloads require network access, and Launchpad's
builders run offline, the workflow runs `cargo vendor` before building the
source package and ships the vendored dependencies inside it.

## One-time setup

### 1. Launchpad account

1. Create an account at <https://launchpad.net/+login> (or sign in with
   Ubuntu One).
2. Complete the account: set a display name and, ideally, add and verify at
   least one email address under **Personal details -> Change details ->
   Add an email address**. Launchpad requires this before you can activate
   a PPA.

### 2. GPG signing key

Launchpad identifies uploaders by their OpenPGP key, not by password —
`dput` never authenticates with Launchpad credentials.

```bash
gpg --full-generate-key        # RSA, 4096 bit, no expiry or a long one
gpg --list-secret-keys --keyid-format=long   # note the key ID / fingerprint
gpg --armor --export <KEY_ID> > pubkey.asc
```

Upload `pubkey.asc` on Launchpad under **Personal details -> OpenPGP keys
-> Import an OpenPGP key**, then confirm it: Launchpad emails an
encrypted challenge to your registered address; decrypt it with
`gpg --decrypt` and follow the link it contains. The key is unusable for
uploads until this confirmation step is done.

### 3. Create the PPA

**Personal details -> Create a new PPA** (or on a team page, if the PPA
should be owned by a team rather than your personal account). Note the
resulting `<owner>/<ppa-name>` — e.g. `openrtmp/librtmp2` — this is the
value the workflow uploads to via `dput ppa:<owner>/<ppa-name>`.

If the PPA is team-owned, make sure your user is a member of that team
with upload rights.

### 4. Rust toolchain availability (no extra setup currently needed)

`debian/control` declares `Build-Depends: rustc (>= 1.93)`, matching this
crate's actual MSRV (`rust-version` in `Cargo.toml`) rather than whatever
toolchain happened to be installed in CI. `resolute` (26.04, the current
Ubuntu LTS) ships `rustc` up to `1.93` in its own archive as of this
writing, so Launchpad's build chroot can satisfy this Build-Depends
natively — no PPA dependency configuration is required right now.

This is worth re-checking whenever this crate's MSRV moves again (edition
2024 itself already requires `rustc >= 1.85`, so anything below that is
never an option without dropping the edition): if `rust-version` in
`Cargo.toml` is bumped past what `resolute`'s archive offers, uploads will
sit as "Dependency wait" on Launchpad until you either wait for Ubuntu to
backport a newer `rustc` into the series (Ubuntu has been doing this
periodically for `resolute`, per its multiple parallel `rustc` versions),
add a trusted third-party Rust-toolchain PPA as a **PPA dependency** (PPA's
Launchpad page -> **Admin -> Change details** -> **"PPA dependencies"**),
or self-host one by repackaging the official prebuilt `rustc`/`cargo`
tarballs from `https://static.rust-lang.org/dist/` as a small Debian source
package with a version/epoch high enough that apt prefers it.

### 5. GitHub repository secrets/variables

In the repo's **Settings -> Secrets and variables -> Actions**, add:

| Name | Type | Value |
|---|---|---|
| `PPA_GPG_PRIVATE_KEY` | secret | `gpg --armor --export-secret-keys <KEY_ID>` |
| `PPA_GPG_KEY_ID` | secret | the key ID/fingerprint used above |
| `PPA_GPG_PASSPHRASE` | secret | the key's passphrase |
| `PPA_TARGET` | variable | `<owner>/<ppa-name>`, e.g. `openrtmp/librtmp2` |

Export the private key with `gpg --armor --export-secret-keys`, not
`--export` (which only exports the public key). Treat this secret as
highly sensitive — anyone with it can sign uploads as you on Launchpad.
Consider generating a dedicated subkey for CI use instead of exporting
your primary key, so it can be revoked independently.

## Releasing

Push a tag `vX.Y.Z` matching the version in `Cargo.toml` (or run
`release.yml` manually via `workflow_dispatch` with that tag) and
`release.yml`'s `publish-ppa` job calls this workflow automatically once
the GitHub Release build succeeds. To publish to the PPA on its own — e.g.
retrying just the PPA upload, or targeting a different `series` list
without a full release — run `publish-ppa.yml` directly via its own
`workflow_dispatch`.

Each targeted Ubuntu series gets its own source upload, versioned
`X.Y.Z-1~<series><build-id>` (for example, `0.17.1-1~resolute15r1` or
`0.17.1-1~stonking15r1`). `X.Y.Z` is the upstream librtmp2 release;
`-1` is the Debian packaging revision, which starts at 1 for each new
upstream release. The suffix includes the Ubuntu series and the GitHub
run number plus attempt number, making retries distinct. This newer
non-native versioning format also sorts above the old PPA's native
`0.17.1~resolute...` versions.

The Debian source format is `3.0 (quilt)`, so the workflow generates a
reproducible `X.Y.Z.orig.tar.xz` containing the source and generated
header. Packaging files and the vendored offline crate archive live
under `debian/` in a separate `.debian.tar.xz`, avoiding changes to
the shared upstream tarball when cargo vendor differs between runs.
Launchpad builds the binary packages from this signed source upload.

After Launchpad accepts the upload (check **your PPA's page -> View
package details** or the confirmation email), it queues the binary
build for each configured architecture; that step is entirely on
Launchpad's infrastructure and outside this repo's CI.

## Local packaging files

- `debian/control` — binary packages `librtmp2` (shared library) and
  `librtmp2-dev` (static library + pkg-config file)
- `debian/rules` — offline `cargo build --release --all-features`,
  installs the multiarch-versioned library paths
- `debian/changelog` — rewritten per-series/version by the workflow via
  `dch`; the committed entry is only a placeholder
- `debian/source/format` — `3.0 (quilt)`; the workflow creates an upstream
  `.orig.tar.xz` and stores the offline vendor archive in `debian/`

You can build and inspect a source package locally (signing skipped)
with:

```bash
cargo vendor vendor > .cargo/config.toml
# For 3.0 (quilt), create the upstream .orig.tar.xz and move the
# vendored crate archive to debian/vendor.tar.xz before building,
# using the same archive steps as the CI workflow.
dpkg-buildpackage -S -sa -us -uc -d
```
