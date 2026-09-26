# APT repository layout for privacytk

This directory holds the templates the CI release pipeline uses to
produce an APT-friendly artifact set. Drop these into your internal
APT mirror alongside `Packages.gz`, `Release`, `Release.gpg`.

The CI does **not** run an APT mirror itself — your operator does. The
release workflow uploads the artefacts to GitHub Releases; the
operator (or a separate mirror job) downloads them, generates
`Packages.gz` with `dpkg-scanpackages`, signs `Release` with the
operator's key, and serves the tree from a static file server.

## Files

- `control.template` — fields for `Packages`. Substituted per-version
  by the CI.
- `conffiles` — empty list. privacytk has no conffiles; running
  `--remove` deletes everything in `/etc/skel/.config/privacytk/` if the
  operator chose to drop defaults there (opt-in).
- `postinst` / `prerm` / `postrm` — run after install / before remove
  / after remove. privacytk does not install hooks itself; that is a
  per-user step (`privacytk init -g --agent <name>`) after first login.

## What `control.template` needs from the CI

The CI does not have the operator's `Maintainer` field, so the template
ships with `<MAINTAINER>` as a placeholder. The mirror job substitutes
it from a secret before publishing.

```text
Template variables (substituted by the mirror job):
  ${VERSION}   semver, e.g. 0.1.0
  ${ARCH}      deb arch: amd64, arm64
  ${SHA256}    sha256 of the .deb binary package
  ${SIZE}      bytes of the .deb
  ${DATE}      RFC 3339 timestamp
```

## Building a `.deb` from a release tarball

The CI does not produce `.deb` directly — it produces a static binary.
The mirror job wraps it:

```bash
# 1. Download the release tarball.
wget https://github.com/<your-org>/privacytk/releases/download/v${VERSION}/privacytk-v${VERSION}-${ARCH}.tar.gz

# 2. Stage the .deb layout.
mkdir -p staging/DEBIAN staging/usr/bin
tar -xzf privacytk-v${VERSION}-${ARCH}.tar.gz -C staging/usr/bin
cp control.template staging/DEBIAN/control
# Substitute ${VERSION}, ${ARCH}, ${SHA256}, ${SIZE}.

# 3. Build.
dpkg-deb --build --unsigned staging privacytk_${VERSION}_${ARCH}.deb

# 4. Sign and add to your APT mirror.
dpkg-sig --sign builder privacytk_${VERSION}_${ARCH}.deb
```

## How the operator verifies before promoting

```bash
# 1. SHA-256.
sha256sum -c privacytk_${VERSION}_${ARCH}.deb.sha256

# 2. cosign.
cosign verify-blob \
  --signature privacytk_${VERSION}_${ARCH}.deb.sig \
  --bundle privacytk_${VERSION}_${ARCH}.deb.bundle \
  --certificate-identity-regexp 'https://github.com/<your-org>/privacytk' \
  --certificate-github-workflow-repository <your-org>/privacytk \
  --certificate-github-workflow-ref refs/tags/v${VERSION} \
  privacytk_${VERSION}_${ARCH}.deb

# 3. Smoke battery post-install.
privacytk telemetry status | grep 'this build has no telemetry endpoint'
privacytk verify-network api.example.com | grep 'deny'
```
