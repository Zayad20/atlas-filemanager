# Private folders

Atlas's local private folders are opened from **Protect with password…** on a
single local directory. On Windows, authorized profiles see the original folder
name even while locked; other profiles see no row, including when hidden files
are enabled. The physical container keeps its opaque name in Explorer. Locked
names are encrypted for the current Windows account using DPAPI, bound to the
container UUID and its profile grants. No names are stored in profile preferences.
Older containers learn their display name after one successful password unlock.
On another Windows account/computer, or if the optional label cannot be read,
Atlas falls back to **Protected folder** until a successful unlock refreshes it.
Other platforms retain the generic locked label.

## User flow

- Set and confirm a passphrase (12–1024 characters); select allowed profiles.
  The current profile retains access. Existing profiles receive stable UUIDs;
  renaming preserves identity and duplicating creates a new identity.
- Atlas writes and verifies ciphertext before retiring matching originals.
  Explorer sees an opaque `.atlas-vault-…` directory. Do not rename this reserved
  prefix or modify its internal files. It is a portable encrypted container,
  independent of Explorer attributes or Windows EFS.
- Select the row and choose **Unlock** in its inspector, double-click it, or use
  **Unlock…** in the context menu. Enter the password and press the orange arrow
  (or Enter). On Windows this mounts the vault and enters it in the same normal
  pane, with regular listing, subfolders, selection, copy/paste, rename and opening
  files in their associated applications. No plaintext directory is extracted.
- WinFsp 2.1 is optional and required for filesystem mounting on Windows. If absent,
  the encrypted browser offers **Download and install WinFsp**, explaining the
  download and linking its license. An explicit click downloads the official pinned
  MSI, verifies SHA-256, and starts Windows Installer with a progress interface.
  Windows may request elevation. Atlas retries mounting after success; cancellation
  preserves encrypted files, and restart-required results are explained. DLL loading
  is delayed, so Atlas starts without the driver. This external-driver acquisition
  must be specifically assessed by Store certification; it is not assumed permitted.
- A quiet bottom session bar shows the private folder name, access settings and
  **Lock**. The mounted directory lives at a random path in the reserved
  `atlas-private-mounts` namespace. Tabs and breadcrumbs show the real folder name
  from process memory. Up from its root returns to the original folder's parent.
- The normal inspector uses built-in memory previews for private files, bypassing
  extension previewers and Windows Shell thumbnails. Automatic PDF/video previews
  that require plaintext temporary images are disabled. Files can still be opened
  in external applications. Built-in conversion and persisted tags are unavailable
  inside private folders. Atlas's private reads never create a plaintext staging
  directory or persistent thumbnail cache.
- Copying/pasting out of the mount explicitly creates ordinary unencrypted files.
  Copying in retains the originals; cutting/moving has normal file-manager
  semantics. Deletion inside the mount is permanent, avoiding plaintext copies in
  the Windows recycle bin. Shell undo records are disabled for private operations.
- The access-settings button requires the current password to save changes. It can update authorized
  profiles and optionally replace the password, without re-encrypting file data.
  Cancelling settings remounts the session into the same pane.
- **Lock**, closing the window, switching profiles, or five minutes of inactivity
  revoke the mount and clear private preview/selection/history/undo state. User
  lock, profile switching and normal window closing wait for open files and active
  operations to finish; close documents in external applications first. No new
  opens can race a successful lock. Each vault holds an exclusive OS lock,
  preventing concurrent writers in other Atlas instances.
- Session/profile saves scrub mounted locations, histories, bookmarks and folder
  preferences before serialization, even when saved while unlocked. Private mounts
  are excluded from Atlas's global scans and search results. In-memory file
  clipboard entries are discarded on lock; the current OS file clipboard is cleared
  only if it still contains those private paths. Windows clipboard history and
  third-party application histories/caches remain outside Atlas's control.
- **Remove protection…** explicitly restores the original directory without
  encryption. Ciphertext remains intact on export failure. An inability to remove
  the redundant encrypted copy is reported separately after successful restore.

## Format v2 and v1 compatibility

`vault.json` contains a bounded, versioned public envelope: vault UUID, random
16-byte salt, profile UUID grants, wrapped key, encrypted manifest and an optional
DPAPI-encrypted display name. Profile names, plaintext directory names and all
inner filenames/metadata are excluded from the public envelope. The display name
does not contain a password or file key; its disclosure policy allows the current
Windows account to show it before password entry, after the profile visibility
check. Content and child names still require the vault password. Grants are
authenticated associated data: editing a grant
does not grant decryption and will invalidate password unwrapping.

