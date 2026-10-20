#!/usr/bin/env bash
# Print GitHub release notes for a tag on stdout.
set -euo pipefail
tag=${1:?tag, for example v1.1.0}
cat << EOF
Term Organize ${tag} is a draft build for testing. It is not a published release.

Unofficial. Not affiliated with, endorsed by, or sponsored by Northeastern University or Sandbox.

## Verify

The signing key is \`Term Organize Releases (unofficial)\`, fingerprint \`5276 3E58 9A37 573E 2C75 5AE2 A120 52B3 ED28 4C89\`.

\`\`\`bash
gpg --import term-organize-release-key.asc
gpg --fingerprint 52763E589A37573E2C755AE2A12052B3ED284C89
gpg --verify SHA256SUMS.asc SHA256SUMS
sha256sum -c SHA256SUMS
\`\`\`

## Install

Flatpak (org.gnome.Platform 47):

\`\`\`bash
flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user ./TermOrganize-${tag}-x86_64.flatpak
flatpak run org.termorganize.TermOrganize
\`\`\`

AppImage (GTK 4 and libadwaita are inside the file; \`pdftotext\` is still required for PDF import):

\`\`\`bash
chmod +x TermOrganize-${tag}-x86_64.AppImage
./TermOrganize-${tag}-x86_64.AppImage
\`\`\`
EOF
