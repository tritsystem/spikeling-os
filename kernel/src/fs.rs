//! MILESTONE 18: a minimal filesystem on top of Milestone 11's raw ATA
//! disk I/O -- until now only one fixed sector (LBA 0) could be
//! persisted (the learned weights); this adds real named-file storage,
//! a genuine "you can save your work" capability.
//!
//! MILESTONE 22: multi-sector files and real delete/reclamation. Each
//! of the 8 directory entries (still at LBA 1) now also stores a
//! start LBA and a sector count, allocated from a shared 64-sector
//! data pool at LBA 2..66 (first-fit over contiguous free runs
//! computed from the currently-used entries, not a persisted
//! bitmap) -- so files genuinely span multiple sectors up to a real
//! cap of MAX_FILE_SECTORS (8) * 512 = 4096 bytes, and `rm` actually
//! frees an entry's sectors back into that pool for a later `write`
//! to reuse, rather than just marking a slot deleted forever.
//!
//! MILESTONE 28: real one-level subdirectories. A directory entry can
//! now be either a file or a subdirectory (a new `is_dir` flag on each
//! entry); a subdirectory's "data" is exactly ONE sector, allocated
//! from the SAME shared pool files already use, holding its own
//! 8-entry table in the identical on-disk format as the root table at
//! LBA 1. `mkdir NAME` only creates directories directly under the
//! root (no `mkdir a/b`), and `write`/`read`/`rm` accept one optional
//! `DIR/` prefix (no `a/b/c`) -- honest, disclosed depth cap of 1,
//! matching the same "deliberately minimal" spirit as the 4096-byte
//! file cap above.
//!
//! MILESTONE 32: real ARBITRARY-DEPTH subdirectory nesting, plus
//! `rmdir`. **No on-disk format change was needed for the nesting
//! part** -- a subdirectory's table was already, since Milestone 28,
//! stored in the IDENTICAL on-disk format as the root table (same
//! `DirEntry` layout, same one-sector allocation out of the same
//! shared pool), so a subdirectory entry's `start_lba` could already
//! point at a table that itself contained further subdirectory
//! entries; Milestone 28 simply capped the *logic* that walks that
//! format at one level (`resolve_table` rejected anything past a
//! single `DIR/NAME`, and `mkdir` rejected any `/` in its argument at
//! all). This milestone replaces that one-level walk with
//! `resolve_dir_lba`, a generic loop over an arbitrary-depth
//! `/`-separated chain of components starting at the root, so
//! `write`/`read`/`rm`/`mkdir`/`rmdir`/`ls` all transparently support
//! any depth now -- callers (the shell) additionally layer a real
//! `cd`/current-working-directory concept on top so a user doesn't
//! have to type a full path every time (see shell.rs). Because a
//! corrupted disk could in principle make a subdirectory's start_lba
//! chain cycle back on itself, `resolve_dir_lba` and the now-recursive
//! `collect_occupied` both enforce a defensive `MAX_DEPTH` (32) so
//! that case fails with an error instead of recursing forever and
//! blowing the kernel's stack -- not a deliberate feature cap; real
//! usable depth is bounded far below that anyway by the shared
//! 64-sector pool (every directory level, like every file, consumes at
//! least one pool sector for its own table).
//!
//! `rmdir NAME` mirrors Milestone 22's file-delete reclamation exactly:
//! it refuses (real check, not a stub) unless the target directory's
//! own table is completely empty, then clears the parent's entry for
//! it -- freeing both the directory's slot in its parent AND its
//! one-sector table back into the shared pool for a later
//! `write`/`mkdir` to reuse, via the exact same "recompute occupancy
//! from currently-used entries" mechanism `collect_occupied` already
//! used for files. `rmdir` never touches a directory's contents itself
//! (no recursive delete) -- that's a deliberately different, more
//! dangerous operation this milestone does not implement.
//!
//! Still deliberately minimal, disclosed rather than hidden: no
//! fragmentation-avoiding allocator and no directory/pool compaction
//! -- a disk that has accumulated enough delete/write churn can fail
//! an allocation with "not enough free disk space" even when the
//! total free sectors would suffice, because free space isn't
//! necessarily contiguous and nothing moves existing files to make
//! room. Subdirectories eat into that same shared pool too (one
//! sector per `mkdir`, at ANY depth now), so heavy subdirectory use
//! leaves less pool space for file data. Each directory table (root or
//! any subdirectory, at any depth) is still capped at the original 8
//! entries -- nesting deeper doesn't change that per-level cap.

//! MILESTONE 46: a real trait/dispatch layer generalizing everything
//! above -- until now `write_file`/`read_file`/`delete_file`/`make_dir`/
//! `remove_dir`/`list` were free functions hardcoded to the one ATA
//! disk-backed store this file has ever had. They're now a thin
//! dispatcher (`resolve_backend`) in front of a real `FileSystem` trait,
//! with the EXACT pre-M46 disk logic moved, byte-for-byte unchanged,
//! into a `DiskFs` trait impl (renamed with a `_disk` suffix internally,
//! e.g. `write_file` -> `write_file_disk`), plus a genuinely SECOND,
//! concrete backing store, `RamFs` -- a flat, in-memory,
//! `BTreeMap<String, Vec<u8>>`-backed filesystem with no ATA I/O at all.
//! Routing is by a reserved top-level path component: any path that IS
//! `"ram"` or starts with `"ram/"` addresses `RamFs` (the remainder
//! after `"ram/"` as its own path); every other path is completely
//! unaffected and still resolves through `DiskFs` exactly as it always
//! has -- `resolve_backend` only ever inspects the FIRST path component,
//! so a disk-side subdirectory named `ram` nested below root (e.g.
//! `sub/ram/file`) is untouched, only a true top-level `ram` is
//! reserved. No shell.rs/process.rs/loader.rs changes were needed for
//! this: every caller already goes through these same module-level
//! functions with a plain path string, so `write ram/foo hello` in the
//! shell, `cd ram` (CWD-relative resolution keeps working unchanged, see
//! shell.rs's `resolve_arg`), and even the fd-backed `open()`/`read()`/
//! `fdwrite()`/`close()` syscalls in process.rs (which call
//! `fs::read_file`/`fs::write_file` directly) all transparently reach
//! `RamFs` for a `ram/...` path with zero code changes outside this
//! file.
//!
//! Honest, disclosed scope-cuts for `RamFs`, matching this file's own
//! established style of a real-but-deliberately-minimal subset:
//! - **Flat namespace only** -- no subdirectories under `ram/`;
//!   `make_dir`/`remove_dir` against it, and any path containing a
//!   second `/` after the mount, are refused with a clear error rather
//!   than silently doing something partial. The disk side's
//!   arbitrary-depth nesting (Milestone 32) is not replicated here.
//! - **No fixed entry-count cap** -- unlike the disk directory table's
//!   fixed 8 slots, a `BTreeMap` has no such limit; it's bounded only by
//!   the kernel's 100 KiB heap (`allocator.rs`), same as any other
//!   dynamically-growing kernel allocation.
//! - **Reuses `MAX_FILE_BYTES` (4096 bytes) as ramfs's own per-file
//!   cap** -- not because ramfs shares the disk's 512-byte-sector
//!   physical constraint (it has none), but so behavior stays
//!   predictable across both backends and one runaway write can't eat
//!   a large fraction of the 100 KiB heap.
//! - **Genuinely volatile** -- `RamFs` never touches `ata::*`; its
//!   contents do not survive a reboot. This is the actual, intended
//!   difference from `DiskFs`, not an oversight.
//! - `list()`'s existing `u16` length field (inherited unchanged from
//!   the pre-M46 trait signature) still silently truncates lengths
//!   above 65535 -- a pre-existing, shared limitation of the return
//!   type itself, not something new introduced by ramfs.

//! MILESTONE 62: the first Tier 2 (filesystem completeness) item -- a
//! real on-disk format upgrade adding permissions (a standard 9-bit
//! unix rwxrwxrwx `mode`), ownership (`uid`/`gid`), and real timestamps
//! (`ctime`/`mtime`, genuine unix-epoch seconds derived from the real
//! CMOS RTC Milestone 15 already wired up, via a standard, correct
//! `days_from_civil` civil-calendar algorithm added to `rtc.rs`, not an
//! invented approximation) to every `DirEntry` -- files AND
//! subdirectories alike, since both already share the exact same
//! `DirEntry` struct/on-disk slot.
//!
//! **Dependency reasoning for why this is the right first Tier 2 slice**
//! (checked against the actual code before starting, not assumed): the
//! roadmap's own wording lists "a minimal multi-user model (uid/gid,
//! chmod/chown)" as a SEPARATE item from "a robust on-disk format
//! (permissions, timestamps, hard links)", but a `chmod`/`chown` with no
//! real mode/uid/gid bytes on disk to modify would be a stub, and
//! permission bits are meaningless without an owner to compare against
//! -- so this milestone does both together: the on-disk fields AND
//! real, enforced permission checks (owner/group/other rwx, checked
//! against a kernel-global "current identity" -- see below) in every
//! `DiskFs` operation, plus `chmod`/`chown`/`stat` to inspect and change
//! them. **Hard links are deliberately NOT part of this milestone** --
//! confirmed by reading this file's actual allocation model first: a
//! `DirEntry` embeds its own `start_lba`/`sector_count` directly (no
//! inode-indirection layer separating "a name" from "the data/metadata
//! it refers to"), so a real hard link (two directory entries sharing
//! one refcounted inode) needs that indirection layer added first -- a
//! separate, genuinely bigger structural lift than extending the
//! existing per-entry fields, honestly scoped out rather than faked as
//! "two entries with identical start_lba" (which would silently break
//! `rm`'s existing free-sectors-on-delete reclamation: freeing one
//! link's sectors would corrupt the other link still pointing at them).
//! Symbolic links, `mmap()`, and a real block-device abstraction beyond
//! raw ATA are each independent, separately-scoped Tier 2 items, also
//! not attempted here.
//!
//! **The "current identity" this milestone checks permissions against**
//! is a single kernel-global `(uid, gid)` pair (`CURRENT_ID` below,
//! default `(0, 0)` == root), NOT a real per-process/per-login identity
//! -- deliberately, because a genuine multi-user OS needs real
//! persistent accounts/login first, which this roadmap already lists as
//! its own, much later Tier 8 item ("persistent accounts/login"). Until
//! that exists, a per-process uid would have nothing real to be
//! initialized FROM at process-creation time. The new `whoami`/`setid`
//! shell commands (shell.rs) read/change this global directly, with no
//! password or authentication check at all -- an explicitly disclosed
//! testing lever for exercising real enforcement, not a login system,
//! named `setid` (not `su`) specifically so it doesn't imply real
//! authentication exists.
//!
//! **Root's own permissions/ownership** need a place to live that isn't
//! a `DirEntry` -- root (`DIR_LBA`) is the one directory with no parent
//! table holding an entry FOR it. Rather than smuggling extra bytes
//! into root's own directory-table sector (where `save_dir_at` already
//! zero-fills anything past the entry table on every ordinary
//! mkdir/write/rm at root, which would silently wipe them on the very
//! next unrelated disk write), root's metadata lives in its own
//! dedicated sector, `ROOT_META_LBA` -- the first LBA past the shared
//! data pool (`FILE_DATA_START_LBA + NUM_DATA_SECTORS` = 2 + 64 = 66),
//! genuinely unused by anything else on this disk.
//!
//! **The on-disk `MAGIC` is bumped** (0x53504B46 "SPKF" ->
//! 0x53504B47) specifically because `ENTRY_LEN` changed size -- a
//! pre-M62 disk's raw bytes, reinterpreted at the NEW (larger) entry
//! stride, would misparse `start_lba`/`sector_count` as garbage rather
//! than cleanly failing. Bumping `MAGIC` makes `load_dir_at`'s existing
//! "unrecognized magic -> treat as blank/uninitialized directory"
//! fallback (already there since Milestone 18, for a truly blank disk)
//! trigger safely on any pre-M62 disk too, rather than silently
//! misinterpreting stale-format bytes -- exactly the same safe path a
//! genuinely blank disk already took, not a new code path.

