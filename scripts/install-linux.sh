#!/usr/bin/env bash
# Bootstrap a native repository once; future updates belong to the package manager.
set -euo pipefail
DEFAULT_REPOSITORY_URL=''
DEFAULT_SIGNING_FINGERPRINT=''
repository_url=${ATLAS_REPOSITORY_URL:-$DEFAULT_REPOSITORY_URL}
fingerprint=${ATLAS_SIGNING_FINGERPRINT:-$DEFAULT_SIGNING_FINGERPRINT}
dry_run=false
while (($#)); do
  case "$1" in
    --repo-url) repository_url=${2:?Missing repository URL}; shift 2 ;;
    --fingerprint) fingerprint=${2:?Missing signing fingerprint}; shift 2 ;;
    --dry-run) dry_run=true; shift ;;
    --help)
      echo 'Usage: install-linux.sh [--repo-url HTTPS_URL] [--fingerprint GPG_FINGERPRINT] [--dry-run]'
      exit 0 ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done

fail() { echo "$*" >&2; exit 1; }
[[ $(uname -m) == x86_64 ]] || fail 'These packages support x86_64. Other architectures need a source build.'
os_file=${ATLAS_OS_RELEASE:-/etc/os-release}
[[ -r "$os_file" ]] || fail 'Cannot identify this Linux distribution.'
# Parse data without executing os-release as shell code.
id=$(sed -n 's/^ID=//p' "$os_file" | tr -d '"\047')
like=$(sed -n 's/^ID_LIKE=//p' "$os_file" | tr -d '"\047')
case "$id" in
  ubuntu|debian|linuxmint|pop|elementary|zorin|kali|neon) manager=apt ;;
  fedora|rhel|centos|rocky|almalinux) manager=dnf ;;
  opensuse*|sles) manager=zypper ;;
  arch|endeavouros|manjaro|garuda) manager=pacman ;;
  *)
    case " $like " in
      *' debian '*|*' ubuntu '*) manager=apt ;;
      *' arch '*) manager=pacman ;;
      *' suse '*|*' opensuse '*) manager=zypper ;;
      *' fedora '*|*' rhel '*) manager=dnf ;;
      *) fail 'No supported native repository for this distro. Use the AppImage or build from source.' ;;
    esac ;;
esac
[[ $repository_url =~ ^https://[A-Za-z0-9.-]+(:[0-9]+)?(/[A-Za-z0-9._~/-]*)?$ ]] || {
  if ! $dry_run; then fail 'Supply the official HTTPS repository URL with --repo-url.'; fi
  repository_url='https://YOUR-OFFICIAL-REPOSITORY'
}
repository_url=${repository_url%/}
fingerprint=${fingerprint^^}
if $dry_run; then
  echo "Distribution: $id; manager: $manager; architecture: x86_64"
  echo "Repository: $repository_url"
  echo "Verify the pinned GPG key, add the signed $manager repository, then install atlas."
  echo 'Future updates use the same package manager. No system changes made.'
  exit 0
fi
[[ $fingerprint =~ ^([A-F0-9]{40}|[A-F0-9]{64})$ ]] || fail 'Supply the official GPG fingerprint with --fingerprint.'
for tool in curl gpg "$manager"; do command -v "$tool" >/dev/null || fail "Install $tool first."; done
privileged=()
if ((EUID != 0)); then
  command -v sudo >/dev/null || fail 'Run with administrator privileges or install sudo.'
  privileged=(sudo)
fi
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
mkdir -m700 "$scratch/gnupg"
curl --fail --location --retry 3 --proto '=https' --proto-redir '=https' \
  "$repository_url/atlas-signing-key.asc" -o "$scratch/key.asc"
actual=$(gpg --batch --homedir "$scratch/gnupg" --show-keys --with-colons "$scratch/key.asc" 2>/dev/null |
  awk -F: '$1 == "fpr" {print $10; exit}')
[[ $actual == "$fingerprint" ]] || fail 'Repository signing key does not match the official fingerprint.'
gpg --batch --homedir "$scratch/gnupg" --import "$scratch/key.asc" >/dev/null 2>&1
# Import only the pinned identity, even if the downloaded file contains extra keys.
gpg --batch --homedir "$scratch/gnupg" --armor --export "$fingerprint" > "$scratch/trusted.asc"
[[ -s $scratch/trusted.asc ]] || fail 'Could not export the trusted repository key.'

case "$manager" in
  apt)
    gpg --batch --homedir "$scratch/gnupg" --dearmor --output "$scratch/atlas.gpg" "$scratch/trusted.asc"
    "${privileged[@]}" install -Dm644 "$scratch/atlas.gpg" /etc/apt/keyrings/atlas.gpg
    cat > "$scratch/atlas.sources" <<EOF
Types: deb
URIs: $repository_url/apt
Suites: stable
Components: main
Architectures: amd64
Signed-By: /etc/apt/keyrings/atlas.gpg
EOF
    "${privileged[@]}" install -Dm644 "$scratch/atlas.sources" /etc/apt/sources.list.d/atlas.sources
    "${privileged[@]}" apt-get update
    "${privileged[@]}" apt-get install -y atlas ;;
  dnf|zypper)
    "${privileged[@]}" install -Dm644 "$scratch/trusted.asc" /etc/atlas/atlas-signing-key.asc
    cat > "$scratch/atlas.repo" <<EOF
[atlas]
name=Atlas stable
baseurl=$repository_url/rpm/x86_64
enabled=1
gpgcheck=1
repo_gpgcheck=1
gpgkey=file:///etc/atlas/atlas-signing-key.asc
EOF
    if [[ $manager == dnf ]]; then
      "${privileged[@]}" install -Dm644 "$scratch/atlas.repo" /etc/yum.repos.d/atlas.repo
      "${privileged[@]}" dnf install -y atlas
    else
      "${privileged[@]}" install -Dm644 "$scratch/atlas.repo" /etc/zypp/repos.d/atlas.repo
      "${privileged[@]}" zypper --gpg-auto-import-keys refresh atlas
      "${privileged[@]}" zypper --non-interactive install --from atlas atlas
    fi ;;
  pacman)
    command -v pacman-key >/dev/null || fail 'pacman-key is required.'
    "${privileged[@]}" pacman-key --add "$scratch/trusted.asc"
    "${privileged[@]}" pacman-key --lsign-key "$fingerprint"
    cat > "$scratch/atlas.conf" <<EOF
[atlas]
SigLevel = Required
Server = $repository_url/pacman/x86_64
EOF
    "${privileged[@]}" install -Dm644 "$scratch/atlas.conf" /etc/pacman.d/atlas.conf
    if ! grep -Eq '^Include[[:space:]]*=[[:space:]]*/etc/pacman.d/atlas.conf[[:space:]]*$' /etc/pacman.conf; then
      printf '\nInclude = /etc/pacman.d/atlas.conf\n' | "${privileged[@]}" tee -a /etc/pacman.conf >/dev/null
    fi
    # Arch does not support partial upgrades.
    "${privileged[@]}" pacman -Syu --needed atlas ;;
esac
echo 'Atlas installed. Future updates come through your native package manager.'
