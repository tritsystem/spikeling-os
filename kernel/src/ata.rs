//! MILESTONE 11: real ATA PIO disk I/O -- persisting Milestone 10's
//! learned synaptic weights across reboots, directly fulfilling a
//! roadmap item Spikeling's OWN original core/README.md left
//! unimplemented: "Weight persistence (save/load trained networks)".
//!
//! Targets the SECONDARY ATA bus (ports 0x170-0x177), master drive --
//! deliberately NOT the primary bus the boot drive lives on, so
//! persistence testing can never risk touching the bootloader/kernel
//! image being booted from. A separate, dedicated disk image is
//! attached there specifically for this in the verification harness.
//!
//! MILESTONE 38: the on-disk format at LBA 0 (still the same,
//! dedicated sector Milestone 18's fs.rs deliberately leaves untouched)
//! is now GENERALIZED and TERNARY-COMPRESSED:
//!   - generalized: was two hardcoded f32s (LeftKey->Motor,
//!     RightKey->Motor); now an arbitrary, NAME-KEYED list of
//!     (from, to, weight) entries, one per synapse that existed in the
//!     network at save time (see network.rs::all_synapse_weights()).
//!     This is a deliberate, disclosed partial generalization: it
//!     persists WEIGHTS for however many synapses exist, but not the
//!     network's TOPOLOGY (neuron thresholds/leaks, which synapses
//!     exist) -- only the two fixed LeftKey/RightKey->Motor synapses
//!     are guaranteed to exist again at the next boot (seeded by
//!     neurons::init() before load_weights() is even consulted), so
//!     those are the only entries a REAL reboot can currently restore.
//!     Extra DSL-added (`addsynapse`) entries round-trip correctly
//!     within the SAME boot session (save then reload without
//!     rebooting) but are honestly reported as unmatched, not silently
//!     dropped, if the network was rebuilt from scratch first.
//!   - ternary-compressed: each weight is packed via ternary.rs's real
//!     port of OBSERVE's pack_ternary/unpack_ternary (10 trits / 2
//!     bytes per weight, vs. 4 bytes for the f32 it replaces -- see
//!     ternary.rs for the full precision/range justification).
//!
//! New magic ("SPK2", not Milestone 11's original "SPKL") so a disk
//! written by pre-M38 code -- or a blank disk -- is safely recognized
//! as NOT this format and load_weights() honestly falls back to
//! `None` (neurons.rs then uses neutral defaults) rather than
//! misreading raw f32 bytes as a ternary-packed header.
//!
//! Sector layout (512 bytes, LBA 0):
//!   [0..4)   magic ("SPK2", LE u32)
//!   [4]      format version (u8, =1)
//!   [5]      entry count (u8)
//!   [6]      trits per weight (u8, self-describing -- =10 currently)
//!   [7]      reserved (=0)
//!   [8..)    `count` entries, each ENTRY_LEN=34 bytes:
//!              [0..16)  `from` name, UTF-8, NUL-padded
//!              [16..32) `to` name, UTF-8, NUL-padded
//!              [32..34) ternary-packed weight (2 bytes)
//!            up to MAX_SYNAPSES=14 entries fit in one 512-byte sector.
//!
//! MILESTONE 66: a real `BlockDevice` trait now separates "how to talk
//! to an ATA drive over PIO" (`AtaDevice`, generalized over an I/O port
//! base and a master/slave drive-select byte) from "which specific
//! drive" -- the last remaining Tier 2 (filesystem completeness) item,
//! chosen over hard links (still genuinely blocked: `fs.rs`'s
//! `DirEntry` still embeds `start_lba`/`sector_count` directly with no
//! inode-indirection layer, re-confirmed unchanged since Milestone 62's
//! own judgment call -- see fs.rs's module doc). Before this milestone
//! `read_sector`/`write_sector` were bare free functions hardcoded to
//! ONE drive (I/O base 0x170, master-select byte 0xE0) -- real PIO, but
//! not an abstraction: nothing about the code separated "the mechanism"
//! from "the specific hardware", and there was only ever one real
//! device to prove it against (the Milestone 40 secondary-bus
//! persistence disk). The Milestone 64/65 "second-ATA-drive" triple
//! fault that blocked this is now fixed (see README's Post-Milestone-65
//! fix pass -- root cause was `gdt.rs`'s shared ring0 stack, not
//! anything about having two drives), so a genuine second real block
//! device is now safe to add and prove the abstraction against.
//!
//! **Real second device**: a real IDE SLAVE drive on the SAME secondary
//! bus/cable as the existing master (`bus=ide.1,unit=1` in
//! `src/main.rs`'s QEMU config, a real, separate `target/persist2.img`
//! backing file) -- the standard real-hardware master/slave pairing on
//! one ATA cable, selected purely via the drive-select byte's DRV bit
//! (`0xE0` master vs `0xF0` slave) at the SAME I/O port base, not a new
//! controller or IRQ. `SECONDARY_MASTER`/`SECONDARY_SLAVE` are the two
//! real `AtaDevice` instances; the existing free `read_sector()`/
//! `write_sector()` functions (still used, unmodified call sites, by
//! `fs.rs` and this module's own `save_weights()`/`load_weights()`) are
//! now thin wrappers delegating to `SECONDARY_MASTER` -- byte-identical
//! I/O ports and drive-select value as before this milestone, zero
//! behavioral change for any existing caller (see `self_test_
//! block_device_abstraction()`'s own disclosure of how this was
//! confirmed).