//! MILESTONE 63: real symbolic links -- the next Tier 2 (filesystem
//! completeness) item, chosen over the other three remaining candidates
//! (hard links, `mmap()`, a real block-device abstraction) after
//! re-checking each against the ACTUAL current code, not just the
//! roadmap's wording:
//!
//! - **Hard links** still need the inode-indirection layer Milestone 62
//!   already identified as missing -- re-confirmed here by re-reading
//!   this file's allocation model before starting: `DirEntry` (just
//!   below) still embeds its own `start_lba`/`sector_count` directly,
//!   with every allocation/reclamation function (`write_file_disk`,
//!   `delete_file_disk`, `collect_occupied_at`) still keyed off exactly
//!   one entry owning exactly one allocation. That's unchanged since
//!   Milestone 62's judgment call and is still a genuinely bigger
//!   structural lift than one milestone slice, not attempted here.
//! - **`mmap()`** needs real file-backed pages wired into the page-fault
//!   handler and a process's address space (`process.rs`/
//!   `interrupts.rs`), on top of Milestone 57's demand paging -- the
//!   roadmap's own Tier 1 section (see near "Tier 2" below) already
//!   names copy-on-write and `mmap` together as "the next real VM
//!   increment", i.e. a bigger, separately-scoped unit of work spanning
//!   multiple subsystems, not a filesystem-only change.
//! - **A real block-device abstraction beyond raw ATA** would be a pure
//!   refactor with only one real backing device (`ata.rs`) to abstract
//!   over -- there is no second block device anywhere in this kernel
//!   yet (no ramdisk-as-block-device, no partition table, no second
//!   controller), so the abstraction would have nothing genuinely
//!   different to prove itself against beyond a mock/test double,
//!   unlike `RamFs` which Milestone 46 justified precisely because it
//!   IS a second, real, independently-verifiable backing store.
//! - **Symbolic links**, by contrast, are a self-contained filesystem
//!   feature: no new subsystem, a bounded/well-understood real-world
//!   semantic (path indirection with cycle detection), and directly
//!   verifiable end-to-end through the exact same `fs::*` surface and
//!   self-test style every prior milestone here has used. Chosen as the
//!   right-sized next slice.
//!
//! **On-disk format**: `DirEntry` gains one more field, `is_symlink:
//! bool` (1 byte), appended after Milestone 62's `mtime` -- `ENTRY_LEN`
//! grows from 39 to 40 bytes/entry, so `MAGIC` is bumped again
//! (0x53504B47 "SPKG" -> 0x53504B48 "SPKH") for the exact same reason
//! Milestone 62 bumped it: reinterpreting a pre-M63 disk's bytes at the
//! new, larger stride would misparse `start_lba`/`sector_count` as
//! garbage, so bumping `MAGIC` routes any pre-M63 disk through
//! `load_dir_at`'s existing "unrecognized magic -> blank directory"
//! fallback instead, the same safe path a genuinely blank disk already
//! took.
//!
//! **Storage**: a symlink entry has `is_dir == false`, `is_symlink ==
//! true`, and reuses the EXACT SAME allocation mechanism a small file
//! already uses (`start_lba`/`sector_count` from the shared data pool,
//! `len` = the target path's byte length) -- its "file content" IS the
//! raw target path text. Capped at `MAX_SYMLINK_LEN` (255 bytes, fits
//! in the one sector this milestone always allocates for a link) --
//! smaller than POSIX's typical 4096-byte `PATH_MAX`-based limit, a
//! disclosed, deliberately conservative cap consistent with this
//! kernel's other deliberately-small caps (8 entries/directory, 4096
//! bytes/file). `collect_occupied_at` needed NO changes: it already
//! pushes every USED entry's `(start_lba, sector_count)` regardless of
//! `is_dir`, and only recurses into entries where `is_dir` is true -- a
//! symlink (`is_dir == false`) is therefore already counted as occupied
//! and never misread as a directory table, for free.
//!
//! **Real, bounded dereferencing, exactly where it earns its keep**:
//! `resolve_dir_lba` (renamed internally to a `resolve_dir_lba_from`
//! taking an explicit base LBA + a `symlink_depth` counter, with the
//! original `resolve_dir_lba(path)` now a thin `from(DIR_LBA, path, 0)`
//! wrapper) dereferences a symlink encountered as ANY directory-path
//! component -- intermediate OR the final one, since resolving "the
//! table this directory path names" treats every component uniformly
//! (this is what makes `cd`/`ls`/every multi-level path argument
//! transparently walk THROUGH a symlinked directory, the direct
//! generalization of Milestone 32's own arbitrary-depth-nesting
//! precedent). A relative target (no leading `/`) resolves starting at
//! the directory CONTAINING the symlink (real unix semantics); an
//! absolute target (`/`-prefixed) resolves from root. Symlink chains
//! (link -> link -> ... -> real entry) are followed up to
//! `MAX_SYMLINK_DEPTH` (8) hops before a real, disclosed "too many
//! levels of symbolic links" error (the real ELOOP condition), and a
//! broken link (target doesn't exist) is a real, disclosed "no such
//! file or directory" error too -- neither silently swallowed.
//! `read_file` additionally dereferences when the FINAL path component
//! itself names a symlink to a file (`deref_leaf`), so `read`-ing
//! through a symlink genuinely reaches the target's real bytes.
//!
//! **Real, disclosed scope cuts** (each a deliberate choice, not an
//! oversight -- see the self-test and its own comments for direct
//! proof of each): (1) `stat` on a symlink reports the LINK's own
//! metadata (`is_symlink: true`, `len` = target text length) and does
//! NOT follow -- effectively `lstat` semantics, not `stat` semantics;
//! `readlink` is the real, separate way to read what it points at, and
//! a caller wanting the TARGET's metadata can `readlink` then `stat`
//! that path themselves. (2) `chmod`/`chown` likewise act on the
//! symlink entry itself, never following -- consistent with (1), and
//! harmless in practice since this kernel's own permission checks never
//! consult a symlink's OWN mode bits during traversal anyway (real
//! Linux behavior: symlink permissions are ignored, only the resolved
//! target's mode matters, and that's exactly what `resolve_dir_lba_from`
//! already checks on the DEREFERENCED entry). (3) `write_file` on a
//! path whose FINAL component is an EXISTING symlink is refused with a
//! real, clear, disclosed error rather than either silently overwriting
//! the link itself with file content (which would corrupt/orphan its
//! target) or silently writing through it (which would need threading a
//! dereferenced target table/name back through the create-vs-overwrite
//! and permission-check logic above it -- a real, separately-scoped
//! restructuring this slice deliberately does not attempt). (4)
//! `delete_file` (`rm`) and `remove_dir` (`rmdir`) already act on the
//! raw entry with NO changes needed -- this is not a cut at all, it's
//! genuinely correct real unix behavior (`rm` removes the link, never
//! the target; `rmdir` on a symlink-to-directory correctly refuses with
//! "is a file, not a directory" since a symlink's own `is_dir` is
//! always `false`, matching real `rmdir`'s ENOTDIR on a symlink). (5)
//! `RamFs` gained no symlink support at all (same reasoning as its
//! pre-existing no-subdirectories/no-permissions cuts) -- `symlink`/
//! `readlink` against a `ram/...` path are real, disclosed "not
//! supported" errors. (6) A symlink's target text is stored and
//! resolved PURELY within the backend it was created on (disk-internal
//! LBA resolution, never routed back through `resolve_backend`) -- a
//! disk symlink cannot point into `ram/...` or vice versa, consistent
//! with `RamFs` already being a genuinely separate, non-nested store.
//! (7) no `.`/`..` special-path handling was added anywhere (symlink
//! targets or otherwise) -- this fs has never supported them, at any
//! milestone, so a symlink target containing them is just looked up as
//! a literal (and normally nonexistent) component name, the same
//! pre-existing behavior every other path in this file already has.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;
use spin::Mutex;
use crate::serial;

const DIR_LBA: u32 = 1;
const MAX_ENTRIES: usize = 8;
const FILE_DATA_START_LBA: u32 = 2;
const MAGIC: u32 = 0x53504B48; // "SPKH" -- bumped from 0x53504B47 ("SPKG") for Milestone 63's on-disk format change (added is_symlink); see module doc comment for why
const NAME_LEN: usize = 16;
// MILESTONE 62: extended with real mode/uid/gid/ctime/mtime fields --
// name + used-flag + is_dir-flag + u16 len + u32 start_lba +
// u8 sector_count + u16 mode + u16 uid + u16 gid + u32 ctime + u32 mtime
// = 16+1+1+2+4+1+2+2+2+4+4 = 39 bytes (was 25 pre-M62).
// MILESTONE 63: one more byte, is_symlink -- 39 -> 40 bytes/entry.
const ENTRY_LEN: usize = NAME_LEN + 1 + 1 + 2 + 4 + 1 + 2 + 2 + 2 + 4 + 4 + 1;
const SECTOR_SIZE: usize = 512;
const MAX_FILE_SECTORS: usize = 8; // real per-file cap: 8 * 512 = 4096 bytes
// MILESTONE 35: pub(crate) (was private) so process.rs's fdwrite-syscall
// implementation (write_fd()) can cap an open fd's in-kernel write
// buffer against the EXACT same real limit fs::write_file() itself
// enforces, rather than a second, potentially-drifting copy of "4096".
pub(crate) const MAX_FILE_BYTES: usize = MAX_FILE_SECTORS * SECTOR_SIZE;
const NUM_DATA_SECTORS: usize = MAX_ENTRIES * MAX_FILE_SECTORS; // shared pool, used by both file data AND subdirectory tables at any depth

// MILESTONE 62: root's own permission/ownership/timestamp metadata --
// see module doc comment for why root needs a dedicated sector rather
// than a DirEntry (it has no parent table to hold one).
const ROOT_META_LBA: u32 = FILE_DATA_START_LBA + NUM_DATA_SECTORS as u32; // 2 + 64 = 66, first free LBA past the shared data pool
const ROOT_META_MAGIC: u32 = 0x524D4554; // "RMET"

// MILESTONE 62: standard unix permission-bit meanings (not invented --
// the real octal rwx values every unix mode_t uses).
const PERM_READ: u16 = 0o4;
const PERM_WRITE: u16 = 0o2;
const PERM_EXEC: u16 = 0o1;
const DEFAULT_FILE_MODE: u16 = 0o644; // rw-r--r--, the conventional unix default with no umask concept yet (disclosed -- no umask exists in this kernel)
const DEFAULT_DIR_MODE: u16 = 0o755; // rwxr-xr-x, the conventional unix default directory mode

// MILESTONE 32: defensive recursion guard for resolve_dir_lba and
// collect_occupied -- see the module doc comment above for why this is
// a safety net against a hypothetical corrupted/cyclic disk, not a
// deliberately chosen feature limit.
const MAX_DEPTH: usize = 32;

// MILESTONE 63: bounded symlink-chain dereference depth -- the real
// ELOOP guard (a smaller, disclosed cap than real Linux's ~40, matching
// this kernel's own deliberately-small-cap style elsewhere).
const MAX_SYMLINK_DEPTH: usize = 8;
// MILESTONE 63: a symlink's target text is stored exactly like a tiny
// file's content (see module doc comment) -- capped smaller than a
// full 512-byte sector so it always fits in the one sector this
// milestone always allocates for a link, with room to spare.
const MAX_SYMLINK_LEN: usize = 255;

#[derive(Clone, Copy)]
struct DirEntry {
    used: bool,
    is_dir: bool,
    name: [u8; NAME_LEN],
    len: u16,
    start_lba: u32,
    sector_count: u8,
    // MILESTONE 62: real permission/ownership/timestamp fields, on
    // every entry (file or subdirectory alike -- both use this same
    // struct).
    mode: u16,
    uid: u16,
    gid: u16,
    ctime: u32,
    mtime: u32,
    // MILESTONE 63: true for a symbolic link -- always paired with
    // is_dir == false. Its start_lba/sector_count/len point at the raw
    // target-path text, stored exactly like a small file's content (see
    // module doc comment).
    is_symlink: bool,
}

impl DirEntry {
    fn empty() -> Self {
        DirEntry {
            used: false,
            is_dir: false,
            name: [0; NAME_LEN],
            len: 0,
            start_lba: 0,
            sector_count: 0,
            mode: 0,
            uid: 0,
            gid: 0,
            ctime: 0,
            mtime: 0,
            is_symlink: false,
        }
    }

    fn name_str(&self) -> String {
        let end = self.name.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
        String::from_utf8_lossy(&self.name[..end]).to_string()
    }
}

/// MILESTONE 62: the real, standard unix rwxrwxrwx permission-check
/// rule -- root (`uid == 0`) bypasses every check (standard unix
/// superuser behavior), otherwise the owner/group/other 3-bit field is
/// picked by whether the CALLER's identity matches the entry's owner,
/// its group, or neither, exactly like a real `mode_t` check.
fn entry_allows(mode: u16, entry_uid: u16, entry_gid: u16, uid: u16, gid: u16, need: u16) -> bool {
    if uid == 0 {
        return true;
    }
    let bits = if uid == entry_uid {
        (mode >> 6) & 0o7
    } else if gid == entry_gid {
        (mode >> 3) & 0o7
    } else {
        mode & 0o7
    };
    (bits & need) == need
}

