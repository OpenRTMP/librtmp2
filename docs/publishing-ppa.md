# Publishing to an Ubuntu PPA

`.github/workflows/publish-ppa.yml` builds a signed Debian **source**
package and uploads it to a Launchpad PPA with `dput`. It runs as a job of
`release.yml` (`publish-ppa`), so every tag push or manual `release.yml`
run publishes to the PPA automatically alongside the GitHub Release and
crates.io publish — there's nothing extra to trigger. Launchpad's own
build farm then compiles the binary `.deb` for each targeted Ubuntu series
(default `resolute` only, configurable via the workflow's `series` input
when running it standalone). The workflow never builds or uploads a binary
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

### 4. Rust toolchain availability (Rust 1.95 required)

`Cargo.toml` declares `rust-version = "1.95"` because
`parking_lot 0.13` and its `lock_api` / `parking_lot_core` dependencies
require at least Rust 1.95. `debian/control` therefore requires
`rustc (>= 1.95)` as well, matching the actual crate MSRV.

**Ubuntu 26.04 (resolute) PPA limitation:** the official Ubuntu 26.04
archive currently provides `rustc` 1.93, not 1.95. Launchpad builds the
uploaded Debian source package using that series' package repositories,
not the Rust toolchain installed by GitHub Actions. Without an available
`rustc >= 1.95` package, the PPA build will remain in **Dependency wait**.

Before publishing a release targeting `resolute`, either wait until
Ubuntu provides a compatible Rust toolchain or configure a trusted
additional PPA dependency that publishes a compatible `rustc` (and
matching `cargo`) package for that Ubuntu series. You can configure
this from the Launchpad PPA page under **Admin -> Change details ->
PPA dependencies**. Alternatively, maintain your own source-package
build of the official Rust toolchain for that series. Do not lower
`debian/control`'s minimum solely to make dependency resolution pass:
the vendored crate dependencies would still fail with Rust 1.93.

Re-check the archive and PPA dependency availability on future releases.
The local GitHub Actions source-package verification uses `rustup` and
does not establish toolchain availability inside Launchpad's builders.

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

Each targeted series gets its own upload, versioned
`X.Y.Z~<series><build-id>` (e.g. `0.3.0~resolute15r1` — no Debian
revision, since `debian/source/format` is `3.0 (native)`), since binaries
built for one Ubuntu series are generally not installable on another. The
`<build-id>` suffix (`<run_number>r<run_attempt>`) makes every upload's
filename unique: Launchpad permanently rejects re-uploading a
filename+version whose contents differ from what it already has on
record, and `cargo vendor`'s output isn't byte-reproducible across CI
runs, so a fixed suffix would make any retry after a rejected/failed
upload fail again with "already exists ... but uploaded version has
different contents."

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
- `debian/source/format` — `3.0 (native)`, so the whole working tree
  (including the CI-generated `vendor/` directory) becomes the source
  package with no separate upstream tarball to track

You can build and inspect a source package locally (signing skipped)
with:

```bash
cargo vendor vendor > .cargo/config.toml
dpkg-buildpackage -S -sa -us -uc -d
```
