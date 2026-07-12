# Publishing an Alpine (apk) package

Unlike Ubuntu, Alpine has no Launchpad-style hosted PPA service that builds
and signs packages for you. The equivalent here is self-hosting a small apk
repository: `.github/workflows/publish-apk.yml` builds a signed `.apk` with
`abuild` inside an official `alpine` Docker image (one run per target arch),
then publishes the resulting `packages/<arch>/*.apk` + `APKINDEX.tar.gz`
files to this repo's `gh-pages` branch under `/alpine/`, which GitHub Pages
serves as plain static files. It runs as a job of `release.yml`
(`publish-alpine`), so every tag push or manual `release.yml` run publishes
alongside the GitHub Release, crates.io publish, and Ubuntu PPA upload —
there's nothing extra to trigger.

Because Rust crate downloads require network access and the build runs
offline inside the container, the workflow runs `cargo vendor` before
invoking `abuild`, same as `publish-ppa.yml` does for the Debian source
package.

## One-time setup

### 1. Generate a signing key

apk repositories are signed with a plain RSA keypair (not GPG), the same
kind `abuild-keygen` produces for local package builds:

```bash
openssl genrsa -out librtmp2-signing.rsa 4096
openssl rsa -in librtmp2-signing.rsa -pubout -out librtmp2-signing.rsa.pub
```

Pick a key file name following Alpine's own convention (`<packager
identity>-<8 hex chars>.rsa`), e.g. `openrtmp@openrtmp.org-668f1a2b.rsa` —
`abuild`/`apk` don't require this exact format, but community tooling and
`abuild-keygen` both use it, and the `.pub` file name is what end users will
copy into `/etc/apk/keys/`.

### 2. Enable GitHub Pages

**Repository Settings -> Pages -> Source**: set to "Deploy from a branch",
branch `gh-pages`, folder `/ (root)`. The workflow creates the `gh-pages`
branch itself (as an orphan branch) on its first successful run if it
doesn't already exist, so you don't need to create it manually — just make
sure Pages is pointed at it once it appears.

### 3. GitHub repository secrets

In the repo's **Settings -> Secrets and variables -> Actions**, add:

| Name | Type | Value |
|---|---|---|
| `ALPINE_PRIVATE_KEY` | secret | contents of `librtmp2-signing.rsa` |
| `ALPINE_KEY_NAME` | secret | the private key's file name, e.g. `openrtmp@openrtmp.org-668f1a2b.rsa` |

Treat `ALPINE_PRIVATE_KEY` as highly sensitive — anyone with it can sign
packages as this project. Keep `librtmp2-signing.rsa` (and the matching
`.pub`) somewhere safe outside the repo; the workflow re-derives the public
key from the private one at build time via `openssl rsa -pubout`, so you
never need to store the public half as a secret.

Optionally set the `ALPINE_ARCHES` repository **variable** (comma-separated,
default `x86_64,aarch64`) to change which arches get built.

## Releasing

Push a tag `vX.Y.Z` matching the version in `Cargo.toml` (or run
`release.yml` manually via `workflow_dispatch` with that tag) and
`release.yml`'s `publish-alpine` job calls `publish-apk.yml` automatically
once the GitHub Release build succeeds. To publish on its own — e.g.
retrying just the apk build, or targeting a different arch list without a
full release — run `publish-apk.yml` directly via its own
`workflow_dispatch`.

Each arch is built in its own matrix job (via `docker run --platform`,
using QEMU emulation on the `ubuntu-latest` runner for non-native arches),
producing `packages/<arch>/*.apk` and a freshly signed
`packages/<arch>/APKINDEX.tar.gz`. The `publish` job merges every arch's
output into `gh-pages`'s `/alpine/<arch>/` and commits it — a no-op commit
is skipped if nothing changed (e.g. a re-run that reuses the same version).

## Installing the published package

End users add the repository and its signing key, then install normally:

```bash
# fetch and install the public key
wget -O /etc/apk/keys/openrtmp@openrtmp.org-668f1a2b.rsa.pub \
  https://openrtmp.github.io/librtmp2/alpine/openrtmp@openrtmp.org-668f1a2b.rsa.pub

# add the repository
echo 'https://openrtmp.github.io/librtmp2/alpine/x86_64' \
  >> /etc/apk/repositories

apk update
apk add librtmp2 librtmp2-dev
```

Replace `x86_64` with the target arch and the key file name with whatever
`ALPINE_KEY_NAME` was set to.

## Local packaging files

- `alpine/APKBUILD` — builds `librtmp2` (shared library + pkg-config file)
  and the `librtmp2-dev` subpackage (static library, unversioned `.so`
  symlink, pkg-config file) via `abuild`'s `default_dev` split, mirroring
  `debian/control`'s package split
- `.github/workflows/publish-apk.yml` — builds, signs, and publishes the
  apk repository; see the header comment in that file for the full
  requirements list

You can build and inspect a package locally (signing skipped, native arch
only) with `abuild-keygen` on an Alpine machine or container:

```bash
cargo vendor vendor > /dev/null && mkdir -p .cargo && cargo vendor vendor > .cargo/config.toml
apk add alpine-sdk cargo openssl-dev pkgconf
abuild-keygen -a -i -n
cd alpine && abuild -r
```