// MILESTONE 62: the kernel-global "current identity" self-tests and
// shell chmod/chown/permission checks run against -- see module doc
// comment for why this is deliberately global (not per-process) until a
// real login system (Tier 8) exists.
static CURRENT_ID: Mutex<(u16, u16)> = Mutex::new((0, 0));

pub fn current_uid() -> u16 {
    CURRENT_ID.lock().0
}

pub fn current_gid() -> u16 {
    CURRENT_ID.lock().1
}

pub fn set_current_identity(uid: u16, gid: u16) {
    *CURRENT_ID.lock() = (uid, gid);
}

/// MILESTONE 62: root's own permission/ownership/timestamp metadata --
/// see module doc comment for why this lives in its own dedicated
/// sector rather than a `DirEntry`.
struct RootMeta {
    mode: u16,
    uid: u16,
    gid: u16,
    ctime: u32,
    mtime: u32,
}

impl RootMeta {
    fn default_meta() -> Self {
        RootMeta { mode: DEFAULT_DIR_MODE, uid: 0, gid: 0, ctime: 0, mtime: 0 }
    }
}

fn load_root_meta() -> RootMeta {
    let mut buf = [0u8; 512];
    if crate::ata::read_sector(ROOT_META_LBA, &mut buf).is_err() {
        return RootMeta::default_meta();
    }
    let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    if magic != ROOT_META_MAGIC {
        return RootMeta::default_meta(); // uninitialized -- real defaults, same "blank == not an error" fallback the rest of this file already uses
    }
    RootMeta {
        mode: u16::from_le_bytes(buf[4..6].try_into().unwrap()),
        uid: u16::from_le_bytes(buf[6..8].try_into().unwrap()),
        gid: u16::from_le_bytes(buf[8..10].try_into().unwrap()),
        ctime: u32::from_le_bytes(buf[10..14].try_into().unwrap()),
        mtime: u32::from_le_bytes(buf[14..18].try_into().unwrap()),
    }
}

fn save_root_meta(meta: &RootMeta) -> Result<(), &'static str> {
    let mut buf = [0u8; 512];
    buf[0..4].copy_from_slice(&ROOT_META_MAGIC.to_le_bytes());
    buf[4..6].copy_from_slice(&meta.mode.to_le_bytes());
    buf[6..8].copy_from_slice(&meta.uid.to_le_bytes());
    buf[8..10].copy_from_slice(&meta.gid.to_le_bytes());
    buf[10..14].copy_from_slice(&meta.ctime.to_le_bytes());
    buf[14..18].copy_from_slice(&meta.mtime.to_le_bytes());
    crate::ata::write_sector(ROOT_META_LBA, &buf)
}

// Loads a directory table (root OR any subdirectory's own one-sector
// table, at any depth) from an arbitrary LBA -- root always lives at
// the fixed DIR_LBA, a subdirectory's table lives wherever it was
// allocated out of the shared data pool, recorded as that
// subdirectory's own start_lba in its parent's entry.
fn load_dir_at(lba: u32) -> [DirEntry; MAX_ENTRIES] {
    let mut buf = [0u8; 512];
    let mut entries = [DirEntry::empty(); MAX_ENTRIES];
    if crate::ata::read_sector(lba, &mut buf).is_err() {
        return entries;
    }
    let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    if magic != MAGIC {
        return entries; // uninitialized disk -- empty directory, not an error
    }
    for i in 0..MAX_ENTRIES {
        let off = 4 + i * ENTRY_LEN;
        let used = buf[off] != 0;
        let is_dir = buf[off + 1] != 0;
        let mut name = [0u8; NAME_LEN];
        name.copy_from_slice(&buf[off + 2..off + 2 + NAME_LEN]);
        let len = u16::from_le_bytes(buf[off + 18..off + 20].try_into().unwrap());
        let start_lba = u32::from_le_bytes(buf[off + 20..off + 24].try_into().unwrap());
        let sector_count = buf[off + 24];
        // MILESTONE 62: mode/uid/gid/ctime/mtime, appended after the
        // pre-M62 fields (offsets 25..39 within each 39-byte entry).
        let mode = u16::from_le_bytes(buf[off + 25..off + 27].try_into().unwrap());
        let uid = u16::from_le_bytes(buf[off + 27..off + 29].try_into().unwrap());
        let gid = u16::from_le_bytes(buf[off + 29..off + 31].try_into().unwrap());
        let ctime = u32::from_le_bytes(buf[off + 31..off + 35].try_into().unwrap());
        let mtime = u32::from_le_bytes(buf[off + 35..off + 39].try_into().unwrap());
        // MILESTONE 63: is_symlink, appended after mtime (offset 39
        // within each now-40-byte entry).
        let is_symlink = buf[off + 39] != 0;
        entries[i] =
            DirEntry { used, is_dir, name, len, start_lba, sector_count, mode, uid, gid, ctime, mtime, is_symlink };
    }
    entries
}

fn save_dir_at(lba: u32, entries: &[DirEntry; MAX_ENTRIES]) -> Result<(), &'static str> {
    let mut buf = [0u8; 512];
    buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    for (i, e) in entries.iter().enumerate() {
        let off = 4 + i * ENTRY_LEN;
        buf[off] = e.used as u8;
        buf[off + 1] = e.is_dir as u8;
        buf[off + 2..off + 2 + NAME_LEN].copy_from_slice(&e.name);
        buf[off + 18..off + 20].copy_from_slice(&e.len.to_le_bytes());
        buf[off + 20..off + 24].copy_from_slice(&e.start_lba.to_le_bytes());
        buf[off + 24] = e.sector_count;
        buf[off + 25..off + 27].copy_from_slice(&e.mode.to_le_bytes());
        buf[off + 27..off + 29].copy_from_slice(&e.uid.to_le_bytes());
        buf[off + 29..off + 31].copy_from_slice(&e.gid.to_le_bytes());
        buf[off + 31..off + 35].copy_from_slice(&e.ctime.to_le_bytes());
        buf[off + 35..off + 39].copy_from_slice(&e.mtime.to_le_bytes());
        buf[off + 39] = e.is_symlink as u8;
    }
    crate::ata::write_sector(lba, &buf)
}

/// MILESTONE 62: resolves the real, on-disk (mode, uid, gid) of the
/// directory NAMED by `dir_path` (root when empty) -- used for the
/// real unix "creating/deleting an entry needs WRITE permission on the
/// PARENT directory, not the target" and "listing needs READ
/// permission on the directory being listed" checks. Reuses
/// `resolve_table` for the non-root case, so the same traversal
/// (search/execute) permission checks `resolve_dir_lba` already
/// enforces along the way apply here too -- no separate plumbing.
fn dir_meta_at(dir_path: &str) -> Result<(u16, u16, u16), String> {
    if dir_path.is_empty() {
        let m = load_root_meta();
        return Ok((m.mode, m.uid, m.gid));
    }
    let (parent_lba, leaf) = resolve_table(dir_path)?;
    let table = load_dir_at(parent_lba);
    let target = name_bytes(leaf);
    let raw = table.iter().find(|e| e.used && e.name == target).copied().ok_or_else(|| format!("no such directory '{leaf}'"))?;
    // MILESTONE 63: if the FINAL component of dir_path is itself a
    // symlink (e.g. `ls symlinktodir`), dereference it here too -- every
    // INTERMEDIATE component was already dereferenced by resolve_dir_lba
    // (via resolve_table's dir_part resolution above), but the leaf
    // lookup here is a separate, raw lookup that needs the same
    // treatment for the "symlink-to-directory is the whole argument"
    // case.
    let e = if raw.is_symlink { deref_symlink_entry(raw, parent_lba, 0)?.1 } else { raw };
    if !e.is_dir {
        return Err(format!("no such directory '{leaf}'"));
    }
    Ok((e.mode, e.uid, e.gid))
}

/// MILESTONE 63: reads a symlink entry's stored target-path text back
/// out -- its "file content" is exactly the raw target string, stored
/// via the same sector-based allocation a small file already uses (see
/// module doc comment).
fn read_symlink_target(entry: &DirEntry) -> Result<String, String> {
    if entry.sector_count == 0 {
        return Ok(String::new());
    }
    let mut data = Vec::with_capacity(entry.sector_count as usize * SECTOR_SIZE);
    let mut sector = [0u8; SECTOR_SIZE];
    for i in 0..entry.sector_count as u32 {
        crate::ata::read_sector(entry.start_lba + i, &mut sector).map_err(|e| e.to_string())?;
        data.extend_from_slice(&sector);
    }
    data.truncate(entry.len as usize);
    String::from_utf8(data).map_err(|_| "symlink target is not valid UTF-8 (corrupted?)".to_string())
}

/// MILESTONE 63: follows a symlink entry to whatever it ultimately
/// names -- a plain file/directory, or another symlink (followed again,
/// bounded by `MAX_SYMLINK_DEPTH`). `containing_lba` is the table the
/// symlink entry ITSELF lives in, needed to resolve a relative target
/// (real unix semantics: relative to the symlink's own location, not
/// the caller's CWD -- this fs has no CWD concept anyway, see
/// shell.rs). Returns the table LBA that directly contains the final,
/// non-symlink entry, plus that entry itself.
fn deref_symlink_entry(mut entry: DirEntry, mut containing_lba: u32, mut depth: usize) -> Result<(u32, DirEntry), String> {
    while entry.is_symlink {
        depth += 1;
        if depth > MAX_SYMLINK_DEPTH {
            return Err("too many levels of symbolic links".to_string());
        }
        let target_path = read_symlink_target(&entry)?;
        let (base, rel): (u32, &str) = match target_path.strip_prefix('/') {
            Some(stripped) => (DIR_LBA, stripped),
            None => (containing_lba, target_path.as_str()),
        };
        let (dir_part, leaf_part) = match rel.rsplit_once('/') {
            Some((d, l)) => (d, l),
            None => ("", rel),
        };
        if leaf_part.is_empty() {
            return Err(format!("broken symlink -- target '{target_path}' has no name"));
        }
        let new_table_lba = resolve_dir_lba_from(base, dir_part, depth)?;
        let table = load_dir_at(new_table_lba);
        let next = table
            .iter()
            .find(|e| e.used && e.name == name_bytes(leaf_part))
            .copied()
            .ok_or_else(|| format!("no such file or directory (broken symlink -> '{target_path}')"))?;
        containing_lba = new_table_lba;
        entry = next;
    }
    Ok((containing_lba, entry))
}