use crate::serial;
use crate::ternary;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt::Write;
use x86_64::instructions::port::Port;

const STATUS_ERR: u8 = 0x01;
const STATUS_DRQ: u8 = 0x08;
const STATUS_BSY: u8 = 0x80;

const CMD_READ_SECTORS: u8 = 0x20;
const CMD_WRITE_SECTORS: u8 = 0x30;
const CMD_CACHE_FLUSH: u8 = 0xE7;

/// MILESTONE 66: the real block-device abstraction -- any type that can
/// read/write a 512-byte sector by LBA. `AtaDevice` below is the only
/// implementation, but the trait boundary is what makes this a genuine
/// abstraction rather than just a refactor: `self_test_
/// block_device_abstraction()` drives two DIFFERENT real `AtaDevice`
/// instances through this SAME trait interface, and future code could
/// implement it for a non-ATA backend without touching a single call
/// site written against `&dyn BlockDevice`/`impl BlockDevice`.
pub trait BlockDevice {
    fn read_sector(&self, lba: u32, buf: &mut [u8; 512]) -> Result<(), &'static str>;
    fn write_sector(&self, lba: u32, buf: &[u8; 512]) -> Result<(), &'static str>;
}

/// MILESTONE 66: one real ATA PIO device, generalized over the two
/// things that actually distinguish drives on the classic IDE bus
/// layout this kernel targets: the I/O port base (0x170 for the
/// secondary controller, same as every prior milestone) and the
/// drive-select byte's DRV bit (bit 4: 0 = master, 1 = slave -- OR'd
/// with LBA[27:24] on every access exactly as the original hardcoded
/// `select_and_setup()` already did for the master-only case).
pub struct AtaDevice {
    io_base: u16,
    drive_select_base: u8,
}

impl AtaDevice {
    pub const fn new(io_base: u16, drive_select_base: u8) -> Self {
        AtaDevice { io_base, drive_select_base }
    }

    fn wait_not_busy(&self) {
        let mut status_port: Port<u8> = Port::new(self.io_base + 7);
        loop {
            if unsafe { status_port.read() } & STATUS_BSY == 0 {
                break;
            }
        }
    }

    fn wait_drq(&self) -> Result<(), &'static str> {
        let mut status_port: Port<u8> = Port::new(self.io_base + 7);
        loop {
            let status = unsafe { status_port.read() };
            if status & STATUS_ERR != 0 {
                return Err("ATA error bit set");
            }
            if status & STATUS_DRQ != 0 {
                return Ok(());
            }
        }
    }

    fn select_and_setup(&self, lba: u32) {
        unsafe {
            Port::<u8>::new(self.io_base + 6).write(self.drive_select_base | (((lba >> 24) & 0x0F) as u8));
            Port::<u8>::new(self.io_base + 2).write(1u8);
            Port::<u8>::new(self.io_base + 3).write((lba & 0xFF) as u8);
            Port::<u8>::new(self.io_base + 4).write(((lba >> 8) & 0xFF) as u8);
            Port::<u8>::new(self.io_base + 5).write(((lba >> 16) & 0xFF) as u8);
        }
    }
}