New vaults use envelope version 2. Unlocking a v1 vault upgrades its authenticated
envelope using the entered password; existing ciphertext objects remain readable
and are converted lazily on first edit. Older Atlas versions reject the v2 header.

Argon2id v19 (64 MiB, three passes, one lane) derives a 32-byte password key.
XChaCha20-Poly1305 wraps a random 32-byte master key with a fresh 24-byte nonce.
The master key encrypts the manifest separately. Manifest replacement uses a
synced temporary file and atomic replacement. Keys, decoded manifest entries,
password copies and source/text preview buffers use `zeroize`/`Zeroizing`.
Rendered text and image frames are UI memory owned by the private session;
secrets never enter the serializable app model or profile preferences.

Imported legacy file objects have random UUID names. Each object has a 32-byte header with a
format marker, random 16-byte nonce prefix and original byte length. Objects use
independently authenticated 1 MiB chunks with a monotonic 64-bit nonce suffix.
Vault UUID, object UUID, byte length and chunk number are associated data. Empty
files still have an authentication tag. SHA-256 digests in the encrypted manifest
add whole-file verification. Truncation, extra bytes and swapped chunks fail.

Edited/new files use independent copy-on-write blocks of at most 1 MiB, with random
UUID object names and fresh XChaCha20-Poly1305 nonces on every write. Associated
data binds the vault, logical file identity and block index. The encrypted manifest
authenticates file size, block references and sparse zero ranges. Writes sync new
ciphertext blocks before atomically replacing the encrypted manifest, then retire
superseded blocks. No plaintext journal is used. Interrupted pre-commit writes
leave only encrypted orphan objects, collected at the next mount. Random reads
decrypt only the necessary authenticated blocks. Existing open handles retain
deleted/replaced inodes until their last handle closes, preserving normal Windows
save/replace semantics without publishing them in directory listings.

The implementation uses the existing RustCrypto
[Argon2](https://docs.rs/argon2/0.5.3/argon2/) and
[XChaCha20-Poly1305](https://docs.rs/chacha20poly1305/0.10.1/chacha20poly1305/)
libraries. This new application format has no independent security audit.

## Migration and boundaries

Migration rejects symlinks, junctions, hardlinks and special files rather than
following them or silently leaving plaintext links. Windows named streams are
also rejected, so alternate-stream data is never silently discarded. A committed
copy is decrypted and verified before any original is retired. Every original
is hashed again before removal; on Windows removal targets the verified handle
while denying concurrent writers/deleters. Unexpected/new files prevent directory
removal. Cleanup errors preserve the verified encrypted copy and report that
plaintext originals may still remain. Cancellation is cooperative before commit;
an already committed cleanup can finish safely.

The local directory browser refuses direct payload navigation. Listing previews,
tree/Miller deliveries and watcher deltas apply the same profile visibility rule.
Automatic previews and recursive scans skip the reserved storage namespace;
search results exclude container paths. A successful migration clears live
previews/thumbnails/search snapshots, removes affected saved tabs/history,
bookmarks, recent directories and view preferences, and retires tag associations
on the SQLite worker with secure deletion and a WAL checkpoint.

Profiles are local app identities, not authenticated Windows users. Password
encryption is the cryptographic boundary. A person who can switch to an allowed
profile still needs its folder password. Disk observers can see container
existence, opaque object counts and approximate sizes. Ordinary deletion of
pre-existing plaintext is not secure erasure of SSD blocks, backups, Windows
thumbnail caches, external search indexes, pagefiles or snapshots. Explicit
exports and import source copies are outside the encrypted container.

## Verification

Core tests cover multi-chunk and empty files, plaintext/name absence on storage,
round-trip restoration, wrong passwords, profile grants/AAD tampering, password
replacement, duplicate names, copy-in imports, cancellation, corruption,
truncation, trailing bytes, invalid manifest paths/cycles, listing masking and
direct navigation refusal. Platform tests reject Windows hardlinks/alternate
streams and Unix links. Model tests cover legacy identity migration and rename
stability.

`cargo run -p atlas-ui --example vault_diagnostic --features ui-lab -- <output>`
uses isolated configuration and fixtures in a hidden native window. It checks
light/dark/zoom layouts, password mismatch, actual encryption, same-pane navigation,
normal inspector image previews, sanitized session/profile saves, open-file lock
protection, settings/remount, incorrect passwords, profile unmount and restoration.

`cargo run -p atlas-core --example vault_mount_diagnostic -- <output>` exercises
real Win32 listing, reading, editing, flushing, truncation, creation, renaming,
moving, deletion, replacement with open handles, large files, lock/unlock and
durable edits through WinFsp. `--install-driver` explicitly installs the verified
driver first. The diagnostic operates only on generated fixture files.