fn name_bytes(name: &str) -> [u8; NAME_LEN] {
    let mut out = [0u8; NAME_LEN];
    let bytes = name.as_bytes();
    let n = bytes.len().min(NAME_LEN);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

// MILESTONE 32: walks an arbitrary-depth, '/'-separated chain of
// directory components starting at the root table (DIR_LBA), returning
// the LBA of the table found at the end of the chain. An empty path
// resolves to root itself (the base case used when a leaf lives
// directly at the top level). Each component in between must already
// exist and be a directory -- this is the generalization of Milestone
// 28's one-level-only `resolve_dir_lba`, made possible with NO on-disk
// format change (see module doc comment).
fn resolve_dir_lba(path: &str) -> Result<u32, String> {
    resolve_dir_lba_from(DIR_LBA, path, 0)
}

/// MILESTONE 63: generalizes the pre-M63 `resolve_dir_lba` with an
/// explicit starting LBA (`base`) and a threaded `symlink_depth`
/// counter, so it can be re-entered starting at an arbitrary directory
/// -- specifically, by `deref_symlink_entry` above, to resolve a
/// relative symlink target's directory part starting at the symlink's
/// OWN containing directory rather than always at root. `base ==
/// DIR_LBA` is still the normal, everyday case (every pre-M63 caller
/// goes through the `resolve_dir_lba` wrapper above, which is exactly
/// that). The real search(x)-bit permission check on `base` itself only
/// runs when `base == DIR_LBA` (root) -- a non-root base is always some
/// directory the caller already validated exec-access to on the way
/// there (either as a normal path component below, whose exec bit is
/// checked before `lba` advances into it, or as the containing
/// directory of a symlink being dereferenced, which was itself entered
/// the same way), so re-checking it here would be redundant, not a gap.
fn resolve_dir_lba_from(base: u32, path: &str, symlink_depth: usize) -> Result<u32, String> {
    if path.is_empty() {
        return Ok(base);
    }
    if symlink_depth > MAX_SYMLINK_DEPTH {
        return Err("too many levels of symbolic links".to_string());
    }
    // MILESTONE 62: real search (execute) permission check on root
    // itself before even looking up the first component -- standard
    // unix semantics: traversing INTO a directory (not just reading its
    // listing) requires the x bit, checked against the caller's real
    // current identity.
    let uid = current_uid();
    let gid = current_gid();
    if base == DIR_LBA {
        let root_meta = load_root_meta();
        if !entry_allows(root_meta.mode, root_meta.uid, root_meta.gid, uid, gid, PERM_EXEC) {
            return Err("permission denied -- no search permission on '/'".to_string());
        }
    }
    let mut lba = base;
    for (depth, comp) in path.split('/').enumerate() {
        if comp.is_empty() {
            return Err(format!("'{path}' -- invalid path (empty component)"));
        }
        if depth >= MAX_DEPTH {
            return Err(format!("'{path}' -- path too deep (max {MAX_DEPTH} levels)"));
        }
        let table = load_dir_at(lba);
        let target = name_bytes(comp);
        let mut entry =
            table.iter().find(|e| e.used && e.name == target).copied().ok_or_else(|| format!("no such directory '{comp}'"))?;
        // MILESTONE 63: real, bounded symlink dereferencing -- a
        // symlink encountered as ANY component (intermediate or the
        // final one) of a directory path is followed to whatever it
        // ultimately names, exactly like Milestone 32's own
        // arbitrary-depth traversal generalization. Must land on a real
        // directory to keep being usable as a path component.
        if entry.is_symlink {
            entry = deref_symlink_entry(entry, lba, symlink_depth)?.1;
        }
        if !entry.is_dir {
            return Err(format!("'{comp}' is a file, not a directory"));
        }
        // MILESTONE 62: real search permission check on every
        // intermediate/target directory as it's entered -- checked
        // against the REAL, DEREFERENCED entry's own mode (real Linux
        // behavior: a symlink's own permission bits are never
        // consulted, only the resolved target's).
        if !entry_allows(entry.mode, entry.uid, entry.gid, uid, gid, PERM_EXEC) {
            return Err(format!("permission denied -- no search permission on '{comp}'"));
        }
        lba = entry.start_lba;
    }
    Ok(lba)
}

/// MILESTONE 63: looks up `name` in the table at `table_lba` and, if
/// it's a symlink, dereferences it to whatever it ultimately names
/// (bounded, real ELOOP/broken-link errors -- see `deref_symlink_entry`
/// above). Used by `read_file_disk` so reading through a symlink
/// reaches the real target's bytes -- see the module doc comment for
/// exactly which other operations deliberately do NOT use this (they
/// act on the symlink entry itself instead).
fn deref_leaf(table_lba: u32, name: &str) -> Result<(u32, DirEntry), String> {
    let table = load_dir_at(table_lba);
    let entry = table.iter().find(|e| e.used && e.name == name_bytes(name)).copied().ok_or_else(|| format_no_such_file(name))?;
    if entry.is_symlink {
        deref_symlink_entry(entry, table_lba, 0)
    } else {
        Ok((table_lba, entry))
    }
}

// Splits a write/read/rm/mkdir/rmdir path into the directory table its
// leaf lives in (root, or an arbitrary-depth resolved subdirectory) and
// the leaf name within that table. MILESTONE 32: previously rejected
// anything past one `DIR/NAME` level; now delegates the "directory
// part" (everything before the final '/') to the arbitrary-depth
// resolve_dir_lba above.
fn resolve_table<'a>(path: &'a str) -> Result<(u32, &'a str), String> {
    match path.rsplit_once('/') {
        Some((dir, name)) => {
            if name.is_empty() {
                return Err(format!("'{path}' -- missing file/directory name"));
            }
            Ok((resolve_dir_lba(dir)?, name))
        }
        None => Ok((DIR_LBA, path)),
    }
}

// Every currently-occupied pool sector, disk-wide -- both files' data
// AND every subdirectory's own one-sector table, at ANY depth now
// (MILESTONE 32 generalizes this from Milestone 28's one-level-only
// recursion) -- so allocation never hands out a sector already claimed
// by some OTHER directory's entry anywhere in the tree. `exclude`, when
// given, is the (start_lba, sector_count) of the allocation currently
// being replaced in-place, so rewriting an existing file/subdir doesn't
// see its own old sectors as busy. `MAX_DEPTH` is the same defensive
// cycle guard as resolve_dir_lba, not a feature limit.
fn collect_occupied(exclude: Option<(u32, u8)>) -> Vec<(u32, u8)> {
    let mut ranges = Vec::new();
    collect_occupied_at(DIR_LBA, &mut ranges, 0);
    if let Some(ex) = exclude {
        if let Some(pos) = ranges.iter().position(|&r| r == ex) {
            ranges.remove(pos);
        }
    }
    ranges
}

fn collect_occupied_at(table_lba: u32, ranges: &mut Vec<(u32, u8)>, depth: usize) {
    if depth >= MAX_DEPTH {
        return; // corrupted/cyclic disk guard -- see module doc comment
    }
    for e in load_dir_at(table_lba).iter() {
        if !e.used || e.sector_count == 0 {
            continue;
        }
        ranges.push((e.start_lba, e.sector_count));
        if e.is_dir {
            collect_occupied_at(e.start_lba, ranges, depth + 1);
        }
    }
}

// First-fit scan over the shared data pool (pool-relative sector
// indices, 0..NUM_DATA_SECTORS) for a run of `needed` free sectors,
// against a disk-wide occupancy list from collect_occupied.
fn find_free_span(occupied: &[(u32, u8)], needed: usize) -> Option<u32> {
    'outer: for start in 0..=(NUM_DATA_SECTORS - needed) {
        let cand_start = FILE_DATA_START_LBA + start as u32;
        let cand_end = cand_start + needed as u32;
        for &(occ_start, occ_count) in occupied {
            let occ_end = occ_start + occ_count as u32;
            if cand_start < occ_end && occ_start < cand_end {
                continue 'outer;
            }
        }
        return Some(start as u32);
    }
    None
}

// MILESTONE 32: `path` used to be a single bare name restricted to
// directly-under-root; it's now a full path (e.g. `a/b/c`), resolved
// via the same resolve_table every file operation uses, so a new
// directory can be created at any already-existing depth. The shell
// layers a real `cd`/CWD concept on top so a user normally just types
// the leaf name (see shell.rs's `resolve_arg`).
fn make_dir_disk(path: &str) -> Result<(), String> {
    // MILESTONE 62: real unix rule -- creating an entry needs WRITE
    // permission on the PARENT directory, not the (not-yet-existing)
    // target. `resolve_table` below already enforces search/execute
    // permission on every ancestor including this immediate parent (via
    // resolve_dir_lba), so only the write check is new here.
    let dir_part = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let (pmode, puid, pgid) = dir_meta_at(dir_part)?;
    if !entry_allows(pmode, puid, pgid, current_uid(), current_gid(), PERM_WRITE) {
        return Err("mkdir: permission denied (no write permission on parent directory)".to_string());
    }

    let (table_lba, name) = resolve_table(path)?;
    let mut table = load_dir_at(table_lba);
    let target_name = name_bytes(name);

    if table.iter().any(|e| e.used && e.name == target_name) {
        return Err(format!("'{name}' already exists"));
    }
    let slot = table.iter().position(|e| !e.used).ok_or_else(|| "directory full (max 8 entries)".to_string())?;

    let occupied = collect_occupied(None);
    let start_pool =
        find_free_span(&occupied, 1).ok_or_else(|| "not enough free disk space (fragmented or full)".to_string())?;
    let new_table_lba = FILE_DATA_START_LBA + start_pool;

    save_dir_at(new_table_lba, &[DirEntry::empty(); MAX_ENTRIES]).map_err(|e| e.to_string())?;

    let now = crate::rtc::to_unix_timestamp(crate::rtc::now());
    table[slot] = DirEntry {
        used: true,
        is_dir: true,
        name: target_name,
        len: 0,
        start_lba: new_table_lba,
        sector_count: 1,
        mode: DEFAULT_DIR_MODE,
        uid: current_uid(),
        gid: current_gid(),
        ctime: now,
        mtime: now,
        is_symlink: false,
    };
    save_dir_at(table_lba, &table).map_err(|e| e.to_string())?;
    Ok(())
}

fn write_file_disk(path: &str, data: &[u8]) -> Result<(), String> {
    if data.len() > MAX_FILE_BYTES {
        return Err(format!(
            "file too large -- max {MAX_FILE_BYTES} bytes ({MAX_FILE_SECTORS} sectors per file, no bigger files yet)"
        ));
    }
    let (table_lba, name) = resolve_table(path)?;
    let mut entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);

    let existing = entries.iter().position(|e| e.used && e.name == target_name);
    if let Some(i) = existing {
        if entries[i].is_dir {
            return Err(format!("'{name}' is a directory"));
        }
        // MILESTONE 63: real, disclosed scope cut -- writing to an
        // EXISTING symlink is refused rather than either silently
        // overwriting the link itself with file content (corrupting/
        // orphaning its target) or silently writing through it (which
        // would need the create-vs-overwrite and permission logic above
        // reworked around a dereferenced target table/name -- a bigger,
        // separately-scoped change; see module doc comment).
        if entries[i].is_symlink {
            return Err(format!(
                "'{name}' is a symbolic link -- write-through-symlink not supported this milestone (remove it first with rm, or write its target path directly)"
            ));
        }
        // MILESTONE 62: real unix rule -- overwriting an EXISTING file
        // needs WRITE permission on the file itself.
        if !entry_allows(entries[i].mode, entries[i].uid, entries[i].gid, current_uid(), current_gid(), PERM_WRITE) {
            return Err(format!("'{name}' -- permission denied (write)"));
        }
    } else {
        // MILESTONE 62: real unix rule -- CREATING a brand-new entry
        // needs WRITE permission on the PARENT directory instead
        // (there's no target-owned mode yet to check).
        let dir_part = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
        let (pmode, puid, pgid) = dir_meta_at(dir_part)?;
        if !entry_allows(pmode, puid, pgid, current_uid(), current_gid(), PERM_WRITE) {
            return Err("permission denied (no write permission on parent directory)".to_string());
        }
    }
    let slot = existing
        .or_else(|| entries.iter().position(|e| !e.used))
        .ok_or_else(|| "directory full (max 8 entries)".to_string())?;

    let old_alloc = if entries[slot].used { Some((entries[slot].start_lba, entries[slot].sector_count)) } else { None };
    let needed = if data.is_empty() { 0 } else { (data.len() + SECTOR_SIZE - 1) / SECTOR_SIZE };

    let start_pool = if needed == 0 {
        0
    } else {
        let occupied = collect_occupied(old_alloc);
        find_free_span(&occupied, needed).ok_or_else(|| "not enough free disk space (fragmented or full)".to_string())?
    };

    for i in 0..needed {
        let mut sector = [0u8; SECTOR_SIZE];
        let start = i * SECTOR_SIZE;
        let end = (start + SECTOR_SIZE).min(data.len());
        sector[..end - start].copy_from_slice(&data[start..end]);
        crate::ata::write_sector(FILE_DATA_START_LBA + start_pool + i as u32, &sector).map_err(|e| e.to_string())?;
    }

    // MILESTONE 62: real metadata -- a fresh create gets the real
    // default mode + the real CURRENT caller as owner + a real
    // creation timestamp; an overwrite PRESERVES the existing
    // mode/uid/gid/ctime (real unix `write()` never resets permissions
    // or ownership) and only bumps `mtime`.
    let now = crate::rtc::to_unix_timestamp(crate::rtc::now());
    let (mode, uid, gid, ctime) = match existing {
        Some(i) => (entries[i].mode, entries[i].uid, entries[i].gid, entries[i].ctime),
        None => (DEFAULT_FILE_MODE, current_uid(), current_gid(), now),
    };

    entries[slot] = DirEntry {
        used: true,
        is_dir: false,
        name: target_name,
        len: data.len() as u16,
        start_lba: if needed == 0 { 0 } else { FILE_DATA_START_LBA + start_pool },
        sector_count: needed as u8,
        mode,
        uid,
        gid,
        ctime,
        mtime: now,
        is_symlink: false,
    };
    save_dir_at(table_lba, &entries).map_err(|e| e.to_string())?;
    Ok(())
}