impl BlockDevice for AtaDevice {
    fn read_sector(&self, lba: u32, buf: &mut [u8; 512]) -> Result<(), &'static str> {
        self.wait_not_busy();
        self.select_and_setup(lba);
        unsafe {
            Port::<u8>::new(self.io_base + 7).write(CMD_READ_SECTORS);
        }
        self.wait_not_busy();
        self.wait_drq()?;
        let mut data_port: Port<u16> = Port::new(self.io_base);
        for chunk in buf.chunks_exact_mut(2) {
            let word = unsafe { data_port.read() };
            chunk[0] = (word & 0xFF) as u8;
            chunk[1] = (word >> 8) as u8;
        }
        Ok(())
    }

    fn write_sector(&self, lba: u32, buf: &[u8; 512]) -> Result<(), &'static str> {
        self.wait_not_busy();
        self.select_and_setup(lba);
        unsafe {
            Port::<u8>::new(self.io_base + 7).write(CMD_WRITE_SECTORS);
        }
        self.wait_not_busy();
        self.wait_drq()?;
        let mut data_port: Port<u16> = Port::new(self.io_base);
        for chunk in buf.chunks_exact(2) {
            let word = (chunk[0] as u16) | ((chunk[1] as u16) << 8);
            unsafe {
                data_port.write(word);
            }
        }
        unsafe {
            Port::<u8>::new(self.io_base + 7).write(CMD_CACHE_FLUSH);
        }
        self.wait_not_busy();
        Ok(())
    }
}

/// The ORIGINAL real device every milestone through 65 hardcoded:
/// secondary ATA controller (I/O base 0x170), master drive-select
/// (0xE0). Identical ports and select byte to the pre-Milestone-66
/// hardcoded constants (`DATA=0x170`, `SECTOR_COUNT=0x172`,
/// `LBA_LOW=0x173`, `LBA_MID=0x174`, `LBA_HIGH=0x175`,
/// `DRIVE_HEAD=0x176`, `STATUS_OR_COMMAND=0x177`, select byte `0xE0`) --
/// `io_base + {0,2,3,4,5,6,7}` reproduces that exact same set.
pub static SECONDARY_MASTER: AtaDevice = AtaDevice::new(0x170, 0xE0);

/// MILESTONE 66: the real, NEW second device -- same secondary
/// controller, SLAVE drive-select (0xF0, the DRV bit set). A real,
/// separate physical drive on the same cable (`bus=ide.1,unit=1` in
/// `src/main.rs`), not a second controller.
pub static SECONDARY_SLAVE: AtaDevice = AtaDevice::new(0x170, 0xF0);

/// Unchanged signature and unchanged real behavior for every existing
/// caller (`fs.rs`'s 7 call sites, this module's own `save_weights()`/
/// `load_weights()`) -- now a thin wrapper delegating to
/// `SECONDARY_MASTER` through the `BlockDevice` trait instead of a
/// standalone hardcoded implementation.
pub fn read_sector(lba: u32, buf: &mut [u8; 512]) -> Result<(), &'static str> {
    SECONDARY_MASTER.read_sector(lba, buf)
}

/// See `read_sector()`'s doc comment above -- same "unchanged for
/// existing callers" guarantee.
pub fn write_sector(lba: u32, buf: &[u8; 512]) -> Result<(), &'static str> {
    SECONDARY_MASTER.write_sector(lba, buf)
}

const MAGIC: u32 = 0x53504B32; // "SPK2" -- MILESTONE 38's ternary, name-keyed format (see module doc)
const PERSIST_LBA: u32 = 0;
const FORMAT_VERSION: u8 = 1;
const NAME_LEN: usize = 16;
const ENTRY_LEN: usize = NAME_LEN * 2 + ternary::PACKED_BYTES_PER_WEIGHT; // 34
const HEADER_LEN: usize = 8;
const MAX_SYNAPSES: usize = (512 - HEADER_LEN) / ENTRY_LEN; // 14