fn read_file_disk(path: &str) -> Result<Vec<u8>, String> {
    let (table_lba, name) = resolve_table(path)?;
    // MILESTONE 63: real dereference -- if the FINAL path component
    // names a symlink, follow it (bounded, real ELOOP/broken-link
    // errors) to reach the actual target's bytes, matching real open()-
    // for-read-through-a-symlink behavior.
    let (_final_table_lba, entry) = deref_leaf(table_lba, name)?;
    if entry.is_dir {
        return Err(format!("'{name}' is a directory, not a file"));
    }
    // MILESTONE 62: real unix rule -- reading a file needs READ
    // permission on the file itself.
    if !entry_allows(entry.mode, entry.uid, entry.gid, current_uid(), current_gid(), PERM_READ) {
        return Err(format!("'{name}' -- permission denied (read)"));
    }

    if entry.sector_count == 0 {
        return Ok(Vec::new());
    }

    let mut data = Vec::with_capacity(entry.sector_count as usize * SECTOR_SIZE);
    let mut sector = [0u8; SECTOR_SIZE];
    for i in 0..entry.sector_count as u32 {
        crate::ata::read_sector(entry.start_lba + i, &mut sector).map_err(|e| e.to_string())?;
        data.extend_from_slice(&sector);
    }
    data.truncate(entry.len as usize);
    Ok(data)
}

/// Real, boot-time, non-interactive proof that write_file()/read_file()
/// actually reach the disk -- every prior "verified" disk write in this
/// kernel's history relied on an interactive shell command (`write`,
/// `seedfdtest`, `seedtestelf`, `seedpipetest`), all gated on QEMU's
/// `sendkey` reaching the guest. This exercises the exact same
/// write_file()/read_file() path with zero interactive input, so a
/// broken sendkey path (or a missing secondary-ATA-drive QEMU argument,
/// the drive write_file/read_file's ata::write_sector/read_sector calls
/// actually depend on) shows up in the boot serial log on every single
/// run, not just when someone remembers to test it by hand.
pub fn self_test_disk_write() {
    const PATH: &str = "selftestwrite";
    const CONTENT: &[u8] = b"fs self-test disk write/read roundtrip";
    let result = write_file(PATH, CONTENT).and_then(|_| read_file(PATH));
    match result {
        Ok(bytes) if bytes == CONTENT => {
            let _ = writeln!(
                serial(),
                "fs self-test: disk write+read roundtrip OK -- real bytes matched"
            );
        }
        Ok(bytes) => {
            let _ = writeln!(
                serial(),
                "fs self-test: disk write+read MISMATCH -- wrote {} bytes, read back {} bytes",
                CONTENT.len(),
                bytes.len()
            );
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: disk write+read FAILED -- {e}");
        }
    }
}

fn delete_file_disk(path: &str) -> Result<(), String> {
    // MILESTONE 62: real unix rule -- unlinking a file needs WRITE
    // permission on the PARENT directory, not the file's own
    // permission bits (real unix `rm` behaves the same way: you can
    // delete a file you have no read/write permission on, as long as
    // you can write to the directory it lives in).
    let dir_part = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let (pmode, puid, pgid) = dir_meta_at(dir_part)?;
    if !entry_allows(pmode, puid, pgid, current_uid(), current_gid(), PERM_WRITE) {
        return Err("rm: permission denied (no write permission on parent directory)".to_string());
    }

    let (table_lba, name) = resolve_table(path)?;
    let mut entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);
    let slot = entries.iter().position(|e| e.used && e.name == target_name).ok_or_else(|| format_no_such_file(name))?;
    if entries[slot].is_dir {
        return Err(format!("'{name}' is a directory -- use rmdir to remove an (empty) directory, rm only removes files"));
    }
    // MILESTONE 63: no change needed here -- `rm` on a symlink already
    // acts on the raw entry (removes the link itself, frees its own
    // one-sector allocation), never the target it points at. This is
    // real, correct unix `unlink()` behavior, not a symlink-aware
    // special case.
    entries[slot] = DirEntry::empty(); // frees both the directory slot and its data-pool sectors for reuse
    save_dir_at(table_lba, &entries).map_err(|e| e.to_string())?;
    Ok(())
}

// MILESTONE 32: removes an EMPTY directory -- a real check (loads the
// target's own table and refuses unless every one of its entries is
// unused), matching real-world `rmdir` semantics rather than silently
// recursing into a "rm -rf". Mirrors delete_file's reclamation exactly:
// clearing the parent's slot frees both that slot AND the directory's
// one-sector table back into the shared pool, recomputed the next time
// collect_occupied runs -- the identical mechanism Milestone 22 built
// for file deletes, just applied to a directory's own entry instead of
// a file's.
fn remove_dir_disk(path: &str) -> Result<(), String> {
    // MILESTONE 62: same real unix rule as delete_file_disk above --
    // rmdir needs WRITE permission on the PARENT directory.
    let dir_part = path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let (pmode, puid, pgid) = dir_meta_at(dir_part)?;
    if !entry_allows(pmode, puid, pgid, current_uid(), current_gid(), PERM_WRITE) {
        return Err("rmdir: permission denied (no write permission on parent directory)".to_string());
    }

    let (table_lba, name) = resolve_table(path)?;
    let mut entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);
    let slot = entries.iter().position(|e| e.used && e.name == target_name).ok_or_else(|| format_no_such_file(name))?;
    if !entries[slot].is_dir {
        // MILESTONE 63: no change needed -- a symlink's own `is_dir` is
        // always false (even a symlink-to-a-directory), so it already,
        // correctly, falls into this same error path as any other
        // non-directory -- real `rmdir` on a symlink returns ENOTDIR
        // too, it never follows.
        return Err(format!("'{name}' is a file, not a directory -- use rm to remove files"));
    }
    let sub_table = load_dir_at(entries[slot].start_lba);
    if sub_table.iter().any(|e| e.used) {
        return Err(format!("'{name}' is not empty -- rmdir only removes empty directories"));
    }
    entries[slot] = DirEntry::empty(); // frees both the parent's slot and the (now-confirmed-empty) directory's own table sector for reuse
    save_dir_at(table_lba, &entries).map_err(|e| e.to_string())?;
    Ok(())
}

/// MILESTONE 63: real `ln -s TARGET LINKPATH` -- creates a NEW symlink
/// entry (never overwrites an existing name, same "already exists"
/// rule `make_dir_disk` already uses) whose "content" is `target`'s raw
/// text, stored via the same one-sector allocation a tiny file already
/// uses. `target` is stored completely verbatim/uninterpreted -- no
/// validation that it exists, no path normalization -- exactly like
/// real `symlink(2)`, which happily creates a dangling link.
fn symlink_disk(target: &str, linkpath: &str) -> Result<(), String> {
    if target.is_empty() {
        return Err("ln: target must not be empty".to_string());
    }
    if target.len() > MAX_SYMLINK_LEN {
        return Err(format!("ln: target too long -- max {MAX_SYMLINK_LEN} bytes"));
    }
    // MILESTONE 62-style rule: creating a new entry needs WRITE
    // permission on the PARENT directory. resolve_table below already
    // enforces search/execute on every ancestor via resolve_dir_lba.
    let dir_part = linkpath.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
    let (pmode, puid, pgid) = dir_meta_at(dir_part)?;
    if !entry_allows(pmode, puid, pgid, current_uid(), current_gid(), PERM_WRITE) {
        return Err("ln: permission denied (no write permission on parent directory)".to_string());
    }

    let (table_lba, name) = resolve_table(linkpath)?;
    let mut entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);
    if entries.iter().any(|e| e.used && e.name == target_name) {
        return Err(format!("'{name}' already exists"));
    }
    let slot = entries.iter().position(|e| !e.used).ok_or_else(|| "directory full (max 8 entries)".to_string())?;

    let occupied = collect_occupied(None);
    let start_pool =
        find_free_span(&occupied, 1).ok_or_else(|| "not enough free disk space (fragmented or full)".to_string())?;
    let mut sector = [0u8; SECTOR_SIZE];
    sector[..target.len()].copy_from_slice(target.as_bytes());
    crate::ata::write_sector(FILE_DATA_START_LBA + start_pool, &sector).map_err(|e| e.to_string())?;

    let now = crate::rtc::to_unix_timestamp(crate::rtc::now());
    entries[slot] = DirEntry {
        used: true,
        is_dir: false,
        name: target_name,
        len: target.len() as u16,
        start_lba: FILE_DATA_START_LBA + start_pool,
        sector_count: 1,
        mode: 0o777, // real unix convention -- a symlink's own mode is always shown as rwxrwxrwx and never actually consulted (see module doc comment)
        uid: current_uid(),
        gid: current_gid(),
        ctime: now,
        mtime: now,
        is_symlink: true,
    };
    save_dir_at(table_lba, &entries).map_err(|e| e.to_string())?;
    Ok(())
}

/// MILESTONE 63: real `readlink` -- returns the RAW target text of the
/// symlink named by `path` itself, with NO dereferencing (the one
/// place in this file that must see the literal link, not what it
/// points at).
fn readlink_disk(path: &str) -> Result<String, String> {
    let (table_lba, name) = resolve_table(path)?;
    let entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);
    let entry = entries.iter().find(|e| e.used && e.name == target_name).ok_or_else(|| format_no_such_file(name))?;
    if !entry.is_symlink {
        return Err(format!("'{name}' is not a symbolic link"));
    }
    read_symlink_target(entry)
}

fn format_no_such_file(name: &str) -> String {
    alloc::format!("no such file '{name}'")
}

/// MILESTONE 62: real `stat` -- root is handled specially (its
/// metadata lives in `ROOT_META_LBA`, not a `DirEntry`; see module doc
/// comment), everything else resolves through the same `resolve_table`
/// every other operation uses (so it enjoys the same real traversal
/// permission checks). Deliberately does NOT itself require any
/// read/write/execute permission on the TARGET -- matches real unix
/// `stat()`, which only needs search permission on the ancestor
/// directories (already enforced by `resolve_table`/`resolve_dir_lba`).
/// MILESTONE 63: deliberately `lstat`-shaped, not `stat`-shaped -- does
/// NOT dereference a symlink named by `path`, reporting the LINK's own
/// metadata instead (`is_symlink: true`, `len` = target text length).
/// `readlink` is the real, separate way to see what it points at; see
/// module doc comment for why.
fn stat_disk(path: &str) -> Result<FileMeta, String> {
    if path.is_empty() {
        let m = load_root_meta();
        return Ok(FileMeta {
            is_dir: true,
            is_symlink: false,
            len: 0,
            mode: m.mode,
            uid: m.uid,
            gid: m.gid,
            ctime: m.ctime,
            mtime: m.mtime,
        });
    }
    let (table_lba, name) = resolve_table(path)?;
    let entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);
    let e = entries.iter().find(|e| e.used && e.name == target_name).ok_or_else(|| format_no_such_file(name))?;
    Ok(FileMeta {
        is_dir: e.is_dir,
        is_symlink: e.is_symlink,
        len: e.len,
        mode: e.mode,
        uid: e.uid,
        gid: e.gid,
        ctime: e.ctime,
        mtime: e.mtime,
    })
}

/// MILESTONE 62: real `chmod` -- only the owner or root may change an
/// entry's mode, the same real unix rule chmod(2) enforces. MILESTONE
/// 63: deliberately does NOT dereference -- `chmod`-ing a symlink path
/// changes the LINK's own (practically-unused) mode bits, consistent
/// with `stat`'s lstat-shaped semantics above; see module doc comment.
fn chmod_disk(path: &str, mode: u16) -> Result<(), String> {
    let mode = mode & 0o777; // real mode_t is a 9-bit rwxrwxrwx field -- extra bits are silently masked, not an error
    if path.is_empty() {
        let mut meta = load_root_meta();
        if !(current_uid() == 0 || current_uid() == meta.uid) {
            return Err("chmod: '/' -- permission denied (not owner or root)".to_string());
        }
        meta.mode = mode;
        return save_root_meta(&meta).map_err(|e| e.to_string());
    }
    let (table_lba, name) = resolve_table(path)?;
    let mut entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);
    let slot = entries.iter().position(|e| e.used && e.name == target_name).ok_or_else(|| format_no_such_file(name))?;
    if !(current_uid() == 0 || current_uid() == entries[slot].uid) {
        return Err(format!("chmod: '{name}' -- permission denied (not owner or root)"));
    }
    entries[slot].mode = mode;
    save_dir_at(table_lba, &entries).map_err(|e| e.to_string())
}

/// MILESTONE 62: real `chown` -- root-only (the stricter modern-unix
/// rule; classic unix let an owner give a file away to someone else
/// too, but that's a real, deliberately disclosed choice this
/// milestone does NOT replicate -- root-only is simpler to reason
/// about and matches most real systems' actual default today).
/// MILESTONE 63: same lstat-shaped, non-dereferencing choice as chmod
/// above -- acts on the symlink entry itself.
fn chown_disk(path: &str, uid: u16, gid: u16) -> Result<(), String> {
    if current_uid() != 0 {
        return Err("chown: permission denied (root only)".to_string());
    }
    if path.is_empty() {
        let mut meta = load_root_meta();
        meta.uid = uid;
        meta.gid = gid;
        return save_root_meta(&meta).map_err(|e| e.to_string());
    }
    let (table_lba, name) = resolve_table(path)?;
    let mut entries = load_dir_at(table_lba);
    let target_name = name_bytes(name);
    let slot = entries.iter().position(|e| e.used && e.name == target_name).ok_or_else(|| format_no_such_file(name))?;
    entries[slot].uid = uid;
    entries[slot].gid = gid;
    save_dir_at(table_lba, &entries).map_err(|e| e.to_string())
}

fn list_disk(dir: Option<&str>) -> Result<Vec<(String, bool, u16)>, String> {
    let d = dir.unwrap_or("");
    let table_lba = resolve_dir_lba(d)?;
    // MILESTONE 62: real unix rule -- listing a directory needs READ
    // permission on that directory (distinct from the search/execute
    // permission resolve_dir_lba just checked to even get here).
    let (mode, uid, gid) = dir_meta_at(d)?;
    if !entry_allows(mode, uid, gid, current_uid(), current_gid(), PERM_READ) {
        return Err("ls: permission denied (no read permission on directory)".to_string());
    }
    Ok(load_dir_at(table_lba).iter().filter(|e| e.used).map(|e| (e.name_str(), e.is_dir, e.len)).collect())
}

/// MILESTONE 46: the real trait every backing store implements --
/// `DiskFs` (the pre-M46 disk logic above, unchanged) and `RamFs` (the
/// new in-memory second backing store, below) both satisfy this with the
/// EXACT same method signatures the pre-M46 free functions already had,
/// so `resolve_backend`'s dispatch is a genuine substitution, not a
/// reimplementation.
trait FileSystem {
    fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String>;
    fn read_file(&self, path: &str) -> Result<Vec<u8>, String>;
    fn delete_file(&self, path: &str) -> Result<(), String>;
    fn make_dir(&self, path: &str) -> Result<(), String>;
    fn remove_dir(&self, path: &str) -> Result<(), String>;
    fn list(&self, dir: Option<&str>) -> Result<Vec<(String, bool, u16)>, String>;
    // MILESTONE 62: real permission/ownership/timestamp metadata --
    // `RamFs` implements these as disclosed "not supported" errors (see
    // its own impl below), consistent with how it already handles
    // subdirectories.
    fn stat(&self, path: &str) -> Result<FileMeta, String>;
    fn chmod(&self, path: &str, mode: u16) -> Result<(), String>;
    fn chown(&self, path: &str, uid: u16, gid: u16) -> Result<(), String>;
    // MILESTONE 63: real symbolic links -- `RamFs` implements these as
    // disclosed "not supported" errors too, same pattern as above.
    fn symlink(&self, target: &str, linkpath: &str) -> Result<(), String>;
    fn readlink(&self, path: &str) -> Result<String, String>;
}

/// MILESTONE 62: the real metadata `stat()` returns -- mirrors real
/// unix `struct stat`'s most relevant fields for this kernel's own
/// deliberately minimal scope (no inode number/nlink/device/blocks
/// fields, since this filesystem doesn't have real inodes -- see the
/// module doc comment's hard-links reasoning for why).
/// MILESTONE 63: `is_symlink` added -- see `stat_disk`'s own doc
/// comment for why this reports the LINK's own info (lstat-shaped),
/// never a dereferenced target's.
#[derive(Debug, Clone, Copy)]
pub struct FileMeta {
    pub is_dir: bool,
    pub is_symlink: bool,
    pub len: u16,
    pub mode: u16,
    pub uid: u16,
    pub gid: u16,
    pub ctime: u32,
    pub mtime: u32,
}

/// The original (Milestone 18-32) ATA-disk-backed store, now expressed
/// as a `FileSystem` impl that does nothing but call straight through to
/// the `_disk`-suffixed functions above -- a zero-behavior-change wrapper,
/// not a rewrite.
struct DiskFs;

impl FileSystem for DiskFs {
    fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String> {
        write_file_disk(path, data)
    }
    fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        read_file_disk(path)
    }
    fn delete_file(&self, path: &str) -> Result<(), String> {
        delete_file_disk(path)
    }
    fn make_dir(&self, path: &str) -> Result<(), String> {
        make_dir_disk(path)
    }
    fn remove_dir(&self, path: &str) -> Result<(), String> {
        remove_dir_disk(path)
    }
    fn list(&self, dir: Option<&str>) -> Result<Vec<(String, bool, u16)>, String> {
        list_disk(dir)
    }
    fn stat(&self, path: &str) -> Result<FileMeta, String> {
        stat_disk(path)
    }
    fn chmod(&self, path: &str, mode: u16) -> Result<(), String> {
        chmod_disk(path, mode)
    }
    fn chown(&self, path: &str, uid: u16, gid: u16) -> Result<(), String> {
        chown_disk(path, uid, gid)
    }
    fn symlink(&self, target: &str, linkpath: &str) -> Result<(), String> {
        symlink_disk(target, linkpath)
    }
    fn readlink(&self, path: &str) -> Result<String, String> {
        readlink_disk(path)
    }
}