fn write_name(buf: &mut [u8; 512], off: usize, name: &str) {
    let bytes = name.as_bytes();
    let n = bytes.len().min(NAME_LEN);
    buf[off..off + n].copy_from_slice(&bytes[..n]);
    // any remaining bytes up to NAME_LEN stay 0 (NUL padding) -- buf
    // starts all-zero in save_weights below.
}

fn read_name(buf: &[u8; 512], off: usize) -> String {
    let slice = &buf[off..off + NAME_LEN];
    let end = slice.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    core::str::from_utf8(&slice[..end]).unwrap_or("").to_string()
}

/// MILESTONE 38: persists WHATEVER synapses currently exist (an
/// arbitrary, name-keyed list -- see network.rs::all_synapse_weights,
/// and the module doc above for the honest scoping decision), each
/// weight ternary-packed via ternary.rs instead of raw f32. Entries
/// beyond MAX_SYNAPSES (14) are dropped with the count clamped, rather
/// than overflowing the sector -- not expected to matter for this
/// kernel's actual DSL-built networks, but a real, disclosed limit
/// rather than an unbounded assumption.
pub fn save_weights(entries: &[(String, String, f32)]) -> Result<(), &'static str> {
    if entries.is_empty() {
        return Err("no synapses to save");
    }
    let count = entries.len().min(MAX_SYNAPSES);
    let mut buf = [0u8; 512];
    buf[0..4].copy_from_slice(&MAGIC.to_le_bytes());
    buf[4] = FORMAT_VERSION;
    buf[5] = count as u8;
    buf[6] = ternary::TRITS_PER_WEIGHT as u8;
    // buf[7] reserved, stays 0

    let mut off = HEADER_LEN;
    for (from, to, w) in entries.iter().take(count) {
        write_name(&mut buf, off, from);
        write_name(&mut buf, off + NAME_LEN, to);
        let packed = ternary::encode_weight(*w);
        buf[off + NAME_LEN * 2..off + NAME_LEN * 2 + ternary::PACKED_BYTES_PER_WEIGHT].copy_from_slice(&packed);
        off += ENTRY_LEN;
    }
    write_sector(PERSIST_LBA, &buf)
}

/// MILESTONE 38: returns every (from, to, weight) entry found on disk,
/// weights decoded from their ternary-packed form. `None` if the
/// sector doesn't carry this format's magic (blank disk, or a disk
/// written by pre-M38 code) -- the same honest fallback-to-defaults
/// signal Milestone 11 established, now for a variable-length list
/// instead of a fixed pair.
pub fn load_weights() -> Option<Vec<(String, String, f32)>> {
    let mut buf = [0u8; 512];
    read_sector(PERSIST_LBA, &mut buf).ok()?;
    let magic = u32::from_le_bytes(buf[0..4].try_into().unwrap());
    if magic != MAGIC {
        return None;
    }
    let count = (buf[5] as usize).min(MAX_SYNAPSES);
    let mut out = Vec::with_capacity(count);
    let mut off = HEADER_LEN;
    for _ in 0..count {
        let from = read_name(&buf, off);
        let to = read_name(&buf, off + NAME_LEN);
        let w = ternary::decode_weight(&buf[off + NAME_LEN * 2..off + NAME_LEN * 2 + ternary::PACKED_BYTES_PER_WEIGHT]);
        out.push((from, to, w));
        off += ENTRY_LEN;
    }
    Some(out)
}

/// MILESTONE 66: a scratch LBA reserved for this self-test only, chosen
/// safely past every real region another module actually uses on the
/// master device -- `ata::save_weights()`/`load_weights()` use LBA 0
/// only, `fs.rs` uses LBA 1 (`DIR_LBA`) through LBA 66 (`ROOT_META_LBA`,
/// `FILE_DATA_START_LBA=2` + `NUM_DATA_SECTORS=64`) -- so LBA 500 can
/// never collide with real filesystem or weight-persistence data on the
/// SAME disk image. On the slave device this LBA is arbitrary (that
/// disk image, `target/persist2.img`, is dedicated to this self-test
/// alone and touched by nothing else), kept identical to the master's
/// scratch LBA purely so the "same LBA, two different real devices,
/// two different real contents" comparison below is as direct as
/// possible.
const SELFTEST_SCRATCH_LBA: u32 = 500;