/// MILESTONE 46: the genuinely second, concrete backing store -- a flat,
/// in-memory filesystem with no ATA I/O anywhere in it. See the module
/// doc comment above for the full, disclosed list of what it does and
/// doesn't support.
struct RamFs {
    files: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl FileSystem for RamFs {
    fn write_file(&self, path: &str, data: &[u8]) -> Result<(), String> {
        if path.is_empty() {
            return Err("'ram' is a mount point (in-memory ramfs), not a file -- write to 'ram/NAME' instead".to_string());
        }
        if path.contains('/') {
            return Err(format!(
                "ramfs: '{path}' -- subdirectories not supported (flat namespace only), write directly under 'ram/'"
            ));
        }
        if data.len() > MAX_FILE_BYTES {
            return Err(format!(
                "file too large -- max {MAX_FILE_BYTES} bytes (ramfs reuses the disk backing store's own per-file cap)"
            ));
        }
        self.files.lock().insert(path.to_string(), data.to_vec());
        Ok(())
    }

    fn read_file(&self, path: &str) -> Result<Vec<u8>, String> {
        if path.is_empty() {
            return Err("'ram' is a mount point (in-memory ramfs), not a file".to_string());
        }
        self.files.lock().get(path).cloned().ok_or_else(|| format_no_such_file(path))
    }

    fn delete_file(&self, path: &str) -> Result<(), String> {
        if path.is_empty() {
            return Err("'ram' is a mount point (in-memory ramfs), not a file".to_string());
        }
        self.files.lock().remove(path).map(|_| ()).ok_or_else(|| format_no_such_file(path))
    }

    fn make_dir(&self, _path: &str) -> Result<(), String> {
        Err("ramfs: subdirectories not supported (flat namespace only)".to_string())
    }

    fn remove_dir(&self, _path: &str) -> Result<(), String> {
        Err("ramfs: subdirectories not supported (flat namespace only)".to_string())
    }

    fn list(&self, dir: Option<&str>) -> Result<Vec<(String, bool, u16)>, String> {
        if let Some(d) = dir {
            return Err(format!("ramfs: no such directory '{d}' (flat namespace only)"));
        }
        Ok(self.files.lock().iter().map(|(name, data)| (name.clone(), false, data.len() as u16)).collect())
    }

    // MILESTONE 62: real, disclosed scope cut -- ramfs stores no
    // permission/ownership/timestamp metadata at all (its `BTreeMap<
    // String, Vec<u8>>` backing has nowhere to put it), so these three
    // are refused with a clear error, exactly the same disclosed-cut
    // style `make_dir`/`remove_dir` above already established for
    // "ramfs doesn't support X".
    fn stat(&self, _path: &str) -> Result<FileMeta, String> {
        Err("ramfs: no permission/ownership/timestamp metadata (disk-backed files only this milestone)".to_string())
    }
    fn chmod(&self, _path: &str, _mode: u16) -> Result<(), String> {
        Err("ramfs: chmod not supported (no permission metadata on ramfs entries)".to_string())
    }
    fn chown(&self, _path: &str, _uid: u16, _gid: u16) -> Result<(), String> {
        Err("ramfs: chown not supported (no ownership metadata on ramfs entries)".to_string())
    }
    // MILESTONE 63: same disclosed-cut style as stat/chmod/chown above
    // -- ramfs has no symlink concept at all (its flat `BTreeMap<String,
    // Vec<u8>>` has nowhere to store a "this name is a link" flag or a
    // target string separately from real file content).
    fn symlink(&self, _target: &str, _linkpath: &str) -> Result<(), String> {
        Err("ramfs: symlinks not supported (disk-backed files only this milestone)".to_string())
    }
    fn readlink(&self, _path: &str) -> Result<String, String> {
        Err("ramfs: readlink not supported (no symlinks on ramfs entries)".to_string())
    }
}

static DISK: DiskFs = DiskFs;
static RAMFS: RamFs = RamFs { files: Mutex::new(BTreeMap::new()) };

const RAM_MOUNT: &str = "ram";
const RAM_PREFIX: &str = "ram/";

/// MILESTONE 46: the actual dispatch -- looks at ONLY the path's first
/// component (`"ram"` exactly, or a `"ram/"` prefix) to pick a backing
/// store, and hands back the REMAINDER of the path (already stripped of
/// the mount prefix) for that store to resolve on its own terms. Every
/// module-level function below calls this once and then talks to
/// whichever `FileSystem` it got back exactly as the pre-M46 free
/// functions talked to the disk directly.
fn resolve_backend(path: &str) -> (&'static dyn FileSystem, &str) {
    if path == RAM_MOUNT {
        (&RAMFS, "")
    } else if let Some(rest) = path.strip_prefix(RAM_PREFIX) {
        (&RAMFS, rest)
    } else {
        (&DISK, path)
    }
}

pub fn make_dir(path: &str) -> Result<(), String> {
    let (backend, sub) = resolve_backend(path);
    backend.make_dir(sub)
}

pub fn write_file(path: &str, data: &[u8]) -> Result<(), String> {
    let (backend, sub) = resolve_backend(path);
    backend.write_file(sub, data)
}

pub fn read_file(path: &str) -> Result<Vec<u8>, String> {
    let (backend, sub) = resolve_backend(path);
    backend.read_file(sub)
}

pub fn delete_file(path: &str) -> Result<(), String> {
    let (backend, sub) = resolve_backend(path);
    backend.delete_file(sub)
}

pub fn remove_dir(path: &str) -> Result<(), String> {
    let (backend, sub) = resolve_backend(path);
    backend.remove_dir(sub)
}

// MILESTONE 62: real permission/ownership/timestamp inspection and
// modification, dispatched through the exact same resolve_backend
// every other operation already goes through.
pub fn stat(path: &str) -> Result<FileMeta, String> {
    let (backend, sub) = resolve_backend(path);
    backend.stat(sub)
}

pub fn chmod(path: &str, mode: u16) -> Result<(), String> {
    let (backend, sub) = resolve_backend(path);
    backend.chmod(sub, mode)
}

pub fn chown(path: &str, uid: u16, gid: u16) -> Result<(), String> {
    let (backend, sub) = resolve_backend(path);
    backend.chown(sub, uid, gid)
}

// MILESTONE 63: real symbolic links, dispatched the same way -- only
// `linkpath`/`path` (where the link itself lives, or is looked up) go
// through resolve_backend; `target` (a symlink's stored text) is passed
// through completely verbatim and is resolved purely within whichever
// single backend the link itself lives on (see module doc comment for
// why a symlink can't point across the disk/ram boundary).
pub fn symlink(target: &str, linkpath: &str) -> Result<(), String> {
    let (backend, sub) = resolve_backend(linkpath);
    backend.symlink(target, sub)
}

pub fn readlink(path: &str) -> Result<String, String> {
    let (backend, sub) = resolve_backend(path);
    backend.readlink(sub)
}

pub fn list(dir: Option<&str>) -> Result<Vec<(String, bool, u16)>, String> {
    match dir {
        // Unchanged from pre-M46: no argument means the disk's own
        // root, exactly as it always has -- the ramfs mount does NOT
        // inject a synthetic entry into this listing (a disclosed
        // scope-cut, see module doc comment), so this can never regress
        // any existing self-test/shell flow that depends on root's
        // exact contents.
        None => DISK.list(None),
        Some(d) => {
            let (backend, sub) = resolve_backend(d);
            backend.list(if sub.is_empty() { None } else { Some(sub) })
        }
    }
}

/// MILESTONE 46: real, boot-time, non-interactive proof that the SECOND
/// backing store (`RamFs`) is genuinely reachable through the identical
/// `fs::write_file`/`fs::read_file`/`fs::list` surface `self_test_disk_write`
/// above already exercises against the disk -- and that it's a real,
/// separate store, not just the disk under a different-looking name
/// (the isolation check below proves the ram-mounted file does NOT show
/// up in the disk's own root listing).
pub fn self_test_ramfs() {
    // Deliberately a DIFFERENT literal name than self_test_disk_write()'s
    // "selftestwrite" -- an earlier version of this self-test reused that
    // same name and its isolation check below reported a false FAILURE:
    // the disk root legitimately already contained "selftestwrite" from
    // self_test_disk_write() running first, nothing to do with routing.
    // Real bug, found via the actual serial log, fixed by giving the two
    // self-tests disjoint names rather than by weakening the check.
    const NAME: &str = "ramfsselftest";
    const RAM_PATH: &str = "ram/ramfsselftest";
    const CONTENT: &[u8] = b"ramfs self-test write/read roundtrip";

    match write_file(RAM_PATH, CONTENT).and_then(|_| read_file(RAM_PATH)) {
        Ok(bytes) if bytes == CONTENT => {
            let _ = writeln!(
                serial(),
                "fs self-test: ramfs write+read roundtrip OK -- real bytes matched, routed through the same fs::write_file/read_file surface as the disk path"
            );
        }
        Ok(bytes) => {
            let _ = writeln!(
                serial(),
                "fs self-test: ramfs write+read MISMATCH -- wrote {} bytes, read back {} bytes",
                CONTENT.len(),
                bytes.len()
            );
            return;
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: ramfs write+read FAILED -- {e}");
            return;
        }
    }

    match list(Some("ram")) {
        Ok(entries) if entries.iter().any(|(n, is_dir, len)| n == NAME && !is_dir && *len as usize == CONTENT.len()) => {
            let _ = writeln!(serial(), "fs self-test: ramfs list() OK -- '{NAME}' present with the correct length");
        }
        Ok(entries) => {
            let _ = writeln!(serial(), "fs self-test: ramfs list() MISMATCH -- got {entries:?}");
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: ramfs list() FAILED -- {e}");
        }
    }

    match list(None) {
        Ok(entries) if !entries.iter().any(|(n, _, _)| n == NAME) => {
            let _ = writeln!(
                serial(),
                "fs self-test: ramfs isolation OK -- disk root listing does NOT contain '{NAME}' (two real, separate backing stores)"
            );
        }
        Ok(entries) => {
            let _ = writeln!(
                serial(),
                "fs self-test: ramfs isolation FAILED -- disk root listing unexpectedly contains '{NAME}': {entries:?}"
            );
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: disk list() FAILED (during isolation check) -- {e}");
        }
    }
}

/// MILESTONE 62: real, boot-time, non-interactive proof that the new
/// on-disk permissions/ownership/timestamps actually work -- real
/// create-time defaults (mode/owner/timestamp), chmod (owner-or-root)
/// and chown (root-only) real enforcement, granular read-vs-write
/// denial by mode bit, the real unix "create/delete need write on the
/// PARENT directory, not the target" rule, and directory search(x)-bit
/// traversal gating that blocks reaching a file even when the file's
/// OWN permission bits would otherwise allow it. `CURRENT_ID` is reset
/// to (0, 0) (root) at both the start and the end, so this self-test
/// never leaves a later self-test or the interactive shell running
/// under the (42, 42) test identity it exercises in between.
/// POST-MILESTONE-65 FIX: best-effort, idempotent teardown of
/// `self_test_permissions()`'s own on-disk fixtures. Removes deepest
/// entries first (a non-empty directory's own `remove_dir` call would
/// otherwise fail) and covers every path the test could possibly have
/// created, including two (`permtestdir/blocked`, `permtestdir/sub`)
/// the test itself asserts should NEVER successfully get created -- a
/// real bug in some future edit of the test body could still leave one
/// behind, and this stays correct either way since every call's Result
/// is deliberately ignored. Caller is responsible for already being
/// root (uid 0) -- see this function's call site.
fn teardown_permtestdir_fixtures() {
    let _ = delete_file("permtestdir/locked/inner");
    let _ = remove_dir("permtestdir/locked");
    let _ = delete_file("permtestdir/blocked");
    let _ = remove_dir("permtestdir/sub");
    let _ = delete_file("permtestdir/g1");
    let _ = delete_file("permtestdir/f1");
    let _ = remove_dir("permtestdir");
}

/// POST-MILESTONE-65 FIX: same idempotent-teardown discipline as
/// `teardown_permtestdir_fixtures()` above, for `self_test_symlinks()`'s
/// own fixtures -- including `abslink`, the one fixture this test
/// creates OUTSIDE `symtestdir` itself (a root-level entry, so it needs
/// its own explicit cleanup rather than being covered by `symtestdir`'s
/// own removal).
fn teardown_symtestdir_fixtures() {
    let _ = delete_file("symtestdir/realsub/inner.txt");
    let _ = remove_dir("symtestdir/realsub");
    let _ = delete_file("symtestdir/lockedreal/secret.txt");
    let _ = remove_dir("symtestdir/lockedreal");
    let _ = delete_file("symtestdir/sublink");
    let _ = delete_file("symtestdir/link2");
    let _ = delete_file("symtestdir/lockedlink");
    let _ = delete_file("symtestdir/link_to_real");
    let _ = delete_file("symtestdir/link_to_link");
    let _ = delete_file("symtestdir/broken");
    let _ = delete_file("symtestdir/loop_a");
    let _ = delete_file("symtestdir/loop_b");
    let _ = delete_file("symtestdir/emptylink");
    let _ = delete_file("symtestdir/real.txt");
    let _ = remove_dir("symtestdir");
    let _ = delete_file("abslink");
}

pub fn self_test_permissions() {
    let mut ok = true;
    set_current_identity(0, 0);

    // POST-MILESTONE-65 FIX: this self-test's own fixtures
    // (`permtestdir` and everything under it) are created with
    // create-exclusive calls (`make_dir`/`write_file` for a NEW name)
    // that fail once that name already exists -- which it does, on any
    // boot after the first, against a disk this self-test already ran
    // on once. Real, idempotent fix: unconditionally tear down every
    // fixture this test could possibly have left behind BEFORE creating
    // anything, ignoring every removal's Result (on a genuinely fresh
    // disk none of these paths exist yet -- "no such file" is the
    // expected, harmless case here, not a real error). Runs as root
    // (identity already set above), so stale permission bits a PRIOR
    // run's own chmod calls left behind (e.g. `permtestdir/locked` at
    // 0700) can never block cleanup -- root bypasses every check.
    teardown_permtestdir_fixtures();

    macro_rules! check {
        ($cond:expr, $label:expr) => {
            if $cond {
                let _ = writeln!(serial(), "fs self-test: permissions {}=PASS", $label);
            } else {
                let _ = writeln!(serial(), "fs self-test: permissions {}=FAIL", $label);
                ok = false;
            }
        };
    }

    // Real create-time defaults: mkdir as root.
    check!(make_dir("permtestdir").is_ok(), "mkdir_root");
    match stat("permtestdir") {
        Ok(m) => {
            check!(m.is_dir, "mkdir_is_dir");
            check!(m.mode == DEFAULT_DIR_MODE, "mkdir_default_mode_0755");
            check!(m.uid == 0 && m.gid == 0, "mkdir_owner_root");
            check!(m.ctime == m.mtime, "mkdir_ctime_eq_mtime_fresh");
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions stat_after_mkdir=FAIL ({e})");
            ok = false;
        }
    }

    // Real create-time defaults: write a new file as root.
    check!(write_file("permtestdir/f1", b"hello").is_ok(), "write_create_root");
    let mut f1_ctime = 0u32;
    match stat("permtestdir/f1") {
        Ok(m) => {
            check!(!m.is_dir && m.len == 5, "f1_is_file_len5");
            check!(m.mode == DEFAULT_FILE_MODE, "f1_default_mode_0644");
            check!(m.uid == 0 && m.gid == 0, "f1_owner_root");
            f1_ctime = m.ctime;
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions stat_after_write=FAIL ({e})");
            ok = false;
        }
    }

    // Overwrite preserves mode/uid/gid/ctime, only bumps mtime.
    check!(write_file("permtestdir/f1", b"HELLO!").is_ok(), "write_overwrite_root");
    match stat("permtestdir/f1") {
        Ok(m) => {
            check!(m.mode == DEFAULT_FILE_MODE && m.uid == 0 && m.gid == 0, "f1_overwrite_preserves_owner_mode");
            check!(m.ctime == f1_ctime, "f1_overwrite_preserves_ctime");
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions stat_after_overwrite=FAIL ({e})");
            ok = false;
        }
    }

    // chmod as root -- narrow to owner-only rw.
    check!(chmod("permtestdir/f1", 0o600).is_ok(), "chmod_root_narrows_to_0600");
    match stat("permtestdir/f1") {
        Ok(m) => check!(m.mode == 0o600, "f1_mode_now_0600"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions stat_after_chmod=FAIL ({e})");
            ok = false;
        }
    }

    // Switch to a non-root, non-owner identity -- 0600 must deny both
    // read and write to this file entirely.
    set_current_identity(42, 42);
    check!(read_file("permtestdir/f1").is_err(), "uid42_read_denied_mode0600");
    check!(write_file("permtestdir/f1", b"x").is_err(), "uid42_write_denied_mode0600");

    // Root re-opens it to world-readable-only (0644) -- uid42 can now
    // read but still not write, proving read/write are checked
    // separately, not as one combined "any access" bit.
    set_current_identity(0, 0);
    check!(chmod("permtestdir/f1", 0o644).is_ok(), "chmod_root_reopens_to_0644");
    set_current_identity(42, 42);
    match read_file("permtestdir/f1") {
        Ok(data) => check!(data == b"HELLO!", "uid42_read_allowed_mode0644"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions uid42_read_allowed_mode0644=FAIL ({e})");
            ok = false;
        }
    }
    check!(write_file("permtestdir/f1", b"y").is_err(), "uid42_write_still_denied_mode0644");

    // Real "create/delete needs WRITE on the PARENT, not the target"
    // rule: permtestdir is 0755 (default) -- "other" (uid42, not owner,
    // not group 0) has no write bit, so uid42 cannot create a new file
    // directly under it.
    check!(write_file("permtestdir/blocked", b"z").is_err(), "uid42_create_denied_parent_0755");
    check!(make_dir("permtestdir/sub").is_err(), "uid42_mkdir_denied_parent_0755");

    // Root opens permtestdir to world-writable -- uid42 can now create
    // AND delete inside it, and the new entry it creates is genuinely
    // OWNED by uid42 (real unix create-time-uid semantics), not root.
    set_current_identity(0, 0);
    check!(chmod("permtestdir", 0o777).is_ok(), "chmod_root_permtestdir_0777");
    set_current_identity(42, 42);
    check!(write_file("permtestdir/g1", b"made-by-42").is_ok(), "uid42_create_allowed_parent_0777");
    match stat("permtestdir/g1") {
        Ok(m) => check!(m.uid == 42 && m.gid == 42, "g1_owned_by_creator_uid42"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions stat_g1=FAIL ({e})");
            ok = false;
        }
    }

    // Owner (uid42) may chmod its own file even though it isn't root.
    check!(chmod("permtestdir/g1", 0o640).is_ok(), "uid42_chmod_own_file_allowed");
    // But chown is root-only -- even the genuine owner is refused.
    check!(chown("permtestdir/g1", 7, 7).is_err(), "uid42_chown_denied_root_only");

    // Root CAN chown it, to anyone.
    set_current_identity(0, 0);
    check!(chown("permtestdir/g1", 7, 7).is_ok(), "root_chown_g1_to_7_7");
    match stat("permtestdir/g1") {
        Ok(m) => check!(m.uid == 7 && m.gid == 7, "g1_now_owned_by_7_7"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions stat_g1_after_chown=FAIL ({e})");
            ok = false;
        }
    }

    // uid42 deletes its own-created g1 (parent permtestdir is still
    // 0777, so the "delete needs parent write" rule allows it even
    // though g1 itself is no longer owned by uid42 -- real unix rm
    // semantics, proven directly).
    set_current_identity(42, 42);
    check!(delete_file("permtestdir/g1").is_ok(), "uid42_delete_via_parent_write_0777");

    // Real directory search(x)-bit traversal gating: make a subdir with
    // a world-readable FILE inside it, then strip the subdir's own x
    // bit for non-owners -- uid42 must be unable to reach the file at
    // all, even though the file's own mode would allow it, proving
    // traversal permission is checked independently of the leaf's
    // permission bits.
    set_current_identity(0, 0);
    check!(make_dir("permtestdir/locked").is_ok(), "mkdir_locked_subdir");
    check!(write_file("permtestdir/locked/inner", b"secret").is_ok(), "write_inner_file");
    check!(chmod("permtestdir/locked/inner", 0o666).is_ok(), "chmod_inner_world_rw");
    check!(chmod("permtestdir/locked", 0o700).is_ok(), "chmod_locked_owner_only_0700");
    set_current_identity(42, 42);
    check!(read_file("permtestdir/locked/inner").is_err(), "uid42_traversal_denied_despite_file_0666");
    check!(list(Some("permtestdir/locked")).is_err(), "uid42_ls_traversal_denied");

    // Root's own metadata: chmod/chown "/" itself. Non-root is refused
    // both; root can do both, restored to the real default afterward.
    set_current_identity(0, 0);
    let root_before = stat("");
    check!(root_before.is_ok(), "stat_root");
    set_current_identity(42, 42);
    check!(chmod("", 0o700).is_err(), "uid42_chmod_root_denied");
    check!(chown("", 42, 42).is_err(), "uid42_chown_root_denied");
    set_current_identity(0, 0);
    check!(chown("", 5, 5).is_ok(), "root_chown_root_to_5_5");
    match stat("") {
        Ok(m) => check!(m.uid == 5 && m.gid == 5, "root_now_owned_by_5_5"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: permissions stat_root_after_chown=FAIL ({e})");
            ok = false;
        }
    }
    // Restore root to its real default owner/mode so nothing downstream
    // (later self-tests, the interactive shell) inherits a non-default
    // root ownership from this self-test having run.
    check!(chown("", 0, 0).is_ok() && chmod("", DEFAULT_DIR_MODE).is_ok(), "root_restored_to_defaults");

    // POST-MILESTONE-65 FIX: tear this test's own fixtures back down
    // once done, not just at the next boot's start -- root's own
    // directory table is a real, small, ENFORCED 8-entry capacity
    // (MAX_ENTRIES), shared by every self-test that ever writes a
    // top-level name to disk in the SAME boot (`selftestwrite`,
    // `altentry`, `mmapf`, the ring-3 `stdiotest.elf` program's own
    // `stdiotest_a`/`stdiotest_b`, and this test's own `permtestdir`).
    // Leaving `permtestdir` (and everything under it) sitting there for
    // the rest of the boot after this test is already done with it was
    // real, avoidable pressure on that shared capacity -- directly
    // responsible (confirmed against a real disk image) for
    // `loader::self_test_execargv()`'s own later `write_file()` for
    // `argvtarget` failing with "directory full" even on a single boot,
    // never mind a repeated one. Freeing it back up here, identity
    // already root, ignoring every removal's Result (identical
    // reasoning to `teardown_permtestdir_fixtures()`'s own call at this
    // function's start).
    teardown_permtestdir_fixtures();

    set_current_identity(0, 0); // leave the kernel-global identity as root, whatever happened above
    let _ = writeln!(serial(), "fs self-test: permissions OVERALL={}", if ok { "PASS" } else { "FAIL" });
}

/// MILESTONE 63: real, boot-time, non-interactive proof that symbolic
/// links actually work end-to-end -- creation (`ln -s`-equivalent
/// `symlink()`), raw inspection (`readlink`, `stat`'s lstat-shaped
/// non-following behavior), real dereferencing on `read_file` (single
/// hop, multi-hop chains, and traversal THROUGH a symlinked directory
/// component), real bounded-depth ELOOP and broken-link errors, `rm`
/// removing the link without touching its target, the disclosed
/// write-through-symlink refusal, `rmdir` correctly refusing a
/// symlink-to-directory (ENOTDIR, matching real unix), chmod/chown
/// acting on the link itself, and -- the most important correctness
/// property -- that a symlink can NEVER be used to bypass a real
/// permission check on the directory it ultimately points into.
/// `CURRENT_ID` is reset to (0, 0) (root) at both the start and the end,
/// same discipline as `self_test_permissions` above.
pub fn self_test_symlinks() {
    let mut ok = true;
    set_current_identity(0, 0);

    // POST-MILESTONE-65 FIX: same real non-idempotency this milestone's
    // own writeup disclosed for `self_test_permissions()` above, and the
    // same fix -- tear down every fixture this test (`symtestdir`,
    // `abslink`) could have left behind from a PRIOR boot on this same
    // disk before creating anything, ignoring every removal's Result.
    teardown_symtestdir_fixtures();

    macro_rules! check {
        ($cond:expr, $label:expr) => {
            if $cond {
                let _ = writeln!(serial(), "fs self-test: symlinks {}=PASS", $label);
            } else {
                let _ = writeln!(serial(), "fs self-test: symlinks {}=FAIL", $label);
                ok = false;
            }
        };
    }

    check!(make_dir("symtestdir").is_ok(), "mkdir_symtestdir");
    const CONTENT: &[u8] = b"hello via symlink";
    check!(write_file("symtestdir/real.txt", CONTENT).is_ok(), "write_real_txt");

    // Basic create + real dereference on read + raw (non-dereferencing)
    // inspection.
    check!(symlink("real.txt", "symtestdir/link_to_real").is_ok(), "symlink_create_relative");
    match read_file("symtestdir/link_to_real") {
        Ok(data) => check!(data == CONTENT, "read_through_symlink_single_hop"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks read_through_symlink_single_hop=FAIL ({e})");
            ok = false;
        }
    }
    match stat("symtestdir/link_to_real") {
        Ok(m) => {
            check!(m.is_symlink && !m.is_dir, "stat_link_to_real_is_symlink_lstat_shaped");
            check!(m.len as usize == "real.txt".len(), "stat_link_to_real_len_is_target_text_len");
        }
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks stat_link_to_real=FAIL ({e})");
            ok = false;
        }
    }
    match readlink("symtestdir/link_to_real") {
        Ok(t) => check!(t == "real.txt", "readlink_link_to_real_raw_target"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks readlink_link_to_real=FAIL ({e})");
            ok = false;
        }
    }

    // A 2-hop chain (link_to_link -> link_to_real -> real.txt) resolves
    // through BOTH hops on read, then is deleted to free the slot back
    // (symtestdir's 8-entry cap).
    check!(symlink("link_to_real", "symtestdir/link_to_link").is_ok(), "symlink_create_chain");
    match read_file("symtestdir/link_to_link") {
        Ok(data) => check!(data == CONTENT, "read_through_symlink_chain_two_hops"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks read_through_symlink_chain_two_hops=FAIL ({e})");
            ok = false;
        }
    }
    check!(delete_file("symtestdir/link_to_link").is_ok(), "cleanup_link_to_link");

    // A broken link (target never existed) is a real, disclosed error,
    // not a panic or a silent empty read.
    check!(symlink("doesnotexist.txt", "symtestdir/broken").is_ok(), "symlink_create_broken");
    check!(read_file("symtestdir/broken").is_err(), "read_broken_symlink_fails_cleanly");
    check!(delete_file("symtestdir/broken").is_ok(), "cleanup_broken");

    // A genuine two-node symlink cycle hits the real, bounded ELOOP
    // guard rather than recursing forever.
    check!(symlink("loop_b", "symtestdir/loop_a").is_ok(), "symlink_create_loop_a");
    check!(symlink("loop_a", "symtestdir/loop_b").is_ok(), "symlink_create_loop_b");
    check!(read_file("symtestdir/loop_a").is_err(), "read_symlink_cycle_hits_eloop_guard");
    check!(delete_file("symtestdir/loop_a").is_ok() && delete_file("symtestdir/loop_b").is_ok(), "cleanup_loop");

    // Traversal THROUGH a symlinked directory component -- the
    // Milestone-32-style generalization this milestone extends.
    check!(make_dir("symtestdir/realsub").is_ok(), "mkdir_realsub");
    check!(write_file("symtestdir/realsub/inner.txt", b"abc").is_ok(), "write_inner_txt");
    check!(symlink("realsub", "symtestdir/sublink").is_ok(), "symlink_create_dir_link");
    match read_file("symtestdir/sublink/inner.txt") {
        Ok(data) => check!(data == b"abc", "read_through_symlinked_directory_component"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks read_through_symlinked_directory_component=FAIL ({e})");
            ok = false;
        }
    }
    match list(Some("symtestdir/sublink")) {
        Ok(entries) => check!(
            entries.iter().any(|(n, is_dir, _)| n == "inner.txt" && !is_dir),
            "list_through_symlinked_directory_component"
        ),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks list_through_symlinked_directory_component=FAIL ({e})");
            ok = false;
        }
    }
    // rmdir on a symlink-to-directory correctly refuses (ENOTDIR, real
    // unix behavior -- rmdir never follows) rather than either removing
    // the link or reaching into the real directory.
    check!(remove_dir("symtestdir/sublink").is_err(), "rmdir_symlink_to_dir_refused_enotdir");

    // Writing to an EXISTING symlink is a real, disclosed refusal (see
    // module doc comment) -- and chmod/chown act on the LINK itself,
    // never the target (proven directly: the target's own mode is
    // untouched).
    check!(symlink("real.txt", "symtestdir/link2").is_ok(), "symlink_create_link2");
    check!(write_file("symtestdir/link2", b"x").is_err(), "write_through_existing_symlink_refused");
    check!(chmod("symtestdir/link2", 0o600).is_ok(), "chmod_link2_acts_on_link_itself");
    match stat("symtestdir/link2") {
        Ok(m) => check!(m.mode == 0o600, "link2_mode_now_0600"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks stat_link2_after_chmod=FAIL ({e})");
            ok = false;
        }
    }
    match stat("symtestdir/real.txt") {
        Ok(m) => check!(m.mode == DEFAULT_FILE_MODE, "real_txt_mode_unaffected_by_chmod_on_link"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks stat_real_txt_after_link2_chmod=FAIL ({e})");
            ok = false;
        }
    }

    // rm removes the LINK, never the target -- proven directly: the
    // link is gone (stat fails) but the target's real content survives
    // completely untouched.
    check!(delete_file("symtestdir/link_to_real").is_ok(), "rm_link_to_real");
    check!(stat("symtestdir/link_to_real").is_err(), "link_to_real_gone_after_rm");
    match read_file("symtestdir/real.txt") {
        Ok(data) => check!(data == CONTENT, "real_txt_survives_rm_of_its_link"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks real_txt_survives_rm_of_its_link=FAIL ({e})");
            ok = false;
        }
    }

    // The real correctness property: a symlink can NEVER be used to
    // bypass a permission check on the real directory it points into --
    // a non-root, non-owner identity is denied reaching a file behind a
    // symlink to a directory it isn't allowed to search, exactly as if
    // it had typed the real path directly (proven against
    // self_test_permissions' own "search-bit traversal gating" case,
    // now routed through a symlink instead of a plain path).
    check!(make_dir("symtestdir/lockedreal").is_ok(), "mkdir_lockedreal");
    check!(write_file("symtestdir/lockedreal/secret.txt", b"topsecret").is_ok(), "write_secret_txt");
    check!(chmod("symtestdir/lockedreal", 0o700).is_ok(), "chmod_lockedreal_owner_only");
    check!(symlink("lockedreal", "symtestdir/lockedlink").is_ok(), "symlink_create_lockedlink");
    set_current_identity(42, 42);
    check!(
        read_file("symtestdir/lockedlink/secret.txt").is_err(),
        "uid42_denied_through_symlink_no_privilege_escalation"
    );
    set_current_identity(0, 0);

    // An empty target is refused outright (never silently stored).
    check!(symlink("", "symtestdir/emptylink").is_err(), "symlink_empty_target_refused");

    // Absolute targets resolve from root, not from the link's own
    // directory.
    check!(symlink("/symtestdir/real.txt", "abslink").is_ok(), "symlink_create_absolute");
    match read_file("abslink") {
        Ok(data) => check!(data == CONTENT, "read_through_absolute_symlink"),
        Err(e) => {
            let _ = writeln!(serial(), "fs self-test: symlinks read_through_absolute_symlink=FAIL ({e})");
            ok = false;
        }
    }

    // POST-MILESTONE-65 FIX: same real, shared-root-capacity reasoning
    // as `self_test_permissions()`'s own end-of-test teardown call above
    // -- free `symtestdir` and `abslink`'s root-level slots back up now
    // that this test is done with them, rather than leaving them
    // occupied for the rest of the boot.
    teardown_symtestdir_fixtures();

    set_current_identity(0, 0); // leave the kernel-global identity as root, whatever happened above
    let _ = writeln!(serial(), "fs self-test: symlinks OVERALL={}", if ok { "PASS" } else { "FAIL" });
}