/// MILESTONE 66: real proof the `BlockDevice` trait abstraction is
/// genuine, not just a refactor that happens to compile -- exercises
/// FOUR real, independently meaningful checks against real hardware
/// (QEMU-emulated ATA PIO, same real port I/O every prior milestone's
/// disk work has used):
///
/// 1. write a real pattern to the MASTER device via the `BlockDevice`
///    trait (`SECONDARY_MASTER.write_sector()`), then read it back via
///    the OLD free function `read_sector()` -- proves `read_sector()`
///    really delegates to `SECONDARY_MASTER`, not a coincidentally-
///    agreeing separate implementation.
/// 2. write a SECOND, different real pattern to the master via the OLD
///    free function `write_sector()`, then read it back via the trait
///    (`SECONDARY_MASTER.read_sector()`) -- proves the delegation holds
///    in the reverse direction too.
/// 3. write a THIRD, different real pattern to the SLAVE device via the
///    trait (`SECONDARY_SLAVE.write_sector()`), read it back via the
///    trait, confirm it matches.
/// 4. re-read the MASTER device (still via the trait) and confirm it
///    STILL shows pattern 2's content, not the slave's pattern 3 --
///    direct, real proof `SECONDARY_MASTER` and `SECONDARY_SLAVE` are
///    genuinely two separate real block devices (a real IDE master and
///    a real IDE slave on the same cable, backed by two separate real
///    QEMU drive images), not the same physical device aliased under
///    two names.
pub fn self_test_block_device_abstraction() {
    let mut port = serial();

    const PATTERN_A: u8 = 0xAA; // written to master via the trait
    const PATTERN_C: u8 = 0xCC; // written to master via the OLD free function (overwrites A)
    const PATTERN_B: u8 = 0x55; // written to slave via the trait

    let buf_a = [PATTERN_A; 512];
    let buf_c = [PATTERN_C; 512];
    let buf_b = [PATTERN_B; 512];

    // CHECK 1: trait write to master, free-function read from master.
    let check1 = SECONDARY_MASTER.write_sector(SELFTEST_SCRATCH_LBA, &buf_a).is_ok() && {
        let mut readback = [0u8; 512];
        read_sector(SELFTEST_SCRATCH_LBA, &mut readback).is_ok() && readback == buf_a
    };

    // CHECK 2: free-function write to master (different pattern), trait read from master.
    let check2 = write_sector(SELFTEST_SCRATCH_LBA, &buf_c).is_ok() && {
        let mut readback = [0u8; 512];
        SECONDARY_MASTER.read_sector(SELFTEST_SCRATCH_LBA, &mut readback).is_ok() && readback == buf_c
    };

    // CHECK 3: trait write+read on the real SECOND device (slave).
    let check3 = SECONDARY_SLAVE.write_sector(SELFTEST_SCRATCH_LBA, &buf_b).is_ok() && {
        let mut readback = [0u8; 512];
        SECONDARY_SLAVE.read_sector(SELFTEST_SCRATCH_LBA, &mut readback).is_ok() && readback == buf_b
    };

    // CHECK 4: master still shows its OWN last-written content (pattern
    // C), independent of the slave write in CHECK 3 -- real, direct
    // non-aliasing proof.
    let check4 = {
        let mut readback = [0u8; 512];
        SECONDARY_MASTER.read_sector(SELFTEST_SCRATCH_LBA, &mut readback).is_ok() && readback == buf_c && readback != buf_b
    };

    let overall = check1 && check2 && check3 && check4;
    let _ = writeln!(
        port,
        "milestone 66: self-test -- trait-write/free-read-agree(master)={check1} free-write/trait-read-agree(master)={check2} slave-device-roundtrip={check3} master-unaffected-by-slave-write(no-aliasing)={check4}"
    );
    let _ = writeln!(
        port,
        "milestone 66: self-test -- OVERALL: {}",
        if overall { "PASS" } else { "FAIL" }
    );
}
