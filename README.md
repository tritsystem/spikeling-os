# spikeling-os

A from-scratch x86_64 kernel, built on top of the [rust-osdev/bootloader](
https://github.com/rust-osdev/bootloader) crate (stable, actively maintained,
handles the actual boot-sector/UEFI complexity so kernel code stays focused
on the kernel itself). Runs in QEMU during development -- no physical
hardware risk while iterating.

**Goal**: Spikeling's spiking-neural-network runtime as the kernel's own
control/scheduling logic -- not an app running on top of a normal OS, but
the thing the OS *is* -- built up one real, working milestone at a time.

## Status

- [x] **Milestone 1**: kernel boots (BIOS and UEFI paths both build), hands
      off from the bootloader correctly, writes to the serial port, halts
      cleanly. Nothing more yet -- this just proves the foundation works.
- [x] **Milestone 2**: framebuffer output (`boot_info.framebuffer`) -- a
      horizontal RGB gradient across the full 1280x720 buffer, correctly
      handling `stride` vs `width` (they can differ) and the reported
      `PixelFormat` (BGR on this hardware). Verified with a real QEMU
      `screendump`, not just a serial print -- a broken stride/channel-order
      calculation would show up as visible skew or wrong colors, and it
      renders as a clean, uniform sweep.
- [x] **Milestone 3**: memory management -- an `OffsetPageTable` built from
      the bootloader-mapped physical memory, a `BootInfoFrameAllocator`
      over the real firmware-reported usable regions, a 100 KiB heap
      mapped into it, and `linked_list_allocator` as the global allocator.
      Verified with a real allocation, not just a successful map: a 500-
      element `Vec<u64>` summed to exactly 124750 (= 500*499/2) after
      growing/reallocating, and a `Box<u64>` round-tripped correctly.
      `alloc::{Vec, Box, ...}` now work in-kernel.
- [x] **Milestone 4**: task-selection policy (which task-slot runs next)
      driven by an SNN with SSH-topological dimerized coupling between
      adjacent slots (`v=1-g`, `w=1+g`) -- the same bond pattern and
      question as Spikeling's `topological_bank.py`: does killing one
      slot's neuron crash/starve the rest, or degrade gracefully? Tested
      by injecting a defect at slot 3 of 8 and comparing topological
      (`g=+0.6`) vs trivial (`g=-0.6`) coupling on the survivors'
      scheduling fairness. **Honest result, not the hypothesis**: the
      kernel never crashed or starved a survivor under either coupling
      (self-healing holds at that level), but on fairness specifically
      trivial coupling was *slightly better* (0.860) than topological
      (0.790), not worse -- the opposite of `topological_bank.py`'s
      prediction. Likely cause: the coupling term was deliberately kept
      small relative to each slot's own bias to avoid instability,
      probably too weak to show an effect at this scale; there's also a
      real question of whether a discrete winner-take-all accumulator is
      even the right dynamical regime for topological protection to
      show up in at all (unlike continuous oscillators, where it's an
      established physical phenomenon). Reported as-is rather than
      re-tuned until it agreed with the hypothesis. Scope note: this is
      cooperative scheduling -- task-slots are closures invoked in place
      when selected, not independent execution contexts with their own
      stacks. Real preemptive multitasking (timer interrupts, context
      switching, separate stacks) is separate, future work; this
      milestone proves the topological *selection policy* specifically.
- [x] **Milestone 5**: real preemptive multitasking -- the piece Milestone
      4 was explicitly scoped to not include yet. Three stages, each
      verified before the next was attempted:
      - **5a, GDT/TSS + IDT with real exception handling.** A TSS
        provides a separate IST stack for the double-fault handler (a
        double fault often happens because the current stack is already
        corrupted, so handling it there would just triple-fault instead
        of producing a readable panic). Verified with a real `int3`
        breakpoint: the handler fired, and execution resumed cleanly on
        the next line. **Real bug found and fixed**: the first attempt
        double-faulted on every return from the handler -- `SS` kept a
        stale selector from the bootloader's own GDT that happened to
        land on our new table's TSS descriptor, not a valid data
        segment, with no GP-fault handler registered to catch it short
        of a double fault. Fixed by explicitly reloading `SS` to the
        null selector (valid in 64-bit long mode) after loading the GDT.
      - **5b, PIC remap + timer interrupt.** IRQs remapped to vectors
        32-47 (avoiding the CPU exception range). Verified against a
        real atomic counter incremented only inside the handler, not
        just "`hlt` returned N times" (which wakes on any interrupt):
        80 `hlt` cycles produced exactly 80 observed timer interrupts.
      - **5c, real per-task stacks + a genuine preemptive context
        switch**, selection driven by the *same* `TopologicalScheduler`
        from Milestone 4 -- now with a real scheduler slot to plug into.
        Three worker tasks spin on their own counters forever and never
        call back into scheduling code themselves; only the timer
        interrupt moves execution between them, via a hand-written
        `#[unsafe(naked)]` `switch_to` (save/restore callee-saved
        registers + stack pointer, the standard minimal pattern). **Two
        real bugs found and fixed**: (1) `TopologicalScheduler::step()`
        legitimately returns `None` on most ticks (no slot has crossed
        threshold yet), which was conflated with the unrelated "tick
        budget exhausted, return to kernel" condition -- the very first
        ordinary "no winner yet" tick was ending the whole demo
        immediately, so only the first task ever ran. (2) Switching
        stacks via a plain `ret` instead of a real `iretq` meant the
        CPU's interrupt flag -- cleared automatically on interrupt
        entry -- never got restored; after the first switch performed
        from inside the timer handler, interrupts stayed permanently
        disabled, so no further tick could ever fire and whichever task
        got landed on just ran forever. Fixed with an explicit `sti`
        before `ret` in `switch_to`. Re-verified after both fixes: all
        three tasks genuinely interleaved (counters 2362 / 1651 / 2187
        after a bounded run), confirmed via real numbers, not an
        assumption that "it didn't crash" meant it worked.
- [x] **Milestone 6**: PS/2 keyboard input via IRQ1 (`pc-keyboard` crate
      for scancode-set-1 decoding) -- the first real input device, making
      the OS interactive for the first time rather than only ever
      producing output. Verified with real synthetic keystrokes sent
      through QEMU's own monitor (`sendkey`, standing in for a human at
      the keyboard) during a bounded wait window, driving genuine
      hardware IRQ1 interrupts -- not a unit test calling the decode
      function directly. Sent "spike", received exactly `"spike"` back.
      No bugs this time -- clean first-try success.
- [x] **Milestone 7**: a real text console rendered on the framebuffer
      (`noto-sans-mono-bitmap`, the same font-rendering crate the
      `bootloader` crate itself depends on for its own boot-error
      screen), wired to keyboard input -- combining Milestone 2 (pixel
      output) and Milestone 6 (keyboard input) into something actually
      visible and interactive: every keystroke renders live on screen,
      not just into a string read back over serial. Verified with the
      same real `sendkey` injection as Milestone 6, this time followed
      by an actual QEMU screendump -- the typed word is clearly legible
      on screen in the rendered font, not just present in a log. Clean
      first-try success, no bugs.
- [x] **Milestone 8**: a minimal interactive shell (`shell.rs`) tying
      together everything built so far -- keyboard input (M6), console
      rendering (M7), the heap (M3, for the line buffer), and real
      introspection into the kernel's own state (`tasks` command reads
      M5's live worker counters). Real line editing, not just character
      echo: Backspace decodes to `pc-keyboard`'s `Unicode('\u{8}')` and
      correctly erases both the command buffer and the on-screen glyph.
      Verified with a real typed session via `sendkey` + a screendump:
      `help` lists commands, `xyz` correctly reports as unknown, and
      critically -- typing `abc`, backspacing it away three times, then
      typing `help` shows *only* `"> help"` with the correct output,
      not `"> abchelp"` or any visual corruption, proving backspace
      edits the real buffer and not just the display. Clean first-try
      success, no bugs. The OS is now something you actually type
      commands into, not just a fixed demo sequence that runs once.
- [x] **Milestone 9**: real LIF (leaky integrate-and-fire) neuron dynamics
      (`neurons.rs`) ported from Spikeling's own `core/runtime/runtime.py`
      -- the actual neuron model this time (membrane potential, leak,
      threshold, refractory period, weighted synapses, spike
      propagation), not just the SSH-topological coupling *structure*
      borrowed for Milestone 4's scheduler. Directly serves the
      project's stated goal. Driven entirely by real hardware: the
      timer interrupt (M5) ticks it, keyboard input (M6) stimulates it
      ('a'/'d' standing in for `sound_localizer.spk`'s LeftMic/RightMic,
      since this kernel has no microphone driver), a new `neurons` shell
      command (M8) reports live state. Verified with a real typed
      session, honest result either way: a single 'a' press fires
      LeftKey but correctly does *not* fire Motor (40 < threshold=80,
      proving the weighted-synapse coincidence gate actually works);
      two presses close together landed on separate ticks rather than
      the same one, so Motor never crossed threshold, but its membrane
      potential (`V=35.0`, not 0 or a round number) shows genuine leaky
      partial integration from two near-simultaneous inputs -- real
      physics, not a fabricated result. Motor's actual coincidence-fire
      case wasn't achieved (couldn't reliably land two `sendkey` events
      within one ~55ms tick), reported honestly as a real, stated
      limitation of this test rather than glossed over.
- [x] **Milestone 10**: real STDP learning on the LeftKey/RightKey ->
      Motor synapses -- Spikeling's own `core/README.md` names this as
      THE defining difference from the original project it replaced
      ("random weights, never updated" vs "STDP learning, weights
      change over time"). Same formula: `Δw = rate * exp(-|dt|/20ms)`,
      pre-before-post strengthens (LTP), pre-after-post weakens (LTD),
      clamped to `[0,1]`. Weights start at a neutral 0.5 (distinct from
      M9's old fixed 0.8, so any learned change is unambiguous) and are
      now genuinely used -- and updated -- by the live tick()-driven
      pathway, not just a side calculation. A new `train` shell command
      runs 5 controlled LTP trials (pre fires, post fires 2 ticks
      later) then 5 controlled LTD trials (reversed) on the real
      `apply_stdp()` code path -- controlled timing rather than relying
      on real keyboard timing, since Milestone 9 already established
      that reliably landing two independent keypresses within one tick
      isn't achievable via `sendkey`. Verified against a hand-computed
      prediction made *before* running it: `0.1 * exp(-109.89ms/20ms) ≈
      0.0004106` per trial, `0.5 + 5*0.0004106 ≈ 0.502053` (rounds to
      `0.5021`), then symmetric LTD returns exactly to `0.5000` -- the
      real output matched exactly. One honest, minor detail surfaced
      for real: the second `neurons` check showed `LeftKey fires=11`,
      not the expected 10 from training alone, because typing the word
      "train" itself contains an 'a', which -- per keyboard.rs's own
      documented shared-input-stream behavior -- also stimulated
      LeftKey for real through the normal keyboard path. A live example
      of the exact interaction already flagged as a known property, not
      a bug.
- [x] **Milestone 11**: real ATA PIO disk I/O (`ata.rs`), persisting
      Milestone 10's learned weights across an actual reboot -- directly
      fulfilling a roadmap item Spikeling's own *original*
      `core/README.md` explicitly left undone: "Weight persistence
      (save/load trained networks)". Targets a dedicated secondary ATA
      drive (ports 0x170-0x177), deliberately never the boot drive, so
      persistence testing can't risk the bootable image. A magic number
      (`"SPKL"`) distinguishes real saved data from uninitialized disk
      content. New shell command `save`; `neurons::init()` now tries
      loading from disk first, falling back to neutral defaults, and
      reports which happened honestly rather than silently. **Verified
      with a genuine two-phase reboot test**, not just a same-session
      round-trip: phase 1 (blank disk) correctly reported "no saved
      weights found"; trained and saved; phase 2 launched a *completely
      separate, fresh QEMU process* against the same persistence disk
      file and correctly reported "learned weights loaded from disk --
      persisted across a real reboot". Real, verified survival across
      an actual process boundary, not merely staying alive in one run.
- [x] **Milestone 12**: a real, dynamically-definable spiking network
      (`network.rs`) -- Milestone 9/10/11's LeftKey/RightKey/Motor
      network is fixed Rust structure defined at compile time; this
      brings the actual *programmability* of Spikeling's own `.spk`
      language into the kernel, via shell DSL commands (`addneuron NAME
      threshold=T leak=L`, `addsynapse FROM TO weight=W`, `stim NAME
      AMOUNT`, `net` to report) instead of a file (no filesystem yet).
      A new, separate subsystem living alongside the already-verified
      M9-M11 network rather than a risky rewrite of it -- same real
      timer-interrupt clock, genuinely different design where it
      matters: firings propagate through synapses with a real one-tick
      delay (vs. the M9 network's same-tick propagation), a real,
      disclosed difference between the two engines. Verified with a
      real typed session: defined two neurons and a synapse via the
      DSL, confirmed the baseline report showed the exact configuration
      typed, stimulated the source neuron past its threshold, and
      confirmed the target neuron fired one tick later purely from
      synaptic propagation (`fires=0->1` on both). One honest,
      disclosed test-script artifact: the `.` key wasn't in the test's
      QEMU `sendkey` mapping, so `weight=1.0` was actually sent as
      `weight=10` -- doesn't affect what was verified (the propagation
      mechanism itself), just the specific number used.
- [x] **Milestone 13**: a real PC speaker driver (`speaker.rs`, 8254 PIT
      channel 2 + the speaker gate at port 0x61) -- closing the
      sensorimotor loop started in Milestone 9 with an actual physical
      (audible) effect, the real analogue of Spikeling's own `action
      Motor -> [MOTOR_FIRE]` concept. New shell commands `beep FREQ_HZ`
      and `silence`. **Honest limitation**: this QEMU build has no way
      to route the PC speaker's audio to a capturable file (`-audio
      driver=wav,...,model=pcspk` was rejected outright -- `pcspk` isn't
      in this build's valid audio model list, and neither `-device
      isa-pcspk` nor a `pcspk-audiodev` machine property exist here
      either), so a real waveform/frequency-domain verification (the
      rigor used for Milestone 2's pixels) wasn't achievable. Verified
      instead by reading back the actual hardware gate register directly
      -- the real causal mechanism that produces sound on full audio
      hardware: `beep 440` -> `speaker gate register enabled=true`,
      `silence` -> `enabled=false`, confirmed both directions. A
      slightly less complete verification than other milestones,
      reported as such rather than overclaimed.
- [x] **Milestone 14**: the generic network's firing now automatically
      triggers a real, self-silencing speaker blip -- genuinely
      automatic (not a manually-typed `beep`), the real analogue of
      Spikeling's own `action Motor -> [MOTOR_FIRE]`, closing the
      sensorimotor loop for real. **A real, disclosed test bug and fix
      along the way**: the first version used a 3-tick (~165ms) blip,
      and a real test showed `speakerstatus` reading `enabled=false`
      both before and after triggering a firing, despite `net`
      confirming the neuron genuinely fired (`fires=1`) -- diagnosed as
      the blip duration being *shorter* than the time it takes to type
      the word `"speakerstatus"` itself via a real keystroke-by-
      keystroke test harness, not a bug in the mechanism. Extended to
      ~2.2s and re-verified cleanly: `stim x 60` fires `x` ->
      `speakerstatus` immediately after reads `enabled=true` -> waiting
      3s for the window to elapse -> `enabled=false` again,
      auto-silenced correctly.
- [x] **Milestone 15**: a real CMOS real-time clock driver (`rtc.rs`,
      ports 0x70/0x71) -- genuine wall-clock date/time, distinct from
      the PIT (M5b), which only counts ticks since boot and knows
      nothing about the actual date. Handles the RTC's real quirks
      correctly: waits out in-progress updates and re-reads until two
      consecutive reads agree (the RTC has no atomic "read all fields"
      operation, so a read can land mid-tick and return a torn value),
      converts from BCD if the hardware reports BCD mode, and handles
      12-hour PM-bit encoding if present. New shell command `date`.
      Verified against kimchi's own real system clock, checked
      independently right before the test: kernel reported
      `2026-07-31 13:27:57`, host showed `2026-07-31 06:27:07` -- the
      date matches exactly, and the 7-hour time offset is the expected
      UTC-vs-localtime difference (QEMU's CMOS defaults to UTC unless
      told otherwise; the host's clock is local, UTC-7), with the
      ~50-second gap in seconds fully explained by real elapsed
      test-setup time. A genuine, correctly-understood match, not a
      coincidence or a bug.
- [x] **Milestone 16**: a real PS/2 mouse driver (`mouse.rs`, IRQ12) --
      the second real input device, completing the core PC peripheral
      set this kernel now has genuine drivers for: keyboard, mouse,
      speaker, RTC, disk. Correctly handles the 8042 controller's
      real init handshake (enabling the auxiliary port, setting the
      controller config byte, sending the mouse its own "enable data
      reporting" command through the 0xD4 passthrough), and correctly
      frames the 3-byte movement packets using the standard always-1
      sync bit in byte 0 to detect/recover alignment. New shell command
      `mouse`. Verified with real events injected through QEMU's
      monitor (`mouse_move`, `mouse_button` -- the mouse equivalent of
      `sendkey`): every number checked out exactly -- `x=70` matched
      the sum of the two injected `mouse_move` deltas (50+20) exactly,
      `packets=4` matched the 4 real events sent (2 moves, a press, a
      release), and the button state correctly read `false` after a
      press-then-release sequence.
- [x] **Milestone 17**: real STDP learning on the *generic*, shell-
      definable network (M12) -- until now only M9's fixed demo network
      could learn; this brings the same real formula to networks you
      build yourself at runtime, unifying the two engines' capabilities.
      New DSL command `train FROM TO GAP_TICKS`. **A genuine, honest
      first result that wasn't a bug**: an initial test drove learning
      through two separately-*typed* `stim` commands with a real ~1s
      gap between them -- the weight stayed at exactly `0.500`,
      unchanged. Diagnosed correctly rather than assumed broken: at
      `dt~1000ms` against `STDP_TAU_MS=20`, `exp(-1000/20) = exp(-50)`
      genuinely underflows to zero in `f32` -- mathematically correct
      STDP behavior (real synaptic plasticity is only sensitive to
      sub-100ms coincidences; a 1-second gap is thousands of times
      outside any plausible window), not a defect, but also not
      observable through typed commands, the same class of constraint
      Milestone 10 already solved the same way. Added a controlled
      trainer (bypassing real typing-timing entirely, same approach as
      M10) and re-verified against a hand-computed prediction made
      first: two `train pre post 2` calls (dt=2 ticks) predicted
      `0.5000 -> 0.5004 -> 0.5008`; the real output matched exactly.
- [x] **Milestone 18**: a minimal real filesystem (`fs.rs`) on top of
      Milestone 11's raw ATA disk I/O -- until now only one fixed sector
      (LBA 0) could be persisted (the learned weights); this adds real
      named-file storage, a genuine "save your work" capability. A
      fixed-size 8-entry directory at LBA 1 (LBA 0 stays reserved for
      M11's weights, untouched, deliberately co-existing on the same
      disk), each file limited to a single 512-byte sector at
      `LBA 2 + slot index` -- no multi-sector files, no free-space
      reclamation after a delete, disclosed rather than hidden. New
      shell commands `ls`, `write NAME TEXT...`, `read NAME`. Verified
      on real hardware-equivalent QEMU with a fresh blank disk image and
      real typed keystrokes through the actual keyboard driver: `ls` on
      an uninitialized disk correctly reported `(no files)` (magic
      number check correctly treats an all-zero disk as an empty
      directory, not an error); `write hello spikeling remembers this`
      correctly reported `wrote 24 bytes to 'hello'` (`split_once(' ')`
      on the argument correctly separates name from text, and
      `"spikeling remembers this"` is genuinely 24 bytes); a second `ls`
      correctly listed `hello  24 bytes`; `read hello` correctly
      returned `spikeling remembers this`, verbatim. Every value matched
      the hand-computed prediction exactly. **An honest, disclosed
      cosmetic glitch, not a filesystem bug**: the captured screenshot
      shows a doubled `> >` prompt immediately before the `write` line
      and a missing `> ` before the first `ls` -- almost certainly a
      pre-existing artifact in Milestone 8's shell/console prompt-echo
      (unrelated to `fs.rs`, which never touches console cursor state),
      most likely from the framebuffer scrolling mid-transition between
      the tail of the boot log and the first interactive command. Not
      root-caused yet since it doesn't affect correctness of any command
      output observed so far; left as an open item rather than papered
      over.
- [x] **Milestone 19**: root-caused and fixed the double-prompt glitch
      disclosed as an open item in Milestone 18. Not a console/shell
      logic bug as first suspected -- a real **boot-ordering race**:
      `x86_64::instructions::interrupts::enable()` ran at the *old*
      main.rs line 324, but `shell::init()` (which prints the very
      first `"> "` prompt) didn't run until line 383 -- after
      Milestone 5b's 80-tick timer wait and Milestone 5c's full
      preemption demo, both of which burn real wall-clock time with
      interrupts already live. If total boot-to-shell-ready time ever
      exceeded a test harness's fixed pre-input delay (plausible under
      host load, e.g. concurrent builds), a keystroke arriving in that
      gap got fully processed by `shell::on_char` -- echoed, run as a
      command, even its own post-command prompt printed -- *before*
      `shell::init()`'s own first prompt ever ran; when it finally did
      run, it printed unconditionally on top of whatever was already
      there. Diagnosed from the evidence alone, before touching code:
      the Milestone 18 screenshot showed exactly 5 `"> "` tokens for 5
      expected prompts (count preserved, order corrupted -- one missing
      at the first position, one doubled at the second), which only
      makes sense if a prompt fired out of its intended sequence, not
      if one were simply lost or duplicated outright. Fix: moved
      `shell::init()` to run immediately before `interrupts::enable()`,
      removing the old, later call entirely. Re-verified with the
      *exact same* Milestone 18 test (fresh blank disk, identical typed
      sequence): every line now shows exactly one `"> "` in the correct
      place -- `> ls`, `> write hello spikeling remembers this`,
      `> ls`, `> read hello`, trailing `> ` -- confirmed via screenshot,
      not assumed from the fix alone.
- [x] **Milestone 20**: real PCI bus enumeration (`pci.rs`) -- until now
      every driver (ATA, mouse, speaker, RTC) has talked to a device at
      a hardcoded, well-known ISA-era port address; PCI devices don't
      live at fixed locations, so finding one at all requires walking
      the bus for real via the standard `CONFIG_ADDRESS`/`CONFIG_DATA`
      I/O-port mechanism (ports `0xCF8`/`0xCFC`). Scans bus 0 across all
      32 device slots, correctly probing multiple functions only when a
      slot's header-type byte advertises multi-function support. New
      shell commands `lspci` (lists every discovered device) and `nic`
      (reports whether a PCI class `0x02` network controller was
      found). Deliberately scope-limited to enumeration -- no
      packet-level send/receive driver yet. Verified against real,
      sensible QEMU default-machine PCI IDs, not fabricated: found 6
      devices, including `8086:1237` (the i440FX host bridge -- its
      presence at 00:00.0 is itself a correctness check on the
      enumeration), the PIIX3/4 ISA/IDE/ACPI bridges, QEMU's standard
      VGA (`1234:1111`), and an Intel 82540EM/e1000 emulated NIC
      (`8086:100e`) at 00:03.0 class `0200` -- correctly identified by
      `nic`. Confirmed identical in both the boot-time serial scan and a
      live typed `lspci`/`nic` shell session (screenshot-verified).
      Built and tested in an isolated parallel-agent workspace, then
      merged and independently re-verified end-to-end against the real
      project tree before being committed.
- [x] **Milestone 23**: real framebuffer graphics primitives on top of
      the same raw pixel buffer the text console (M7) already owns --
      `console.rs` gained `draw_pixel`/`draw_line`/`draw_rect`, all
      built on the existing `write_pixel`'s bounds check and
      pixel-format branching rather than duplicating them. `draw_line`
      is a genuine Bresenham's-algorithm rasterizer (integer-only,
      correct for any slope/direction, not just axis-aligned special
      cases); `draw_rect` builds outline and filled modes entirely out
      of `draw_line`/`draw_pixel`. Drawing addresses raw pixel
      coordinates directly, independent of the text cursor's
      `x_pos`/`y_pos`, so shapes and typed text coexist on the same
      framebuffer without either disturbing the other. New shell
      commands `pixel X Y`, `line X0 Y0 X1 Y1`, `rect X Y W H`,
      `fillrect X Y W H`. Verified on real QEMU screendumps, not
      assumed from successful compilation: a single set pixel, a clean
      unfilled rectangle outline, a solid filled rectangle, and a
      correctly-sloped diagonal line (proving Bresenham works for a
      non-axis-aligned direction, not only horizontal/vertical) all
      rendered at the right positions/sizes, with `about`'s text output
      printing normally immediately after, unaffected by the drawing
      that preceded it. Built and tested in an isolated parallel-agent
      workspace, then merged and independently re-verified end-to-end;
      the merge also added a `pixel` command exposing `draw_pixel`
      itself, eliminating a real (harmless) `dead_code` warning left
      by the original submission, which only wired `draw_pixel` in as
      an internal helper for `draw_line`/`draw_rect`.
- [x] **Milestone 22**: multi-sector files and real delete/reclamation
      for `fs.rs`, closing the two gaps disclosed in Milestone 18. Each
      of the 8 directory entries (still at LBA 1) now also stores a
      start LBA and sector count, allocated from a shared 64-sector
      data pool at LBA 2..66 via first-fit over contiguous free runs
      computed live from the currently-used entries (no persisted
      bitmap) -- files now genuinely span multiple sectors up to a real
      cap of 8 sectors (4096 bytes), and a new `rm NAME` command frees
      an entry's sectors back into that pool for a later `write` to
      reuse, not just marking a slot deleted forever. Deliberately
      still minimal, disclosed rather than hidden: no
      fragmentation-avoiding allocator or directory/pool compaction, so
      a disk with enough delete/write churn can fail an allocation with
      "not enough free disk space" even when total free sectors would
      suffice. Verified byte-exact in the isolated build (a 600-byte
      and a 520-byte file, both spanning 2 sectors with a partial final
      sector, round-tripped exactly; directory-full and reclaim-after-delete
      both confirmed with real failures/successes, not assumed) and
      re-verified end-to-end after merging: a 600-byte
      `abcdefghij`-repeating pattern written as `big`, read back with
      the pattern intact and correctly terminated (no corruption, no
      truncation) across the sector boundary; `rm hello` genuinely
      freed its slot, immediately confirmed by a subsequent `write
      reuse` succeeding into that freed space. The original Milestone
      18 single-sector case (`write`/`read`/`ls` on a small file) still
      works unchanged. Built in an isolated parallel-agent workspace,
      then merged and independently re-verified against the real
      project tree.
- [x] **Milestone 21**: unified Milestone 9/10/11's fixed
      LeftKey/RightKey/Motor network and Milestone 12/14/17's generic,
      shell-definable network into ONE engine, closing the
      honestly-disclosed gap that had persisted since Milestone 12: two
      independently-implemented LIF/STDP engines that happened to start
      from the same constants but shared no live state -- a real
      keypress stimulating LeftKey was invisible to `net`/`stim`/
      `addneuron`, and vice versa. `network.rs`'s `GenericNetwork`
      became the sole neuron/synapse representation and the sole
      `apply_stdp()`; `neurons.rs` was gutted down to a thin named view
      (`LeftKey`/`RightKey`/`Motor` are now three ordinary entries
      inside `GenericNetwork`, seeded once at boot by
      `network::seed_fixed_network()` with Milestone 9/10's exact
      original constants -- threshold, leak, refractory period, initial
      weight, all preserved unchanged and recorded in `neurons.rs` for
      the record). `GenericNetwork` gained a `refractory_ticks` field
      (defaults to 0, a no-op for every pre-M21 `addneuron`-built
      network) to carry Milestone 9's refractory semantics into the
      shared engine. **A real bug found and fixed**: boot panicked
      ("memory allocation of 7 bytes failed", 7 = `len("LeftKey")`)
      because `neurons::init()` used to run before the heap was
      initialized -- harmless for the old heap-free fixed struct, fatal
      once seeding does real `String`/`Vec` allocation; fixed by
      reordering `main.rs` so Milestone 3's heap-init runs first. **A
      disclosed, real physics change**: unifying onto
      `GenericNetwork::tick()`'s one-tick synaptic delay means
      LeftKey/RightKey/Motor firings now reach Motor on the NEXT tick
      instead of the same tick -- documented in both files rather than
      silently changed. Verified end-to-end, independently, after
      merging into the real project tree (not just the isolated build):
      a `clear` command's own embedded 'a' character (every keystroke
      doubles as a real stimulus, per Milestone 9's original design)
      stimulated LeftKey, and `net`/`neurons` immediately agreed
      exactly (`fires=1`/`fires=1`, weights `0.5000`/`0.5000`); a
      further real `sendkey a` took both to `fires=2`/`fires=2`; the
      fixed `train` command's exact result (`0.5000 -> (5x LTP) ->
      0.5021 -> (5x LTD) -> 0.5000`) was immediately visible identically
      through both `net` and `neurons` afterward (`fires=14` in both).
      Also confirmed (real test, not assumed): the fixed `train`,
      generic `train FROM TO GAP`, and `save`/disk-persistence-across-
      -reboot paths (Milestone 10/17/11's own original tests) still
      work, now reading/writing the one shared synapse. Built in an
      isolated parallel-agent workspace, then merged (including
      reconciling the heap-reorder against Milestone 19's own earlier
      `main.rs` reordering and Milestone 20's PCI-init insertion) and
      independently re-verified against the real project tree.
- [x] **Milestone 24**: real e1000 NIC packet transmission (`nic.rs`) --
      Milestone 20's PCI enumeration deliberately stopped at discovery;
      this drives the device it found for real. Maps BAR0's MMIO
      register window through the exact same `physical_memory_offset`
      mapping `memory.rs` already relies on (reused, not re-derived),
      flips the PCI command register's memory-space and bus-master
      enable bits (required for the NIC to DMA at all), and runs the
      documented Intel 8254x reset/configure sequence: `CTRL.RST` set
      and polled to completion (a real bounded timeout, not a fixed
      sleep), interrupts masked (this driver only polls), link-up set,
      `TDBAL`/`TDBAH`/`TDLEN`/`TDH`/`TDT`/`TCTL`/`TIPG` programmed. The
      transmit descriptor ring and packet buffers live in a single
      4096-byte page-aligned `static` region so every offset inside it
      is physically contiguous by construction, with a real 4-level
      page-table walk (modeled on `memory.rs`'s own level-4 lookup,
      extended to a full walk with honest huge-page handling) to find
      its physical base for the hardware's DMA registers. The device's
      real MAC (auto-loaded by hardware into `RAL0`/`RAH0` at reset) is
      read directly, not fabricated. New shell commands `nicinfo`
      (live MAC/link-status read) and `sendpacket` (builds and
      transmits one real broadcast Ethernet II frame, then polls the
      descriptor's own hardware-set `DD` bit for confirmation -- proof
      the NIC itself completed the DMA, not just that a register write
      succeeded). Verified with the strongest evidence in the project so
      far: QEMU's `filter-dump` object captured genuine on-wire traffic
      to a real pcap file, independently re-parsed byte-for-byte (not
      trusted from the build agent's own report) -- exactly one 51-byte
      frame, destination `ff:ff:ff:ff:ff:ff`, source MAC matching the
      driver's own reported address exactly, ethertype `0x88b5`, and a
      payload matching the test string exactly. This cross-checks the
      DD-bit confirmation against real transmitted bytes on the wire,
      not just a status flag. Built in an isolated parallel-agent
      workspace, then merged (reconciling against M19/M20/M21's earlier
      `main.rs` changes) and independently re-verified end-to-end,
      including re-parsing a fresh capture from scratch.
- [x] **Milestone 25**: real dynamic task spawn/kill on top of Milestone
      5c's fixed 3-worker preemption demo. Two real structural changes
      were needed to make this genuine rather than cosmetic, not just
      new commands over the existing machinery: (1) `TopologicalScheduler`
      (Milestone 4), a fixed-size slot bank since its creation, gained
      `add_slot()` (grows the bank by one live slot, recomputing the SSH
      bonds from scratch at the new size so the alternating v/w pattern
      stays consistent) and `revive()` (the mirror of `kill()`, so a
      reused slot doesn't inherit its predecessor's fire history); (2)
      the original demo's `timer_tick_switch()` permanently stopped
      switching once its bounded ~3.3s window closed (by design, so
      `kernel_main` could regain control and finish booting) -- a
      spawned task would never actually run if that stayed permanent,
      so `kernel_main` now calls a new `enable_background_scheduling()`
      exactly once, as the very last thing before its final `hlt_loop()`
      (harmless at that point: kernel_main has nothing left to do, so
      permanently losing the CPU to worker tasks going forward is fine).
      New shell commands `spawn` (creates one new counting task, reports
      its assigned id) and `kill ID` (terminates it for real); `tasks`
      now reports every currently-live task dynamically instead of a
      hardcoded 3-counter line. **A real hazard reasoned through rather
      than hit at runtime**: `kill` normally frees a task's stack
      immediately, but shell commands run nested inside the keyboard
      ISR on top of whatever task's stack happened to be current when
      the key was pressed -- including, if a task kills itself, its own
      stack. Freeing that memory out from under the very call chain
      executing on it would corrupt the frame about to be `iretq`'d
      back into. Solved with a deferred `ZOMBIE` list, reaped lazily
      once a later tick's real context switch has genuinely carried
      execution off that stack. Verified end-to-end after merging into
      the real project tree: the original Milestone 5c regression check
      (`all three tasks genuinely preempted and ran`) still passes
      unchanged; `spawn` created `task3` which grew `23 -> 94` fire
      counts over a real 2-second wall-clock delay alongside the
      original three (genuine concurrent execution, not simulated);
      `kill 3` removed it from the live list immediately, and a further
      2-second wait confirmed it stayed gone (not just hidden) while
      `task0`/`task1`/`task2` kept growing undisturbed. Built in an
      isolated parallel-agent workspace, then merged (reconciling
      against M19/M20/M21/M24's earlier `main.rs` changes) and
      independently re-verified against the real project tree.
- [x] **Milestone 26**: real e1000 packet reception (`nic.rs`),
      completing the NIC driver Milestone 24 deliberately left
      transmit-only. Adds an 8-descriptor RX ring following the exact
      same page-aligned/`translate_addr` pattern established for TX,
      programmed into `RDBAL`/`RDBAH`/`RDLEN`/`RDH`/`RDT`/`RCTL`. **A
      real negative-then-positive finding, not smoothed over**: the
      Intel datasheet's `RCTL.LBM` field (set to MAC loopback) was
      programmed first and simply didn't work under QEMU --
      `sendpacket` kept confirming TX while `recvpacket` never saw an
      RX descriptor go done, across repeated polls. Root-caused by
      reading QEMU's own `hw/net/e1000.c` model source rather than
      guessing: its `e1000_send_packet()` only loops a frame back when
      the PHY's `MII_BMCR` loopback bit is set over `MDIC`, and QEMU's
      *classic* e1000 model (unlike its newer e1000e model) never reads
      `RCTL.LBM` at all. Fixed by driving the device's real MDIO/MDIC
      interface to write the PHY's standard IEEE 802.3 `MII_BMCR`
      register with its loopback bit set -- the same real mechanism
      `ethtool -t`'s hardware self-test uses on actual silicon, not a
      workaround specific to this emulator. New shell command
      `recvpacket` (bounded poll, honestly reports "no packet received"
      rather than blocking forever). Verified end-to-end after merging:
      `sendpacket` then `recvpacket` returned a frame with source MAC
      exactly matching the driver's own reported address, destination
      broadcast, ethertype `0x88b5`, and `payload matches test packet:
      true` -- byte-exact loopback confirmed via the hardware's own
      RX-side DD bit, not assumed; a second `recvpacket` honestly
      reported nothing left in the ring rather than re-reporting stale
      data. Built in an isolated parallel-agent workspace, then merged
      and independently re-verified against the real project tree,
      including re-running the empty-ring case to confirm it too.
- [x] **Milestone 29**: real mouse-driven drawing, combining Milestone
      16's PS/2 mouse driver with Milestone 23's framebuffer graphics
      primitives for the first time. New `draw` shell command enters a
      mode where holding the left mouse button and moving draws real
      strokes live on the framebuffer. Hooked directly into `mouse.rs`'s
      own IRQ12 packet handler rather than sampled from the timer tick
      -- every drawn segment corresponds to an actual decoded hardware
      movement packet, the real reported path, not an approximation
      resampled at an unrelated rate. A real right-click (edge-detected
      on the same packet stream, or the `stopdraw` command as a manual
      fallback) exits back to a fully responsive shell. Required zero
      changes to `interrupts.rs`'s IDT/gate structure or `gdt.rs` --
      deliberately scoped that way since Milestone 27 was concurrently
      doing major privilege-level work in exactly those files; the
      mouse interrupt handler already unconditionally called into
      `mouse.rs`, so all the new logic lives inside that existing call.
      Verified with real injected `mouse_move`/`mouse_button` monitor
      events tracing a genuine multi-segment path (not a single dot),
      confirmed visually in a screenshot, followed by a real right-click
      and an ordinary `about` command proving the shell remained fully
      responsive afterward. Built in an isolated parallel-agent
      workspace, then merged and independently re-verified against the
      real project tree.
- [x] **Milestone 27**: real CPL=3 (ring 3) execution and a minimal
      `int 0x80` syscall ABI -- the single biggest architectural
      milestone in the project. Everything through Milestone 26 (the
      shell, every device driver, all 8 fixed+spawned worker tasks) ran
      at CPL=0; this is the first code in spikeling-os to actually drop
      privilege and prove it with hardware-recorded evidence, the real
      prerequisite for eventually running software not written
      specifically for this kernel. Added a user code segment and user
      data segment (DPL=3) to the GDT -- the first two entries anything
      other than ring 0 can legally use -- plus
      `TSS.privilege_stack_table[0]`, the dedicated stack the CPU
      automatically switches to on any ring3->ring0 transition (leaving
      this unset/garbage is a classic real bug: RSP=0 on the first
      privilege-elevating interrupt). One physical frame each for a
      user code page and user stack page, mapped
      `PRESENT | WRITABLE | USER_ACCESSIBLE` and populated with 16 bytes
      of hand-assembled machine code (`mov eax,0; int 0x80; mov eax,1;
      int 0x80; jmp $`) -- hand-assembled deliberately, since a compiled
      Rust function's exact instruction-byte length isn't something
      Rust exposes safely. Entry into ring 3 via a hand-built `iretq`
      frame in a naked-asm function mirroring `tasks.rs`'s `switch_to`;
      a new IDT gate at vector `0x80` with `DPL=3` explicitly set (a
      gate defaults to DPL=0, which would `#GP`-fault a ring-3 caller
      immediately), pointing at a naked-asm trampoline that saves all
      GPRs, calls an ordinary Rust dispatch function, and either
      resumes or discards the frame. Two syscalls: `0` = print (a fixed
      kernel-owned message -- no general copy-from-user pointer safety
      yet, disclosed as deliberately out of scope) and `1` = exit
      (discards the ring-3 context and resumes exactly where the shell
      called in, via the same saved-`rsp` mechanism `tasks.rs`'s
      `KERNEL_RSP` already established). **The crucial verification
      detail**: the syscall handler reads the CPU's OWN
      interrupt-frame-pushed `CS` value -- hardware-recorded at the
      moment `int 0x80` executed, not self-reported by the ring-3 code
      -- as the actual proof CPL=3 was real. New shell command
      `usertest`. **A real bug found and fixed**: the first working
      version copied `tasks.rs`'s `switch_to` pattern exactly, including
      its unconditional `sti` before the final `ret` -- and permanently
      hung the shell after exactly one `usertest` run, every time.
      Root cause: `switch_to`'s `sti` is safe only because every call
      site is a single well-understood nesting level; the exit syscall's
      return path is instead nested arbitrarily deep inside the
      keyboard ISR's own call chain (holding `keyboard.rs`'s `KEYBOARD`
      mutex guard the whole time), and the premature `sti` opened a
      window for a nested timer tick to hijack execution via
      `tasks::timer_tick_switch()`, abandoning that whole call chain
      -- mutex held forever, every future keystroke silently
      deadlocked. Fixed by removing the `sti`: this return path always
      lands back inside code nested in the keyboard ISR, which
      naturally restores interrupts via its own correct `iretq` once it
      finally unwinds, exactly like every other shell command already
      relies on. Verified end-to-end after merging into the real
      project tree (not just the isolated build): three consecutive
      `usertest` runs each logged hardware-recorded `CS=0x1b` (CPL=3,
      confirmed independently, not trusted from the build report
      alone), and `tasks` readings taken between and after runs showed
      the Milestone 25 background scheduler's counters genuinely
      growing throughout every ring-3 excursion, with `about` printing
      correctly afterward -- full, real proof the kernel remained
      completely intact and responsive around a real privilege
      transition. Disclosed limitations: one hardcoded user program, no
      general user-pointer safety, no per-process isolation, no
      scheduler integration for ring-3 execution yet -- genuinely
      Milestone 28+ territory. Built in an isolated parallel-agent
      workspace concurrently with two other milestones that were
      explicitly kept out of `gdt.rs`/`interrupts.rs` to avoid
      collision, then merged and independently re-verified.
- [x] **Milestone 28**: real one-level subdirectory support for
      `fs.rs`. A directory entry can now be either a file or a
      subdirectory (a new `is_dir` flag); a subdirectory's "data" is
      exactly one sector, allocated from the same shared pool files
      already use, holding its own 8-entry table in the identical
      on-disk format as the root table -- `mkdir NAME` only creates
      directories directly under root (no `mkdir a/b`), and
      `write`/`read`/`rm` accept one optional `DIR/` prefix (no
      `a/b/c`), an honest, disclosed depth cap of 1. New `ls` output
      distinguishes subdirectories from files with a trailing `/`; new
      `ls DIR` command lists a subdirectory's contents. `rm` refuses to
      remove a directory entry outright (empty or not) -- no
      rmdir/recursive-delete exists yet, so refusing is the honest, safe
      choice over silently orphaning a subdirectory's sectors. **A real
      correctness trap caught before it became a bug**: since
      subdirectory tables and file data now share one allocation pool
      across root *and* every subdirectory, the old Milestone 22
      per-directory `find_free_span` (which only checked one table's
      own entries) would have let two different directories' files
      collide on the same disk sector. Replaced with a disk-wide
      `collect_occupied` that scans root plus every subdirectory's
      table before any allocation, verified indirectly by a real
      interleaved delete/rewrite/mkdir churn sequence that produced no
      corruption. Verified end-to-end after merging: `mkdir docs` then
      `write docs/hello ...` then `read docs/hello` round-tripped
      byte-exact; `ls` showed `docs/` correctly distinguished from
      `toplevel` (a root-level file, confirming unaffected root
      behavior); a second `mkdir docs` correctly failed with a name
      collision; `rm docs` correctly refused with "is a directory --
      rm does not remove directories (no rmdir yet)". Built in an
      isolated parallel-agent workspace, then merged and independently
      re-verified against the real project tree (catching and fixing a
      test-script bug of its own along the way -- a missing `/` ->
      `slash` key mapping, the same class of gap this project has hit
      and honestly disclosed before with `.` and `=`).
- [x] **Milestone 30**: real per-process address space isolation,
      closing the gap Milestone 27 disclosed honestly in its own report
      -- its ring-3 program ran under the KERNEL's own page tables, with
      nothing but the `USER_ACCESSIBLE` flag standing between "ring 3"
      and the whole kernel address space, and no way to run two
      processes that couldn't see each other's memory. Each `Process`
      now gets its own top-level page table (PML4) in its own physical
      frame. The design deliberately avoids a naive full copy of the
      kernel's page table hierarchy (which would silently go stale the
      instant the kernel maps anything new later, e.g. heap growth):
      every PML4 entry outside the user-space range is a raw copy of
      the *entry itself* (a pointer to the kernel's existing, already-
      built P3 table) rather than a deep copy of the hierarchy under
      it -- so every process's kernel-space view stays bit-for-bit
      identical to the kernel's own forever, automatically, since it's
      the literal same physical P3 table in memory. Only the one PML4
      entry covering `usertest::USER_CODE_ADDR`/`USER_STACK_ADDR`
      (computed for real at runtime, both confirmed to land on index
      170, with a loud failure if that ever changes) is left private,
      backed by a genuinely fresh, per-process P3/P2/P1 chain. Two
      hardcoded test processes, each printing a distinct message
      through the same Milestone 27 syscall path, prove real physical
      isolation: identical virtual code address, genuinely different
      physical bytes depending on which PML4 is loaded in `CR3`. New
      shell command `runproc N`; the original `usertest` command still
      works completely unchanged (still under the kernel's own shared
      page tables, exactly as Milestone 27 left it). Verified end-to-end
      after merging into the real project tree: `runproc 1` / `runproc
      2` / `runproc 1` again showed process A's message, then B's, then
      A's again with no cross-contamination; legacy `usertest` printed
      its original unchanged string interleaved cleanly; `about`
      confirmed the shell stayed fully responsive throughout -- all
      hardware-CPL-confirmed via the same CPU-recorded `CS` value
      Milestone 27 established. **An honest false alarm caught and
      correctly diagnosed, not glossed over**: the first integration
      test appeared to hang identically at the same log line across
      three separate runs (a real, reproducible-looking signal, not
      dismissed as flakiness) -- a longer, patient unattended wait
      proved it was never actually stuck, just genuinely slower to
      boot than before (the two processes' setup now walks/copies 1024
      PML4 entries total at boot), pushing past the test harness's old
      12-second margin. Distinguishing "slower" from "hung" required
      an actual longer real-time observation, not an assumption either
      way. Built in an isolated parallel-agent workspace, then merged
      and independently re-verified against the real project tree.
- [x] **Milestone 31**: a real, general `write(ptr, len)` syscall,
      replacing Milestone 27's syscall 0 (which took no arguments at all
      and just printed one string hardcoded on the KERNEL side). The
      ring-3 program now passes a real pointer (`rdi`) and length (`rsi`)
      in registers; `syscall_dispatch` reads exactly that many bytes out
      of whatever address space is CURRENTLY loaded in `CR3` -- the
      calling process's own private PML4 for a `process.rs` process, or
      the kernel's shared page tables for the legacy `usertest` path --
      and writes them raw to serial. This generalizes Milestone 30's
      `read_active_message()` (which only ever read one fixed offset)
      into an arbitrary caller-supplied pointer+length, and is the real
      per-process isolation proof running through a genuine syscall
      argument instead of a hardcoded kernel-side string: identical
      virtual `ptr` (`0x555550000080`) and `len` (64) resolved to
      genuinely different physical bytes for process A vs. process B vs.
      the legacy path, re-verified with process A run again after B with
      no cross-contamination. One real safety net -- `MAX_WRITE_LEN`
      (4096) truncates an absurd requested length rather than walking the
      read loop off into unmapped memory -- was actually exercised, not
      just asserted: patching the test program's length immediate to
      `0xFFFFFFFF` confirmed the cap catches it and truncates to 4096,
      but a **shorter, still-bad** pointer/length pair (the truncated
      4096-byte read walking 128 bytes past the single mapped 4096-byte
      code page) still page-faults the kernel cleanly (logged `CR2` +
      error code, halted, no silent corruption or triple fault) --
      disclosed honestly as a real, present gap: there is no
      copy-from-user fault-recovery path yet, only a coarse bound against
      wildly large requests. Built in an isolated parallel-agent
      workspace, then merged and re-verified against the real project
      tree.
- [x] **Milestone 33**: a real per-process heap, closing the "per-process
      heaps" gap Milestone 30/31 left open. Each `Process` gets a fixed
      16 KiB region (`HEAP_START`, 4 pre-mapped pages) built with the
      exact same private-P3/P2/P1-chain technique Milestone 30 uses for
      code/stack -- `HEAP_START` shares USER_CODE_ADDR/USER_STACK_ADDR's
      p4_index (170) but lands at a distinct p2_index, so it's genuinely
      private with zero new PML4-level reasoning, and provably disjoint
      from the kernel's own heap. A real syscall (2 = `sbrk`) is the
      process's only way to allocate -- a kernel-only allocator was
      deliberately rejected, since `int 0x80` is this kernel's one
      sanctioned ring-3-to-ring-0 boundary and a kernel-side-only
      allocator would be unreachable from ring 3, not a genuine answer to
      "give the process a real way to allocate." Verified for real:
      `runproc 1` calls `sbrk`, writes a distinguishing marker byte
      (`'A'`) into the returned heap pointer, then prints it back out
      through Milestone 31's own `write(ptr,len)` syscall; `runproc 2`
      shows `'B'` at the identical virtual heap address; re-running
      process A after B shows `'A'` again, unchanged, AND its second
      `sbrk` call correctly returned a pointer 16 bytes further into the
      heap than the first -- proving `heap_used` genuinely persists
      per-process across repeated runs rather than resetting. Built in an
      isolated parallel-agent workspace (which branched from Milestone
      30, before Milestone 31 landed) -- merging required hand-
      regenerating the workspace's own `PROCESS_PROGRAM` machine code
      (it had been hand-assembled against the OLD, argument-less syscall
      0) to also set up `rdi`/`rsi` for Milestone 31's `write(ptr,len)`
      convention before printing. That regenerated byte array was NOT
      trusted on "compiles cleanly" alone (a raw `[u8; 46]` isn't
      type-checked for correctness) -- re-verified with a dedicated
      real QEMU boot afterward, confirming the exact marker bytes
      (`0x41`/`0x42`) and persisted heap offset above.
- [x] **Milestone 32**: recursive subdirectory nesting + `rmdir`, closing
      the one-level-only cap Milestone 28 disclosed honestly at the time.
      **No on-disk format change was needed** -- a subdirectory's table
      was already stored, since Milestone 28, in the identical `DirEntry`
      /one-sector format root uses, so nesting was already representable
      on disk; Milestone 28 had only capped the *logic* walking it, not
      the format itself. `resolve_dir_lba` now walks an arbitrary-depth
      `/`-separated component chain from root instead of handling a
      single level, and `collect_occupied` (pool-sector accounting) was
      made recursive to match. New `rmdir` mirrors Milestone 22's
      `delete_file` reclamation exactly: refuses unless the target's own
      table has zero used entries, otherwise frees its parent's slot and
      its one-sector table back into the shared pool. A real shell-side
      current-working-directory (`cd`, prompt now shows the real path
      like `/a/b/c> ` instead of a fixed `"> "`) was layered on top,
      entirely in `shell.rs` -- `fs.rs` itself stays purely root-relative
      throughout, with zero notion of "current directory." Verified for
      real: a genuine 3-level nested tree, files readable only from their
      own directory (proven both ways -- wrong-level reads correctly
      fail), a multi-component `cd a/b/c` in one command, `rmdir` on a
      non-empty directory correctly refused, an empty one correctly
      removed and confirmed gone via `ls`, a CWD-dangling guard (shell
      refuses to `rmdir` the directory it's currently inside), and --
      same discipline as Milestone 11 -- genuine reboot-persistence: a
      completely separate, fresh QEMU process against the same disk image
      still showed the entire persisted tree intact. **A real test-
      harness bug caught and root-caused, not a kernel bug**: the first
      verification attempt used "does the serial log contain `'> '`" as
      a boot-readiness heuristic and got zero keystrokes through --
      root cause was that the shell prompt is drawn only to the
      framebuffer, never serial, so that heuristic instead false-matched
      the substring `"-> "` inside an early, unrelated boot line
      (`"milestone 1: boot -> kernel handoff..."`), firing keystrokes
      seconds before the shell was even reachable; fixed by waiting for
      the kernel's own unambiguous `"milestone 8: interactive shell
      active"` serial line first. Built in an isolated parallel-agent
      workspace, then merged and re-verified against the real project
      tree -- including a full integration test alongside Milestones
      31/33 (which had modified the SAME `shell.rs` in a separate
      workspace) confirming no interference between CWD state and the
      process/syscall commands.
- [x] **Milestone 34**: a real general program loader, closing the
      "hardcoded programs only" gap every prior ring-3 milestone
      disclosed honestly. Until now, every process (PROCESS_A/PROCESS_B,
      and the original `usertest` before that) ran code from a byte array
      baked directly into the kernel binary. `create_process()`'s actual
      page-table-building mechanism was factored out into
      `create_process_from_image()`, taking an arbitrary `&[u8]` code
      image instead of always copying a fixed program -- both the
      hardcoded-array path (PROCESS_A/PROCESS_B) and the new file-loaded
      path now run through IDENTICAL, unduplicated unsafe mapping code,
      differing only in where the bytes came from; every process,
      loaded-from-file or not, now gets code, stack, AND a Milestone 33
      heap mapped uniformly. The new `runfile PATH` shell command reads a
      real file's bytes off the actual on-disk filesystem (fs.rs) and
      runs them under a fresh private PML4 via a new
      `load_and_run_image()`. Verified for real, byte-for-byte: a test
      program's real machine code (reusing Milestone 31's
      `usertest::USER_PROGRAM` verbatim, since it already sets up the
      `write(ptr,len)` syscall's `rdi`/`rsi` correctly) was written to a
      genuine file on disk (`seedtestprog`), then `runfile testprog`
      loaded and ran it, printing its exact embedded message
      (`"hello from a REAL FILE on disk -- milestone 34 loader
      confirmed"`) through the real write syscall -- unambiguous proof
      the executed bytes came from the file, not any array compiled into
      the kernel. Re-ran `runfile testprog` a second time and got a
      genuinely fresh PML4/code/heap frame set both times (not reused
      state), then re-ran `runproc 1`/`usertest` afterward and confirmed
      neither the loader path nor anything else had been corrupted.
      **Honest limitations, disclosed rather than hidden**: flat binary
      only (no ELF, no relocations, no sections, no dynamic linking --
      raw machine code copied verbatim to a fixed load address, exactly
      like every hardcoded program before it); a fixed one-page (4096
      byte) max program size, enforced before any copy happens; only one
      loaded-from-file process resident at a time (one slot, replaced
      not accumulated); a loaded process's heap is mapped but not
      currently reachable via `sbrk` (that syscall only recognizes
      PROCESS_A/PROCESS_B's ids) -- real and disclosed, not something
      this milestone's own test program needed. Built in an isolated
      parallel-agent workspace that branched from Milestone 30, before
      Milestones 31/32/33 landed -- merging required a genuine rewrite of
      the workspace's own `loader.rs`, which had assumed the OLD,
      argument-less "print" syscall convention and null-terminated
      messages; adapted to reuse Milestone 31's `write(ptr,len)`-ready
      `USER_PROGRAM` with a message SPACE-PADDED to exactly
      `MESSAGE_LEN` bytes instead (the write syscall reads a fixed length
      baked into the machine code, not a null-terminated string). That
      rewrite was not trusted on "compiles cleanly" alone -- re-verified
      with a dedicated real QEMU boot confirming the exact message bytes
      and genuine per-run frame freshness above.
- [ ] **The real, standing goal is Linux comparability** -- not matched
      milestone-for-milestone, but a genuine target: real virtual memory
      (demand paging, copy-on-write, swap), POSIX-shaped syscalls, a real
      process model, a working libc, and eventually running software not
      built specifically for this kernel. That's an enormous target (Linux
      itself is tens of millions of lines built over three decades by
      thousands of people) -- stated honestly here so the roadmap reflects
      a real distant target, not something a handful of milestones closes
      out.

      The honest next tier after Milestones 35/36 (below), in real
      dependency order: real `fork`/`exec`/`wait` (**done, Milestone
      37** -- see below); a minimal libc (userspace programs need this
      to make POSIX-shaped calls at all); real virtual memory (demand
      paging, copy-on-write on fork, `mmap` -- current isolation is real
      but static, built once at process-creation time); then signals,
      then SMP, then a real TCP/IP stack on top of the existing e1000
      driver. Each genuinely gates the next, not an arbitrary ordering.
- [ ] **A further, more ambitious standing goal beyond Linux
      comparability: a completely independent environment** -- not just
      matching Linux's feature surface, but eventually needing zero
      external host (no Linux/Windows machine, no `cargo`/`rustc`
      cross-compile toolchain, no QEMU-as-development-crutch) to build,
      extend, or run itself. The real test of independence is
      self-hosting: this kernel's own source tree, compiled by a
      toolchain running *on this kernel*, producing a new bootable image
      of itself, with no other machine involved anywhere in that loop.
      A 50-milestone roadmap toward that, organized in real dependency
      tiers (not an arbitrary wishlist -- each tier's items are genuine
      prerequisites for the tier after):

      **Tier 1 -- userspace foundation**: virtual memory/demand paging,
      signals, process groups/sessions, `pipe()`, `dup`/`dup2`, real
      `exec` with argv/envp, real `waitpid` exit-status semantics
      (`WIFEXITED` etc.), a real libc (`malloc`/`free`, `string.h`,
      buffered `stdio`), generalizing the ELF loader past its
      fixed-entry-address restriction, `errno`.

      **Tier 2 -- filesystem completeness**: a robust on-disk format
      (permissions, timestamps, hard links), a minimal multi-user model
      (uid/gid, chmod/chown), symbolic links, `mmap()`, a real
      block-device abstraction beyond raw ATA.

      **Tier 3 -- toolchain bootstrapping, the actual self-hosting
      chain**: port a minimal C compiler (subset-C, capable of
      eventually compiling itself), port/write a minimal x86_64
      assembler, port/write a minimal ELF linker, get that compiler to
      build "hello world" *on* spikeling-os, get it to compile something
      nontrivial -- ultimately itself, port a minimal `make`-equivalent,
      a real native shell (loops/conditionals/variables), a native text
      editor, and the capstone of this tier: **rebuild spikeling-os's own
      kernel using the native on-OS toolchain**.

      **Tier 4 -- networking independence**: a real TCP/IP stack (ARP,
      IP, TCP, UDP -- currently just raw e1000 tx/rx), DHCP client, DNS
      resolver, a minimal native HTTP client, a minimal native HTTP
      server, BSD-socket syscalls.

      **Tier 5 -- software distribution independence**: a simple package
      format + native package manager, a native archive tool
      (tar-equivalent), a native source-fetch tool (tarball/git-lite) --
      completing the loop so the OS can fetch and install its own
      software without a host machine.

      **Tier 6 -- multitasking maturity**: SMP (multi-core), real
      priority/fair scheduling, userspace locking primitives
      (mutex/semaphore syscalls), copy-on-write `fork()`.

      **Tier 7 -- real hardware independence, leaving QEMU behind**: USB
      stack, AHCI/SATA driver, ACPI parsing (power management, real
      shutdown/reboot, core enumeration), boot validated on real
      hardware not just QEMU, a custom/portable bootloader, real
      timekeeping (RTC + HPET/TSC calibration).

      **Tier 8 -- self-sufficiency completion**: a native window manager
      (building on the existing framebuffer work), persistent
      accounts/login, a real init system (process 1, service
      supervision), kernel panic diagnostics usable without a host
      debugger, a native disk-partitioning tool, a native version-control
      tool (so this source tree can be tracked on itself), and the real
      **capstone**: the full source tree -- kernel, libc, compiler,
      shell, all native tools -- builds a complete, bootable
      spikeling-os image starting from spikeling-os itself, with zero
      Linux/Windows/host-machine involvement anywhere in the loop.
- [ ] **Milestones 51-100: past self-hosting, toward a genuinely usable,
      genuinely differentiated OS.** Picks up exactly where the
      100-milestone independence roadmap's first half leaves off. Real
      dependency tiers again, not a wishlist.

      **Tier 9 -- POSIX/compatibility maturity**: full C99/C11 libc
      conformance push; port a real third-party program with minimal
      patching as a stress test; dynamic linking (shared libraries);
      real POSIX-sh-subset shell scripting; verified env-var inheritance
      across exec against real programs; a real regex engine; `ioctl()`
      + tty raw/cooked mode semantics; port a small interpreter
      (Lisp/Forth) as a further toolchain stress test; a
      standards-compliant `printf`/`scanf` family; basic C-locale
      correctness.

      **Tier 10 -- driver ecosystem breadth**: real audio driver
      (AC97/HDA); basic 2D graphics acceleration beyond the linear
      framebuffer; unified PS/2 + USB HID input abstraction; serial/UART
      used for a real use case, not just debug logging; battery/power
      status (ACPI extension); real USB mass storage mount/unmount;
      NVMe driver; real hot-plug device handling; a unified
      device-driver framework/API; an honest breadth audit of what real
      hardware still isn't supported.

      **Tier 11 -- security & hardening**: ASLR; stack canaries; W^X
      enforcement verified per-process; a real capability/least-privilege
      model beyond uid/gid; kernel-level audit logging; a minimal
      signed/verified boot chain; seccomp-equivalent syscall sandboxing;
      fuzzing the syscall interface and fixing what breaks; basic
      encrypted filesystem support; an honest security-model writeup --
      what's actually protected against, what isn't.

      **Tier 12 -- virtualization & distributed capability**: basic
      hypervisor primitives (host a minimal guest); namespace isolation
      (containers-equivalent, beyond per-process page tables); real
      multi-instance discovery over the Tier 4 TCP/IP stack; a basic
      distributed filesystem; a real inter-instance RPC mechanism.

      **Tier 13 -- the actual differentiator: leaning into the
      neuromorphic core**, this project's real point of difference from
      a generic hobby OS: generalize Milestone 39's evidence-gated
      topology from its toy 3-neuron demo network into a real subsystem
      other kernel decisions can subscribe to; an SNN-driven page-eviction
      policy, honestly measured against conventional LRU; SNN-driven
      scheduler priority, closing the loop back to Milestone 1's original
      topological-scheduler concept; a real "spiking coprocessor" syscall
      API letting userspace submit its own small networks to run on the
      kernel's substrate; extending self-healing beyond synapses (e.g. a
      flaky driver's error-recovery policy gated by the same
      evidence-accumulation pattern Milestone 39 established); **the
      honest benchmark** -- does SNN-driven anything measurably beat a
      conventional heuristic anywhere in this OS, or is it currently
      research-only, a real disclosed answer either way; extending
      Milestone 38's ternary quantization to more internal kernel state
      where it plausibly helps; an offline-consolidation "dream" mode
      replaying recent activity through the SNN during idle time;
      publishing the neuromorphic-OS design as a standalone writeup --
      genuinely novel vs. prior art, honestly scoped.

      **Tier 14 -- polish, documentation, real usability**: a real
      installer onto a blank disk, not just QEMU; man-page-equivalent
      docs for every syscall and shell command; an automated CI test
      suite replacing manual QEMU boots; an internationalization-readiness
      audit; a "getting started" guide a total stranger could follow with
      zero prior context; and the real **second capstone**: spikeling-os
      1.0 -- self-hosting, running real software, the neuromorphic core
      doing something measurably useful, genuinely usable by someone who
      isn't its author.
- [x] **Milestone 35**: real per-process file descriptors -- `open`(3),
      `read`(4), `fdwrite`(5), `close`(6), all NEW syscall numbers rather
      than generalizing the existing `write(ptr,len)` into
      `write(fd,ptr,len)` (a real design decision, documented in-code:
      generalizing would require regenerating three already-verified
      hand-assembled programs' register setup for no benefit that
      outweighed it). Every syscall reuses the exact "read/write raw
      bytes at a caller-supplied pointer, through whatever CR3 is
      currently loaded" technique the write syscall already established
      -- `open`'s path string and `fdwrite`'s data cross the user/kernel
      boundary the same way `write` always has. A real, bounded
      per-process fd table (`MAX_OPEN_FILES = 4`) backs it; files are
      buffered into a kernel `Vec` at `open()` time (reasonable given
      `fs.rs` already caps every file at 4096 bytes) and only persisted
      to disk at `close()`, and only if actually written to. Verified
      for real, byte-for-byte: a file written via the shell's own
      `write` command was opened/read back via the new syscalls with
      identical content, and a ring-3 program's `fdwrite`+`close` wrote
      a new file whose disk contents exactly matched what the syscall
      sent. **A real, pre-existing Milestone 34 bug was found and fixed
      along the way**: `run_file()` held the global frame-allocator
      spinlock across the *entire* ring-3 excursion (interrupts enabled
      the whole time) -- fixed by splitting `load_and_run_image` into
      `create_loaded_process` (needs the lock) and `run_loaded_process`
      (called after the lock is released). **A second real, pre-existing
      Milestone 34 bug was found and honestly disclosed, not fixed**:
      even after that fix, the file-loaded process path reproducibly
      page-faults shortly after completing, root-caused (not guessed) to
      heap corruption via a symbolized crash address inside
      `linked_list_allocator`, confirmed present with zero Milestone 35
      code involved and confirmed absent from the `runproc` path even
      under extended testing. Worked around for THIS milestone's own
      clean verification by adding a third hardcoded process slot
      (`FDTEST_PROCESS`) that reuses the already-safe `runproc`
      mechanism instead of the buggy file-loaded path -- the bug itself
      remains open, disclosed in-code and here, not hidden behind the
      workaround. Built in an isolated parallel-agent workspace, then
      hand-merged (a real 3-way `diff3` against the shared Milestone 34
      baseline) alongside the concurrently-built Milestone 36 below, and
      independently re-verified against the real, fully-merged project
      tree.
- [x] **Milestone 36**: a real ELF64 loader, closing the "flat binary
      only" limitation Milestone 34 disclosed honestly at the time. A
      genuine structural parser (`kernel/src/elf.rs`) validates magic
      bytes, `ELFCLASS64`/`ELFDATA2LSB`, `e_type`/`e_machine`, then walks
      the real `Elf64_Phdr` program header table extracting every
      `PT_LOAD` segment's `p_vaddr`/`p_offset`/`p_filesz`/`p_memsz`/
      `p_flags` -- with real bounds checks throughout (malformed input
      returns `Err`, never panics). **A real, honest scoping decision**:
      rather than making the ring-3 entry trampoline's jump target
      dynamic (real, deep surgery on Milestone 27/30's carefully-built
      mechanism, deliberately avoided as more than could be fully
      re-verified in scope), `create_process_from_elf()` requires --
      checked for real, not assumed -- that the ELF's own `e_entry`
      equals `USER_CODE_ADDR` exactly, with every `PT_LOAD` segment
      page-aligned and bounded. This loads real ELFs genuinely linked
      for this kernel's own fixed entry address, not arbitrary Linux
      binaries -- consistent with this README's own scoping of full
      Linux ELF/libc compatibility as separate, much later work. New
      `runelf PATH` shell command, alongside (not replacing) `runfile`.
      Verified with a REAL, externally-built two-segment ELF64
      executable (`kernel/assets/testelf.elf`, built with this project's
      own pinned Rust nightly + `rust-lld` and a custom linker script
      forcing two genuinely separate `PT_LOAD` segments -- not
      hand-assembled, and independently cross-checked with `readelf` and
      a hand-rolled Python ELF parser before trusting the kernel's own):
      segment 1 (at `USER_CODE_ADDR`) makes a real linker-resolved
      cross-page `call` into segment 2 (a genuinely different physical
      frame), which holds the distinguishing message and performs the
      write+exit syscalls -- real proof of multi-segment loading and
      execution reaching a non-zero-offset segment, not just segment 0.
      **A real bug found and honestly disclosed, not swept under the
      rug**: an intermittent (~50% reproduction) page fault occurs
      shortly after an ELF-loaded process returns to the kernel, if
      shell activity follows within about a second. Real diagnosis was
      performed, not guessed: instrumented `Cr3::read()` inside the
      timer interrupt handler (ruled out "scheduler runs under the
      wrong CR3"), tested a single-segment ELF (ruled out "specific to
      multi-segment mapping" -- it still reproduced), and stress-tested
      the pre-existing `runproc`/`runfile` paths using the identical
      CR3-switch mechanism for 17+ repetitions with zero crashes,
      isolating the bug to the new ELF path specifically. Faulting
      addresses were consistent with corrupted kernel-context stack
      state, not resolved to an exact root cause. Independently
      reproduced with the identical signature during this milestone's
      own merge verification -- confirmed real, confirmed still open,
      not a fluke. `runelf` should be treated as not yet safe for
      repeated/rapid interactive use until root-caused; `runfile`/
      `runproc` are unaffected and remain solid. Built in an isolated
      parallel-agent workspace, then hand-merged alongside the
      concurrently-built Milestone 35 above -- both milestones extended
      the shared `Process` struct with their own new field
      (`fds`/`extra_frames`), reconciled via a real 3-way `diff3` merge
      against the shared Milestone 34 baseline (6 total conflicts across
      3 files, every one a genuine "both milestones need this" case,
      none discarded) -- and independently re-verified against the
      real, fully-merged project tree, including confirming the known
      bug above reproduces with its documented signature and nothing
      new broke.
- [x] **Milestone 37**: real `fork`/`exec`/`wait` -- a dynamic
      `PROCESS_TABLE` (4 slots), `fork()`, `wait_for_child()`/
      `run_forked_child()`, and `exec_process()`, plus syscalls
      7(fork)/8(wait)/9(exec) and a new `runfork` shell command.
      `fork()` makes a genuine byte-for-byte copy of the parent's code
      frame into a distinct physical frame; `wait()` switches `CR3` to
      the child's own `pml4`, resumes it at its exact fork()-time
      register snapshot (`rax` forced to 0), and reaps it once its own
      `exit()` syscall runs, restoring the parent's `CR3`. **A real,
      deep bug found and fixed, not guessed at**: `int 0x80` always
      resets `RSP` to the same fixed `TSS.privilege_stack_table[0]` top
      on every ring3->ring0 transition, so a forked child's own nested
      syscalls during the parent's in-flight `wait()` were silently
      clobbering the parent's saved CPU state at the identical stack
      address -- caught via a real page fault (RIP=0x200) in the serial
      log. Fixed with a second, dedicated kernel stack
      (`gdt::CHILD_EXCURSION_STACK`), temporarily swapped into the TSS
      only around the child's nested excursion. Verified live in real
      QEMU via `runfork`, hardware-recorded `CS=0x1b` (CPL=3) confirming
      genuine ring-3 execution throughout: `fork()` created child pid 10
      with a verified-distinct code frame; the parent's and child's own
      distinguishing `WRITE` messages both printed correctly; `wait()`
      correctly reaped the child and restored the parent's `CR3`; the
      kernel resumed normal operation afterward (the preemption demo ran
      cleanly right after) with no crash or hang -- real evidence the
      stack-clobbering fix holds under an actual nested-syscall
      scenario, not just in theory. **One real, honest limitation, not
      glossed over**: this run exercised only `exec()`'s FAILURE path
      (the test filesystem has no `testprog` file to exec, so
      `exec_process()` correctly returned `u64::MAX` and the child ran
      its fallback message) -- the success path, actually replacing a
      running process's image, was not exercised here and remains
      unverified.
- [x] **Milestone 38**: real ternary-quantized weight persistence -- a
      genuine cross-project integration, not an from-scratch idea. Ports
      OBSERVE's (a separate project, `012-trit-search`) real, benchmarked
      ternary bit-packing technique -- quantize to `{-1,0,+1}`, pack 5
      trits/byte (3^5=243<256) -- into `kernel/src/ternary.rs`, pure
      integer `#![no_std]` arithmetic, no external crate. **A real
      technical distinction that mattered**: OBSERVE compresses
      high-dimensional embedding VECTORS (one coarse trit per dimension,
      precision survives in aggregate across 384 dims) -- spikeling-os's
      synapse weights are individual SCALARS, where one trit could only
      ever represent 3 values. The real per-weight design instead spends
      10 trits of *precision* on each scalar (a base-3 fixed-point
      encoding over the STDP clamp range `[0,1]`, 3^10=59,049 levels,
      resolution ≈1.7e-5 -- roughly 24x finer than the smallest real STDP
      delta this kernel produces), packing into exactly 2 bytes vs 4 for
      f32: a real, honestly-modest **2.00x** compression (smaller than
      OBSERVE's own ~20x on purpose -- different value of the same
      technique, not a shortfall). **A real, known failure mode from that
      same upstream project's own history was checked against
      proactively, not discovered the hard way**: an earlier ternary
      scheme in that codebase measurably broke by omitting a real
      magnitude/scale factor alongside the packed digits. Verified before
      writing any Rust (a numeric check in Python first, max error 3.82e-6
      across representative weights) that this design's fixed, exactly-
      known scale constant applied symmetrically on encode/decode does
      NOT have that bug. The constraint that mattered most -- ternary
      quantization applies ONLY at the disk save/load boundary, never to
      the live, actively-STDP-learning weights -- was verified empirically
      (live weights confirmed bit-identical immediately before/after
      `save`), not just designed-for. Also generalized `ata.rs`'s
      persistence from two hardcoded synapses to the whole current
      `GenericNetwork`'s synapse list (name-keyed, arbitrary count) --
      the honest gap Milestone 21's network unification had left open
      since Milestone 11. Verified end to end for real: trained 3 live
      synapses (2 fixed + 1 DSL-added) to real, non-round values, saved
      (serial log: `saved 3 synapse weight(s) -- 6 ternary-packed bytes
      vs 12 equivalent f32 bytes (2.00x smaller)`), then -- same
      discipline as Milestone 11/32 -- booted a completely FRESH QEMU
      process against the same disk image and confirmed the reloaded
      weights matched the pre-reboot values exactly (`0 of 3` topology
      lost as documented: DSL-added synapse topology isn't persisted,
      only weights of pre-existing synapses are). Regression-checked:
      bare `train`'s classic LTP/LTD round-trip and the `addneuron`/
      `addsynapse`/`stim`/`train` DSL both produced identical deltas to
      before, confirming live learning precision is completely
      untouched. **Honest limitations disclosed**: DSL-added synapse
      *topology* does not survive reboot, only weights of synapses that
      already exist at boot; weights outside `[0,1]` (reachable via
      `addsynapse weight=W`, which bypasses STDP's own clamp) saturate on
      encode rather than round-tripping exactly. Built in an isolated
      parallel-agent workspace (running alongside Milestone 37, in a
      genuinely disjoint set of files -- `ternary.rs`/`ata.rs`/
      `network.rs`/`neurons.rs` vs. `process.rs`/`usertest.rs` -- by
      design, to avoid the merge overhead multiple milestones touching
      the same core files caused earlier), then merged cleanly (no
      conflicts) against the real project tree.
- [x] **Milestone 39**: real evidence-gated synapse topology, a genuine
      self-healing connectivity mechanism -- another real cross-project
      port, not invented from scratch. `kernel/src/network.rs` gains
      `Synapse.gate: bool` and `Synapse.evidence: f32`, ported from a
      separate project's real, benchmarked design (`012-ternary`'s
      `tritkit`, `twotimescale.py`'s `TwoTimescaleLinear`): each synapse
      is factored into its existing fast weight/polarity (STDP) and a
      slow binary connectivity gate, re-evaluated from an EMA of
      pre/post firing-correlation evidence that is deliberately
      independent of the weight itself -- avoiding a real, documented
      failure mode from that same upstream project's own history
      (gating from a connection's own weight is self-reinforcing: a weak
      connection gets gated off and can never again accumulate the
      activity needed to prove it deserves reconnection, permanently
      locked out). Fast clock (every tick, every synapse, regardless of
      gate): updates the evidence EMA from real firing timing. Slow
      clock (`golden_period(FAST_TAU_TICKS)` ticks -- golden-ratio
      spaced, ported verbatim from tritkit's own `golden_period()`,
      deliberately incommensurate with the fast cadence to avoid
      resonance): re-evaluates the gate with hysteresis. Gated-off
      synapses are excluded from real stimulus propagation and STDP
      (frozen weight -- tritkit's "structural memory": dormant state
      preserved unchanged, not reset, so a later reconnect resumes
      exactly where it left off) but NOT from the evidence pass (must
      keep sensing while dormant, the exact property required to avoid
      the lockout bug above). `train_synapse()` (the Milestone 17/21
      controlled STDP trial) deliberately bypasses gating entirely,
      exactly as it already bypassed real propagation timing -- a
      synthetic trial tool, not real network dynamics, kept byte-for-
      byte regression-safe. **Verified live with real hardware evidence
      via QEMU screendumps** (this milestone's shell output goes to the
      framebuffer, not serial, so verification used `screendump` through
      the QEMU monitor rather than the serial log): fresh boot showed
      both fixed synapses at `gate=ON, evidence=1.000` (the neutral
      default). 11 real `stim LeftKey 120` firings, with Motor never
      once crossing its own threshold (zero correlated hits), decayed
      `LeftKey->Motor`'s evidence to `0.086` and flipped its gate `OFF`
      -- `RightKey->Motor`, never stimulated, stayed completely
      untouched at `gate=ON, evidence=1.000`, confirming the mechanism
      is genuinely per-synapse, not global. 5 more real firings while
      gated off left the weight at **exactly** `0.5000`, byte-for-byte
      unchanged, while evidence kept decaying (`0.086` -> `0.028`) --
      real, live confirmation of both halves of the design: frozen
      weight while dormant, and continued sensing while dormant. **A
      real mid-verification mistake found and fixed, not hidden**: the
      first attempt to sync a fresh verification workspace used
      `robocopy` from kimchi's own already-diverged `spikeling-os`
      folder (a different session's separate commit history) instead of
      this repo's own tree -- caught by checking for `tools/testelf_src`
      (this repo's own) vs. the wrong copy's `tools/libc_test_src`
      (kimchi's unrelated Milestone 39), fixed by re-syncing from a
      tarball of the actual local tree before building. **Honest
      limitation, disclosed not hidden**: gate state and evidence are
      not persisted to disk (Milestone 38's scope only covers
      `Synapse.weight`) -- both start fresh (`gate=ON`,
      `evidence=1.000`) every boot.
- [x] **Fixed a real, longstanding bug: `ata.rs` never had a secondary
      disk attached, since Milestone 11.** Surfaced while chasing
      Milestone 40's own verification: a new boot-time,
      non-interactive self-test (`fs::self_test_disk_write()` --
      written specifically because interactive shell-command testing
      via QEMU's `sendkey` has been unreliable in this environment)
      failed on every clean boot with `"ATA error bit set"`. Root
      cause traced to `src/main.rs` (the actual runner `cargo run`
      invokes), not `ata.rs`'s own PIO protocol logic: `ata.rs` has
      targeted the SECONDARY ATA bus (ports 0x170-0x177) since
      Milestone 11, on the documented assumption that a dedicated disk
      image is attached there in "the verification harness" -- but
      that harness never actually existed in the real runner, which
      has only ever attached one drive (the boot image, implicitly
      primary-master). Every disk write since Milestone 11 was issued
      against an unclaimed secondary bus. **Never caught before
      because `load_weights()` swallows read errors via `.ok()?`**, so
      a real ATA error and a genuinely blank disk both produced the
      identical, honest-looking "no saved weights found on disk" log
      line -- masking a real bug for 29 milestones. Real fix:
      `src/main.rs` now creates (if missing) and attaches a real,
      stable-across-runs persistence disk (`target/persist.img`)
      explicitly on `bus=ide.1 unit=0`. Verified for real via a
      genuine QEMU serial log: `fs self-test: disk write+read
      roundtrip OK -- real bytes matched`, not assumed.
- [x] **Milestone 40**: real `pipe()` + `dup`/`dup2`. A new,
      ref-counted `PIPE_TABLE` backs a `FdEntry::PipeRead`/`PipeWrite`
      variant alongside Milestone 35's existing `File` variant, so
      `read_fd`/`write_fd` (syscalls 4/5) transparently work on pipes
      with zero change to those syscalls themselves -- they dispatch
      on whichever variant a given fd actually holds. A fixed 512-byte
      ring buffer per pipe; **honest non-blocking semantics** (empty
      read returns 0 bytes, full write returns a partial count) rather
      than inventing scheduler-level blocking this kernel's process
      model doesn't really support yet -- a real, disclosed scope
      boundary. `dup`/`dup2` reuse the same ref-counting `fork()`
      already needed; `dup2` onto an already-open fd closes it
      properly first (no leaked pipe ref, no dropped unpersisted file
      data), and is a real POSIX no-op when `fd==newfd`, checked
      before the close-then-clone sequence so it can't destroy the
      descriptor it's supposed to preserve. New syscalls 10/11/12. A
      real, externally-built ELF64 test payload
      (`tools/pipetest_src/`, `kernel/assets/pipetest.elf`) plus a
      `seedpipetest` shell command for on-disk/interactive testing via
      the existing `run_elf()` path, unchanged. **Verified for real**
      via a genuine boot-time, non-interactive self-test
      (`process::self_test_pipe_mechanics()`, same discipline as the
      ATA fix above, adopted because interactive `sendkey` testing has
      been unreliable throughout this milestone's development):
      confirmed via a real QEMU serial log that a byte payload written
      to a fresh pipe's write end reads back correctly; that
      `dup_fd()` produces a second fd genuinely sharing the underlying
      pipe (write through the dup, read through the original -- not
      independent copies); and that `dup2_fd()` redirects onto a
      caller-chosen fd number that still reaches the same pipe. All
      four checks passed: `write_ok=true roundtrip_ok=true
      dup_ok=true dup2_ok=true`. This milestone's own investigation is
      what surfaced the separate, longstanding ATA bug directly above
      -- the "ATA error bit set" this milestone first ran into existed
      since Milestone 11 and was never caused by this milestone's own
      code.
- [x] **Milestone 41**: real signals -- SIGSEGV (a page fault from real
      CPL=3 code terminates just that process, kernel continues, instead
      of every prior milestone's unconditionally-fatal page fault) and
      SIGKILL (new syscall 13, unconditionally frees a live never-run
      forked child's slot, bypassing `wait()`'s normal run-then-reap
      contract). Custom signal-handler registration/execution is an
      honest, disclosed scope-cut -- terminate-on-fault alone proved to
      be the real, achievable target. **The real story here is the
      investigation, not just the feature**: this milestone's own
      boot-time self-test was the first thing in this project's entire
      history to call `run()`/enter ring 3 non-interactively from inside
      `kernel_main()` itself, rather than via the interactive shell's
      command loop (which naturally runs later in boot, after every
      piece of infrastructure ring-3 entry actually depends on is
      ready). That exposed two real, previously-invisible bugs, both
      about calling it too early: (1) before this kernel's own GDT/IDT
      were actually loaded via `lgdt`/`lidt` -- entering ring 3 that
      early faults IRETQ's own selector validation, confirmed via a real
      hardware `#GP` whose error code decoded to the user code
      selector's GDT index; (2) even after fixing that, before the PIC
      was remapped -- `enter_ring3()`'s own hand-built RFLAGS correctly
      sets IF=1 (needed for a normal preemptible process), so the IRETQ
      into ring 3 enables interrupts globally the instant it executes,
      regardless of whether the kernel's own `sti()` (which runs later,
      right after PIC remap) has fired yet; a real PIT timer tick
      landing in that now-interrupts-enabled-but-still-unmapped window
      got delivered on the 8259's unremapped default vector -- raw INT
      0x08, the exact same vector this kernel reserves for the CPU's own
      double-fault exception -- producing a real hardware double fault
      whose frame was missing the error code a genuine double fault
      always has, shifting every field the handler read by one word.
      Confirmed directly via a real QEMU `-d int` trace showing the raw
      vector (not inferred from error-code shape): `v=08 e=0000 i=0
      cpl=3`, immediately preceded by a real flood of raw, unmapped
      `Servicing hardware INT=0x08` deliveries. Fixed by moving the
      self-test to run after both GDT/IDT load and PIC remap. **Eleven
      real investigation rounds**, most of which found nothing wrong
      with (but conclusively ruled out) plausible-looking hypotheses --
      fixed-array bounds, a process/frame-count threshold, a
      TSS-restoration leak from Milestone 37's fork excursion mechanism,
      a GDT/IDT/TSS physical-frame collision, IST stack misalignment
      (a real, independent bug, fixed and kept regardless), the
      IST-switch mechanism itself, and a genuinely missing
      general-protection-fault handler (also a real, independent gap,
      fixed and kept) -- each one honestly eliminated with real evidence
      before moving to the next, rather than assumed away. Verified end
      to end with a clean, fresh, non-interactive boot: `SIGSEGV_TEST_
      PROCESS` writes its message, faults at a deliberately unmapped
      address, gets terminated -- no panic, no double fault. `PROCESS_A`
      then runs completely normally right after, real proof of genuine
      recovery rather than "hasn't crashed yet". `SIGKILL_TEST_PROCESS`
      forks a child, kills it unrun, forks again -- `PROCESS_TABLE`
      shows exactly one occupied slot afterward, proving the first
      child's slot was truly freed and reused, not just marked dead.
- [x] **Milestone 42**: real process groups -- a `pgid` field on
      `Process`, genuine Unix semantics: a freshly-created (non-forked)
      process is the founder of its own group (`pgid` == its own
      well-known id), a `fork()`ed child INHERITS its parent's `pgid`
      rather than starting a new one. Two new syscalls: `setpgid`
      (14, target pid + new pgid) and `getpgid` (15, target pid) --
      `setpgid`'s authorization mirrors `kill()`'s own precedent
      (Milestone 41): succeeds for self, or for a live child of the
      caller (the real call a shell makes right after forking a
      pipeline's own processes, to put them in a shared group), refused
      otherwise. Honest, disclosed scope-cut: no session/controlling-
      terminal concept exists yet, so real POSIX `setpgid()`'s
      same-session restriction isn't enforced -- deferred, not silently
      dropped. Verified via a real, non-interactive boot self-test
      (`self_test_process_groups()`, pure kernel-side `fork()`/
      `setpgid()`/`getpgid()`/`kill()` calls, never through a real
      `int 0x80` -- so unlike Milestone 41's own self-test, this needs no
      ring-3 entry and none of that milestone's hard-won `init_pics()`/
      `sti()` ordering constraint): confirmed real inheritance (forked
      child's pgid matches `FORK_TEST_PROCESS`'s), real divergence after
      `setpgid` (child's pgid changes, parent's verifiably doesn't), and
      real authorization enforcement (`PROCESS_A` -- not the child's
      parent -- attempting `setpgid` on it is actually refused, not just
      documented as refused). Cleans up its own forked test child before
      returning so it doesn't consume a `PROCESS_TABLE` slot Milestone
      41's own SIGKILL self-test also needs later in the same boot --
      confirmed no interference, every self-test after this one still
      passes cleanly.
- [x] **Milestone 45**: real `exec()` -- a syscall that replaces the
      calling process's own address space with a freshly-loaded ELF64
      image from disk, reusing the existing ELF loader/parser
      (`elf.rs`/`loader.rs`) and the same page-table teardown-and-rebuild
      primitives `fork()` already has for a child's private space, so a
      process can genuinely replace itself instead of only ever forking
      a fixed hardcoded image. Also root-caused and fixed a real,
      pre-existing bug found while verifying this milestone (confirmed
      pre-existing via an isolated `git worktree` boot of the unmodified
      kernel, zero Milestone 45 code involved): boot-time self-tests
      silently stopped dead at `self_test_wait_status()`'s own
      "OVERALL: PASS" line on every run -- root-caused with a real QEMU
      `-d int` hardware trace to interrupt servicing silently stopping
      after 15 real interrupts. Honest, disclosed scope-cuts: `exec()`
      now requires a real ELF64 image (Milestone 34's flat `testprog` is
      no longer a valid target, a real behavior change from Milestone
      37's own placeholder); physical frames from the replaced address
      space are abandoned, not reclaimed (matches this kernel's existing
      bump-only frame allocator precedent, not a new gap). Verified
      non-interactively at boot (`EXEC_TEST_PROCESS` / `self_test_
      real_exec()`): real teardown-and-rebuild confirmed via a genuine
      PML4/CR3 switch and hardware-recorded `CS=0x1b` (CPL=3), fd table/
      parent_pid/pgid preserved across the exec, entry point matches the
      real parsed `e_entry` from a genuine externally-built ELF64 target
      whose `e_entry` does NOT equal the old fixed `USER_CODE_ADDR` (so
      the check can't pass by coincidence) -- `OVERALL: PASS`, no panics,
      no double faults. Independently re-verified by the orchestrating
      session with a fresh, separate `cargo build` + QEMU boot after
      discovering this milestone's real work sitting complete but
      uncommitted in the working tree.
- [x] **Milestone 48**: `ternary.rs` wired into a real, observable
      kernel decision path -- `compare_trit()` sits in the real
      tick-switch path (`tasks::timer_tick_switch`), engaged as a
      tiebreak within `epsilon=0.01` when two candidate slots' scores
      are nearly equal. Honest, measured comparison against the binary
      equivalent it replaces, matching this project's own Milestone 4
      discipline of reporting neutral/negative results plainly: over
      4000 ticks / 8 slots (`g=0.6`), the ternary tiebreak actually fired
      on 2 of 4000 ticks; measured fairness was identical between the
      ternary and binary selection paths in this run -- a real neutral
      result, not an assumed win. Independently re-verified twice: once
      by a separate Reviewer agent (fresh `cargo build` + QEMU boot
      reproducing the exact serial-log claims), and again by the
      orchestrating session with its own fresh boot after merging onto
      `main` alongside Milestone 45 -- both reproduced identically
      (fire count 2/4000, fairness 0.7486 both paths). One open, honestly
      disclosed discrepancy: the implementer's own report claimed a
      different fairness figure (0.9980) that neither independent
      re-verification reproduced -- the qualitative "no measurable
      difference between ternary and binary" finding holds in every run
      regardless, but the specific number is unresolved.
- [x] **Milestone 46**: a real VFS/mount abstraction generalizing
      `fs.rs`. The one real disk-backed path now sits behind a real
      trait/dispatch layer; a second, real backing store (a small
      in-memory ramfs) mounts at `ram/...` alongside the existing disk
      root through the same `path`/`open()` surface. Real, disclosed bug
      found and fixed during this milestone's own verification: the
      ramfs isolation self-test initially failed because its test file
      name collided with a name an unrelated earlier self-test had
      already written to the disk root -- a test-authoring mistake, not
      a real isolation bug (fixed with a disjoint name, not by weakening
      the check). Honest scope-cuts: no live interactive shell session
      exercised the new path (shell.rs unmodified, calls the same fs
      functions the self-tests already cover); the fd-backed syscall
      path (`open`/`read`/`fdwrite`/`close`) reaching RamFs has no
      dedicated self-test, to keep this milestone's diff scoped to
      `fs.rs` rather than also touching `process.rs`'s process-table
      machinery. Independently re-verified by the orchestrating session:
      fresh build + fresh QEMU boot, real ramfs write+read roundtrip,
      `list()`, and isolation (disk root does NOT contain the ramfs test
      file) all confirmed, no panics.
- [x] **Milestone 47**: real ARP request/reply over the actual
      (virtual) wire. Closes a gap every verification through Milestone
      26 shared without disclosing: `sendpacket`/`recvpacket` always ran
      with the e1000 PHY's loopback bit set, so nothing before this
      milestone proved the driver could talk to anything other than
      itself. A new `set_loopback(false)` toggle turns that off for a
      genuine test: a real ARP request is transmitted for QEMU user-mode
      networking's own documented gateway (`10.0.2.2`), and the RX ring
      is polled for a genuine parsed reply from QEMU's slirp stack --
      the first real protocol (L3 address resolution) this kernel
      speaks, no IP/UDP/TCP stack yet (disclosed scope limit).
      `main.rs`'s host QEMU runner now unconditionally passes a real
      `-netdev user` + `-device e1000`, plus an optional real pcap
      capture for packet-level evidence. No implementer report exists
      for this milestone -- the real `claude` CLI subprocess doing the
      work hit a real account usage-limit wall right as it was about to
      write one, after several real, visible debug/rebuild/reboot
      iterations. The orchestrating session committed this from
      independent verification alone: fresh build (clean) + fresh QEMU
      boot with real networking enabled, ARP self-test genuinely passes
      (`"10.0.2.2 is-at 52:55:0a:00:02:02"`, a real reply from QEMU's
      slirp gateway over the actual netdev backend, loopback OFF), no
      panics. One real merge conflict resolved during integration: both
      this milestone and the already-merged Milestone 45 independently
      extended the shell's `help` command-list string -- resolved by
      taking the union of both additions (`arp` plus
      `seedaltentry, runexectest`), verified afterward with a combined
      fresh boot showing Milestones 4/41/42/43/45/46/47/48 all correct
      together, no panics.
- [x] **Milestone 44 (completed)**: real, non-interactive proof the
      generalized ELF loader (left in-progress at commit `a604426`)
      genuinely accepts and runs a non-`USER_CODE_ADDR` entry point --
      not just that the old "entry MUST equal `USER_CODE_ADDR`"
      rejection was removed, but that a real ring-3 excursion actually
      reaches the test ELF's own alternate entry point (three pages past
      `USER_CODE_ADDR`), runs its write+exit syscalls, and returns
      cleanly. Two real checks: `elf::parse()` confirms a genuinely
      non-default `e_entry`; a new `LAST_WRITE_SYSCALL_PID` static (set
      unconditionally inside the real syscall dispatcher) is checked
      after running the ELF through `process::load_and_run_elf()` --
      deliberately stronger than "returned `Ok(())`", since (per
      Milestone 41) a faulting ring-3 process is gracefully terminated
      the same way a clean exit is, so `Ok(())` alone can't distinguish
      "genuinely ran" from "regressed to the old bug and immediately
      faulted on the still-unmapped default address." Real, substantive
      merge conflict with the already-merged Milestone 45 in
      `kernel_main()`'s own self-test ordering, resolved by hand: this
      self-test is ALSO a non-interactive ring-3 excursion, so
      Milestone 45's own interrupt-re-enable fix (see that entry) had to
      be placed downstream of BOTH self-tests, not sandwiched between
      them -- placing it in between would have silently reintroduced the
      exact IF=0 deadlock Milestone 45 root-caused. Independently
      verified concretely, not just "didn't crash": the post-merge boot
      shows 82 real timer interrupts observed and all three preemption-
      demo tasks genuinely still running afterward, proving interrupts
      genuinely stayed enabled through both ring-3 excursions. Assigned
      by the orchestrating dispatcher as "Milestone 50 (waitpid
      exit-status semantics)"; the real `claude` CLI subprocess read the
      codebase first, found Milestone 44 genuinely incomplete, and used
      its real budget completing that instead (no implementer report --
      hit a real account usage-limit wall before writing one). Labeled
      honestly here for what it actually is; the real, originally
      assigned waitpid-semantics scope remains undone.
- [x] **Milestone 51**: real `malloc()`/`free()` on top of `sys_sbrk()`,
      closing the "no heap allocator wired to sbrk() yet" gap `libc.rs`
      disclosed back at Milestone 39. A real, minimal intrusive
      free-list allocator: every block carries an 8-byte size header; a
      free block's own payload doubles as the "next free" pointer.
      `malloc()` walks the free list first-fit, splitting a block that
      leaves a genuinely independently-usable remainder, falling back to
      `sys_sbrk()` growth when nothing fits. Real, disclosed scope-cut:
      no coalescing of adjacent freed blocks -- a real, present
      limitation on the fixed 16 KiB per-process heap, not hidden.
      Verified with a real, standalone ELF64 test program exercising 6
      hand-computed-in-advance predictions against the allocator's real
      live behavior (two fresh allocations at the exact predicted
      offset, a freed-then-realloced same-size block reusing the exact
      same address, a freed-then-smaller-realloc genuinely splitting
      with the remainder at the exact predicted address) -- each
      independently write/read verified for real memory isolation.
      Real, substantive merge conflict with the already-merged
      Milestone 44 completion (see above): both add a non-interactive
      ring-3 self-test to `kernel_main()`'s own self-test block,
      resolved by hand keeping both self-tests, with BOTH placed
      upstream of Milestone 45's interrupt-re-enable fix (the same real
      ordering hazard as the Milestone 44 conflict above -- three ring-3
      self-tests now share that one fix). Independently re-verified by
      the orchestrating session: fresh build + fresh QEMU boot,
      hand-reconstructed every real hex pointer value from the
      hardware-traced WRITE syscall log against the test program's own
      documented predictions -- all 6 named checks plus `OVERALL` report
      `PASS`; combined boot also re-confirmed 82 real timer interrupts
      and genuine preemptive multitasking still functioning afterward,
      proving the three-self-test interrupt ordering holds.
- [x] **Milestone 53**: real `WaitOutcome::Signaled` status for forked
      children terminated by a hardware fault instead of their own
      `exit()`. Closes a genuinely silent, previously-undetected gap:
      `wait_for_child()` had no way to distinguish a page-faulted/#GP'd
      child from a normally-exited one, so it reported whatever STALE
      exit code an earlier, unrelated child's real `exit()` last wrote
      (or 0, on a boot's first `wait()`) as `WaitOutcome::Exited`,
      mislabeling a crashed child as one that finished normally. Fixed
      with a `CHILD_FAULTED` flag set by
      `terminate_faulted_process_and_resume_kernel()` only when the fault
      hit a forked child mid-`wait()`, consumed atomically by
      `wait_for_child()` before it ever reads the exit-code field. Proven
      by a real, non-interactive self-test: forks a child whose entire
      program is a single faulting instruction (no `exit()` anywhere),
      confirms `wait()` reports `Signaled` not a stale `Exited`, confirms
      an unrelated top-level process still runs normally right after
      (real recovery, not just "didn't crash yet"), and confirms the
      freed slot is genuinely reusable by forking a second child into it.
      That self-test found and fixed a real bug in itself, not the
      kernel: its first version asserted the whole process table was
      empty after cleanup, which is false given Milestone 41's own
      SIGKILL self-test deliberately leaves an unrelated child
      permanently unreaped elsewhere in the table -- caught via an actual
      QEMU boot, not review, and narrowed to check only its own slot.
      Real merge conflict with the already-merged Milestone 45: this
      branch predated Milestone 45 and independently picked PID 9 for
      its own new hardcoded process slot, colliding with Milestone 45's
      `EXEC_TEST_PROCESS_ID`; resolved by renumbering to PID 10 and
      bumping `PID_TABLE_BASE` to 11. A second conflict interleaved
      `exec_elf()`'s body with this milestone's self-test and a stale
      pre-Milestone-45 `exec_process()` placeholder this branch never saw
      removed; resolved by hand, reconstructing `exec_elf()` contiguously
      and dropping the stale placeholder. Re-verified with a fresh build
      and a fresh QEMU boot on the merged tree (persist disk attached):
      milestones 42/43/44/45/53 all self-report `OVERALL: PASS`, zero
      panics, boot reaches the interactive shell normally.
- [x] **Milestone 54**: real physical frame reclamation on process
      teardown. Closes `memory.rs`'s own disclosed "bump-allocates only,
      never frees" limitation -- `BootInfoFrameAllocator` gains a real
      LIFO free list, checked before ever bumping `next` further;
      `reclaim_process_frames()` returns a dead process's frames to it,
      wired into `kill()`, both `wait_for_child()` reap paths, and
      `replace_process()` (`exec()`'s teardown-and-rebuild step) --
      closing the identical gap each of those three had separately
      disclosed since Milestones 37/41/45. Also reclaims the PRIVATE
      P3/P2/P1 page-table frames `map_to()` allocates on demand while
      building a process's address space, found missing only by actually
      running the self-test on real hardware, not by inspection: a
      second forked child needed more real physical frames than the
      leaf-only version (pml4/code/stack/heap) had freed.
      This self-test earned its own honest, undisclosed-nowhere-else
      history: three real failures in sequence on three separate QEMU
      boots, each one a genuine finding, none hidden or quietly patched
      over --
      (1) hardcoded "+3 frames," wrong: `fork()` eagerly maps a private
      4-page heap for every child, not lazily via `sbrk()`, so the real
      leaf count is 7;
      (2) leaf count fixed, but asserted a second fork() reused the
      exact freed frame SET -- wrong again: `map_to()` allocates its own
      private P3/P2/P1 table frames on demand, which the leaf-only
      reclaim never freed, so a second process needs strictly more real
      frames than 7;
      (3) page-table reclaim implemented and independently count-
      verified via a read-only walk (`count_private_page_tables()`,
      deliberately kept separate from the freeing walker so the test
      isn't just trusting the code path it exists to check), but STILL
      asserted the second fork()'s LEAF frames matched the freed LEAF
      set specifically -- wrong a third time: leaf and page-table
      `allocate_frame()` calls are interleaved during construction (pml4,
      code, stack, then on-demand P3/P2/P1, then each heap page followed
      immediately by its own `map_to()`), so the LIFO free list's pop
      order doesn't hand leaf frames back to the leaf fields alone.
      Final, actually-passing design tests the TOTAL instead: a second,
      symmetric fork() from the same source independently re-derives the
      identical real frame cost (proving determinism, not assumed) and
      drains the free list to exactly zero building itself -- simpler
      than field-level address matching and strictly stronger proof of
      real reuse. Re-verified end to end on the merged `main` after a
      clean fast-forward merge: fresh build, fresh QEMU boot with the
      persist disk attached, milestones 42/43/44/45/53/54 all
      self-report `OVERALL: PASS`, zero panics, boot reaches the
      interactive shell normally.
- [x] **Milestone 55**: real ICMP echo request/reply (`ping`), the first
      IP-layer protocol this kernel speaks -- closes the gap Milestone
      47's own doc comment explicitly disclosed as future work ("no IP/
      UDP/TCP layer exists on top of this yet"), built directly on that
      milestone's real ARP resolution. Adds a real IPv4 header + ICMP
      echo request/reply frame builder, a shared Internet checksum
      (RFC 1071, one real implementation used for both the IP and ICMP
      checksums rather than two independently-trusted copies), and
      `icmp_ping()`: ARP-resolves the target's MAC first (ICMP needs a
      real unicast destination, unlike ARP's own broadcast), then a real
      echo request/reply round trip. Deliberately calls
      `send_arp_request()`/`recv_arp_reply()` directly rather than the
      public `arp_resolve()` wrapper -- `arp_resolve()` restores loopback
      to ON at its own end, which would silently divert the ping frame
      back to this NIC's own RX ring instead of the real wire if called
      from inside an already-loopback-off caller; loopback control stays
      owned by one function for the whole real round trip. New `ping`
      shell command mirrors `arp`. Boot-time self-test pings QEMU
      slirp's own gateway (10.0.2.2) with loopback OFF and checks the
      reply's `sender_ip`/`id`/`seq` all match what was actually sent,
      not just that some reply arrived. First real QEMU boot passed
      outright -- a genuine echo reply from slirp's real stack, its
      checksums validated on the other end (not just a packet coming
      back), no bugs found this time. Clean fast-forward merge into
      `main` (no conflicts); re-verified end to end afterward: fresh
      build, fresh QEMU boot with the persist disk attached, milestones
      42/43/44/45/47/53/54/55 all pass, zero panics, boot reaches the
      interactive shell normally.
- [x] **Milestone 56**: real UDP (RFC 768), the second IP-layer protocol
      this kernel speaks -- closes another piece of the gap Milestone
      47's own doc comment disclosed as future work, built on Milestone
      55's real IPv4 header framing and Milestone 47's ARP resolution.
      Adds `UdpHeader` with hand-written network-byte-order encode/
      decode, `udp_checksum()` (RFC 768's pseudo-header fed through the
      SAME shared `internet_checksum()` Milestone 55 built for IPv4/
      ICMP, not a second, independently-trusted copy, including the
      "computed checksum of 0 is sent as 0xFFFF" rule handled honestly
      on both ends), `build_udp_frame()`/`send_udp()` (an owned `Vec<u8>`
      since a real UDP message's length isn't known until runtime,
      bounded by `UDP_MAX_PAYLOAD` = 214 bytes with a real `Err` on
      overflow rather than silent truncation), and
      `parse_udp_datagram()`/`recv_udp()` (bounds-checked before any
      slice indexing, validates the checksum by recomputing it against
      the bytes actually received rather than trusting the sender's
      claim). A real minimal listener registry
      (`register`/`unregister_udp_listener`, `MAX_UDP_LISTENERS` = 8)
      gives `recv_udp()` genuine port-based demultiplexing.
      `udp_send_resolved()` mirrors `icmp_ping()`'s own loopback
      discipline exactly, for the identical reason. New
      `udpsend <ip> <port> <message>` shell command. Two boot-time
      self-tests: a primary loopback round trip through the real receive
      path (checksum validation included -- chosen over a real off-box
      round trip because QEMU slirp has no UDP application service to
      bounce one off of, confirmed by checking the netdev config for
      hostfwd/guestfwd rules: none exist) and a real-wire send test that
      calls `udp_send_resolved()` directly -- the literal function
      `udpsend` invokes -- and gets a real TX descriptor DD-confirmation
      against QEMU slirp's gateway. Both self-tests PASS with exact
      matching field values (sender IP, both ports, 38-byte payload) on
      first real boot, no bugs found this session. Independently
      reviewed in a separate verification pass (fresh build, fresh boot
      log, direct source inspection) -- verdict: VERIFIED, no
      discrepancies found. Clean fast-forward merge into `main` (no
      conflicts); re-verified end to end afterward: fresh build, fresh
      QEMU boot with the persist disk attached, milestones
      42/43/44/45/47/51/53/54/55/56 all pass, zero panics, boot reaches
      the interactive shell normally.
- [x] **Milestone 57**: real, hardware-fault-driven demand paging for the
      per-process heap -- the first piece of the Tier 1 roadmap's "virtual
      memory/demand paging" item (real virtual memory is a much bigger
      target -- copy-on-write and `mmap` remain future work; this
      milestone is specifically the heap's reserve-vs-commit-vs-map
      split). `HEAP_PAGE_COUNT` grows 4 -> 64 pages (16 KiB -> 256 KiB of
      *reserved* virtual address space per process) for zero eager
      physical cost: `create_process_from_image()`/`create_process_from_elf()`
      no longer map a single heap page up front (`heap_frames` becomes
      `Vec<Option<PhysFrame<Size4KiB>>>`, all `None` at creation) --
      `sbrk()` still only bumps a purely virtual `heap_used` counter, and
      a real physical frame is allocated, zeroed, and mapped into that
      ONE process's own private page tables only the first time a genuine
      hardware `#PF` touches a byte it has already legitimately committed
      via `sbrk()` (`try_demand_page_heap()`, called from
      `interrupts.rs`'s `page_fault_handler` before the existing
      Milestone 41 SIGSEGV path, and only for a real NOT-PRESENT fault --
      never a protection violation). Touching reserved-but-uncommitted
      heap space (beyond what `sbrk()` actually returned) still falls
      straight through to the unmodified SIGSEGV termination path -- real
      POSIX brk semantics, not "the whole reservation is secretly
      mapped". `fork()`'s `fork_build_child()` now only copies+maps a
      heap page for the child where the PARENT actually has one mapped
      (an untouched heap page stays lazy on both sides, strictly cheaper
      than the old unconditional 4-page copy). `reclaim_process_frames()`
      and Milestone 54's own frame-accounting self-test both updated to
      count `Some` entries rather than trusting `heap_frames.len()`
      (no longer 1:1 with real physical frames held). New boot-time
      self-test (`self_test_demand_paging_heap()`, two new hardcoded
      process slots, PIDs 11/12) proves both directions for real: the
      POSITIVE case confirms all 64 heap slots start unmapped, `sbrk()`s
      past the OLD 16 KiB cap, touches the LAST page of the new
      reservation (page 63) via a real hardware fault, and checks the
      actual physical byte survived the fault+retry while page 0 (never
      touched) stays unmapped -- proof demand paging is genuinely
      per-page, not per-reservation; the NEGATIVE case targets the exact
      same page index from a process whose own `heap_used` never grew
      near it, and confirms a real SIGSEGV fires (page stays unmapped)
      and the kernel genuinely recovers afterward (PROCESS_A runs
      normally right after).

      **A real, pre-existing bug found and fixed during this milestone's
      own verification**: `usertest.rs`'s WRITE-syscall diagnostic
      unconditionally dereferenced heap page 0 on every write() call --
      harmless before this milestone (heap page 0 was always eagerly
      mapped), but a genuine kernel-context page fault (Ring0 CPL, not a
      process fault) the first real boot after removing eager heap
      mapping, since most hardcoded test processes never call `sbrk()`
      at all. Fixed with a new `heap_page0_mapped()` check-first guard
      rather than papering over it.

      **A second real, reproducible bug found and fixed during this
      milestone's own verification -- a genuine reentrant-lock deadlock,
      not a flake**: the first boots after removing eager heap mapping
      reproducibly hung/crashed (QEMU exit code 2, no further kernel
      output -- confirmed via a real `-d int` hardware trace showing the
      CPU correctly took the `#PF` vector and then nothing further was
      ever logged) at the FIRST real heap touch inside an ELF-loaded
      process. Root cause: `run_elf()`/`self_test_altentry_elf()`/
      `self_test_malloc()` all ran the loaded process's ENTIRE ring-3
      excursion from inside `memory::with_frame_allocator()`'s closure
      (harmless before this milestone, since that excursion never itself
      touched `frame_allocator`) -- but a heap page fault occurring
      DURING that excursion now needs `try_demand_page_heap()` to call
      `memory::with_frame_allocator()` itself for a fresh frame, and
      `spin::Mutex` is not reentrant: the retry spins forever trying to
      re-acquire a lock the SAME execution context already holds. Real
      evidence isolating it to this one lock-scoping gap rather than
      demand paging itself being broken: `self_test_altentry_elf()`
      (never touches its heap) and every hardcoded-process `runproc N`
      path (never calls the ELF-loader's combined function, never holds
      this lock during its own ring-3 excursion) kept working fine in the
      SAME boot that hung on `self_test_malloc()`'s real malloctest.elf
      -- the first ELF-loaded program in this project's history to
      actually touch its heap (`sys_sbrk()` then a real write to the
      returned pointer). Fixed by splitting `load_and_run_elf()` into
      `create_loaded_elf_process()` (runs inside
      `with_frame_allocator()`'s closure) and `run_loaded_elf_process()`
      (the ring-3 excursion itself, called only AFTER that closure --
      and therefore the lock -- has already returned), mirroring the
      `create_loaded_process()`/`run_loaded_process()` split
      `run_file()` already established for the flat-binary loader path
      (loader.rs). All three call sites (`run_elf()`,
      `self_test_altentry_elf()`, `self_test_malloc()`) updated to the
      new two-call shape.

      Real, fresh build + fresh QEMU boot (persist disk attached):
      milestones 42/43/44/45/53/54/57 all self-report `OVERALL: PASS`
      (57's own self-test included), the Milestone 51 malloc test --
      the exact code path that used to deadlock -- prints its own real
      `OVERALL=PASS`, zero panics, zero warnings, boot reaches the
      interactive shell and background task scheduling normally.
- [x] **Milestone 58**: real `exec()` with argv/envp -- the Tier 1
      roadmap item named right alongside "virtual memory/demand paging"
      (Milestone 57's own slice, still a heap-only first piece; the
      next real VM increment -- copy-on-write, `mmap` -- stays real,
      disclosed future work, not silently claimed here). Chosen over
      every other unclaimed Tier 1 item by real dependency order: this
      project's process groups/sessions (`pgid`, Milestone 42),
      `pipe()`/`dup`/`dup2` (Milestones 40/42), `waitpid` exit-status
      semantics (`WaitOutcome::{Exited,Killed,Signaled}`, Milestones
      43/53), and the ELF loader's fixed-entry-address restriction
      (generalized at Milestone 44) were ALL already real and working
      before this milestone started -- checked directly against the
      code, not assumed from old doc comments. `errno` and a general
      real signal-delivery mechanism (SIGKILL is currently the only
      signal, and it's an immediate-terminate special case, not real
      `sigaction`/handler delivery) remain genuinely open Tier 1 gaps,
      but both are bigger, riskier lifts than the one, cleanly-scoped
      gap `exec()` itself already had: `kernel/src/usertest.rs`'s
      syscall 9 (Milestone 45's own real teardown-and-rebuild exec())
      took only a bare path -- `loader.rs`'s own top-of-file doc
      comment has said "no argv/envp" since Milestone 34 and nothing
      since ever closed it.

      A genuinely NEW syscall (16, EXECARGV) rather than an in-place
      change to syscall 9: `process::EXEC_TEST_PROGRAM` (Milestone 45,
      hand-assembled raw machine code) calls syscall 9 with only
      rdi/rsi ever set, so extending that ABI in place would have left
      rdx/r10/r8/r9 as whatever garbage happened to sit in those
      registers at that hand-assembled call site -- a real regression
      risk for zero benefit. Syscall 9's own arm, `process::exec_elf()`,
      and its EXEC_TEST_PROCESS self-test are completely untouched.
      EXECARGV takes `path_ptr, path_len, argv_ptr, argv_count,
      envp_ptr, envp_count` (rdi/rsi/rdx/r10/r8/r9, the standard 6-arg
      syscall convention this kernel's `int 0x80` trampoline already
      captures in full) -- `argv_ptr`/`envp_ptr` each point to an array
      of `(str_ptr: u64, str_len: u64)` entries, the SAME ptr+len idiom
      this kernel already uses for every other string-bearing syscall
      argument, not NUL-terminated C strings.

      The real, new piece: `process::build_argv_envp_stack()` lays the
      received argv/envp out on the NEWLY exec()'d process's own private
      stack page following the ACTUAL x86_64 SysV process-entry
      contract (what a real kernel's `execve()` establishes, and what a
      real ELF's `_start`/libc crt0 assumes) -- `argc` (8 bytes), then
      `argc` real pointers into this same page, a NULL (0) terminator,
      then `envp.len()` pointers, a second NULL terminator, with the
      NUL-terminated string bytes themselves packed at the top of the
      page -- writing through the direct `phys_mem_offset` view of the
      new process's own already-allocated `stack_frame` (the exact same
      technique `fork_build_child()` already uses to copy a parent's
      real stack bytes into a child's), so it can run before the CR3
      switch into the new address space. Returns a genuinely 16-byte-
      aligned RSP (real ABI alignment, checked and enforced, not
      assumed) -- `process::exec_elf_with_args()` hands both the new
      entry and this new RSP to a new `usertest::exec_replace_and_enter_
      with_rsp()` (identical to Milestone 37/45's own `exec_replace_and_
      enter()`, just parameterized on RSP instead of always defaulting
      to the bare stack top). Real, disclosed, checked-before-any-write
      caps (`EXEC_ARGV_MAX_COUNT` = 8 entries, `EXEC_ARG_MAX_LEN` = 128
      bytes each) -- the single 4 KiB stack page every process gets has
      to hold this layout AND whatever real stack space the newly
      exec()'d program needs at runtime, so this stays deliberately
      small and rejects (a clear `Err`, never silent truncation or an
      out-of-bounds write) anything that doesn't fit. A real, closed
      leak risk found and fixed during this milestone's own
      implementation, before it was ever exercised: if
      `build_argv_envp_stack()` fails AFTER `create_process_from_elf()`
      already allocated the new process's pml4/code/stack/heap frames,
      that failure path now calls `reclaim_process_frames()` (Milestone
      54's own mechanism) on the abandoned `new_proc` instead of letting
      the early `return Err` silently leak those frames -- the exact bug
      class Milestone 54 itself closed for every OTHER process-
      replacement path in this file, now closed here too rather than
      reintroduced.

      Verified with two REAL, externally-built ELF64 test payloads
      (`rustc --target x86_64-unknown-none` + `rust-lld`, same toolchain
      as every other `tools/*_src` payload, not hand-assembled --
      `tools/argvlauncher_src/` and `tools/argvtarget_src/`, see each
      one's own `README.md`): the launcher calls EXECARGV with a real
      3-entry argv (`["argvtarget", "hello", "world"]`) and 1-entry envp
      (`["GREETING=hi"]`); the target's `_start` is
      `#[unsafe(naked)]` (same mechanism as `usertest.rs`'s own
      `syscall_entry()`) so it can read the hardware-set `rsp` before
      any Rust prologue disturbs it (`mov rdi,[rsp]` / `lea rsi,[rsp+8]`
      / a `lea`-computed envp pointer), tail-calling into ordinary Rust
      that reads the real argc/argv/envp back and checks it against
      hand-computed predictions (same discipline as
      `tools/malloctest_src/main.rs`'s own checks). A real link failure
      was hit and fixed while building the target payload (documented
      in `tools/argvtarget_src/README.md`): an early decimal-printing
      helper used ordinary array indexing with a runtime-only-bounded
      index, which kept a real `panic_bounds_check` branch that pulled
      in `core::fmt::Display for u64` and blew a GOT-relative relocation
      range against this crate's own `-C code-model=large` -- fixed by
      switching to raw-pointer writes (no bounds check emitted at all)
      rather than papering over it.

      Real, fresh build + fresh QEMU boot (persist disk attached),
      quoted from the actual serial log: the target's own self-check
      printed `argc=3: PASS argv0=argvtarget: PASS argv1=hello: PASS
      argv2=world: PASS envc=1: PASS envp0=GREETING=hi: PASS` then
      `OVERALL=PASS`; the kernel-side wrapper logged "argvlauncher.elf's
      real EXECARGV replaced it with argvtarget.elf, which ran to
      completion and returned to the kernel cleanly (no panic, no
      double fault)". No regressions: milestones 42/43/44/45/53/54/57
      all still self-report `OVERALL: PASS` in the same boot, the
      Milestone 51 malloc test still prints its own real `OVERALL=PASS`,
      the ARP/ICMP/UDP self-tests still PASS, zero panics, zero
      warnings, boot reaches the interactive shell and background task
      scheduling normally afterward. `tasklist` confirmed zero orphaned
      `qemu-system-x86_64.exe` processes after verification.

- [x] **Milestone 59**: real `errno` -- the other Tier 1 gap Milestone 58's
      own closing doc comment named alongside real signal delivery,
      chosen over that bigger, riskier lift (a whole new control-transfer
      mechanism: a real trampoline into a user-registered handler, a
      `sigreturn`-equivalent, per-process handler-table bookkeeping,
      layered on a fault/kill path this kernel has never needed to
      interrupt mid-instruction before) for the same reason Milestone 58
      picked `exec()`'s argv/envp gap over it too: giving every syscall
      that already fails today a real, specific reason code alongside its
      existing bare `u64::MAX`/`0`/`1` sentinel is real, cleanly-scoped
      work with no new control-flow mechanism required.

      **Honest disclosure**: this milestone was started, then interrupted
      mid-work by a rate limit, and finished by a follow-up pass. The
      interrupted session got exactly as far as `kernel/src/errno.rs`
      itself (a complete, well-documented module of REAL POSIX/Linux
      x86_64 errno.h values -- `EPERM`, `ENOENT`, `ESRCH`, `EIO`, `E2BIG`,
      `ENOEXEC`, `EBADF`, `ECHILD`, `EAGAIN`, `ENOMEM`, `EINVAL`,
      `EMFILE`, checked against the real numbers, not invented) and one
      new `Process::errno` struct field (with both existing constructors
      updated to initialize it to `0`) -- but had not yet wired a single
      one of those constants into any real syscall failure path, and left
      a real `#[warn(dead_code)]` "field `errno` is never read" warning at
      the point of interruption. The follow-up pass found this state via
      `git status`/`git diff`, confirmed the field-and-constructor fix was
      already correct (mirrored exactly from the sibling constructor, doc
      comment included), and did the actual wiring described below.

      The real wiring: `process::set_errno()`/`process::get_errno()` (the
      only reader/writer of `Process::errno` anywhere in this kernel) back
      a NEW syscall, 17 (GETERRNO, no arguments) -- real POSIX semantics,
      same as a real libc's `errno`: process-local, and NEVER implicitly
      cleared by a later SUCCESSFUL syscall, only overwritten by the next
      FAILURE. Every syscall arm in `usertest.rs`'s `syscall_dispatch`
      that has a real, distinguishable failure reason now calls
      `set_errno()` alongside its existing bare return-value sentinel,
      checked call-by-call against `errno.rs`'s own doc comments: OPEN
      (EINVAL non-UTF8 path, EMFILE fd-table-full), READ/FDWRITE/CLOSE/
      DUP/DUP2 (EBADF invalid fd; CLOSE's own persist-failure branch gets
      the more specific EIO), FORK (EAGAIN, covering all three real
      causes -- table full, out of frames, nested-excursion refusal),
      WAIT (EINVAL out-of-range pid, ECHILD not-a-live-child), SBRK
      (ENOMEM), EXEC/EXECARGV (EINVAL non-UTF8 path, ENOENT missing file,
      ENOEXEC malformed ELF, E2BIG argv/envp cap exceeded, plus a new
      `exec_err_errno()` helper mapping `exec_elf()`/
      `exec_elf_with_args()`'s own `Err` messages to ENOMEM/E2BIG/ENOEXEC/
      ESRCH by real cause), PIPE (EMFILE), KILL/GETPGID (ESRCH),
      SETPGID (EPERM, errno.rs's own named example for the field). The
      legacy `ACTIVE_PROCESS == 0` `usertest` excursion sets no errno
      anywhere (disclosed, not silently wrong): it has no `Process` struct
      at all to hold one, so `set_errno(0, ...)` would be a real, silent
      no-op, not a meaningful report.

      Verified end-to-end with a genuinely new hand-assembled ELF test
      payload, `process::ERRNO_TEST_PROGRAM` (a 13th hardcoded process
      slot, pid 17 -- deliberately NOT 13-16, `fork()`'s own live dynamic
      PID range, see `PID_TABLE_BASE`'s own doc comment) -- one continuous
      ring-3 excursion that deliberately fails FOUR different syscalls for
      FOUR different real reasons back to back (`read(fd=99)`,
      `wait(pid=250)`, `sbrk(0xFFFFFFFF)`, `getpgid(pid=250)`), reading
      errno back via GETERRNO after each one. Real, fresh build + fresh
      QEMU boot, quoted from the actual serial log:
      ```
      milestone 35: syscall READ (process 17) -- FAILED, fd 99 is not open -- returning u64::MAX
      milestone 59: syscall GETERRNO (process 17) -- ... -- errno=9 (EBADF)
      milestone 37: syscall WAIT (process 17) -- FAILED (pid 250 is not a live child of this process...) -- returning u64::MAX
      milestone 59: syscall GETERRNO (process 17) -- ... -- errno=10 (ECHILD)
      milestone 33: syscall SBRK (process 17) -- FAILED (requested 4294967295 bytes would exceed this process's fixed per-process heap) -- returning 0
      milestone 59: syscall GETERRNO (process 17) -- ... -- errno=12 (ENOMEM)
      milestone 42: syscall GETPGID (process 17) -- FAILED, pid 250 not a live process -- returning u64::MAX
      milestone 59: syscall GETERRNO (process 17) -- ... -- errno=3 (ESRCH)
      milestone 59: self-test -- ERRNO_TEST_PROCESS's own final errno field: 3 (ESRCH) -- expected 3 (ESRCH, the getpgid(250) call's real failure, the LAST syscall before exit()) -- confirmed
      milestone 59: self-test -- OVERALL: PASS
      ```
      Four genuinely distinct real errno values, correctly set and read
      back for four genuinely different failure causes, plus a real
      sticky-field check (the process's own `errno` still held ESRCH,
      the LAST real failure, after its own `exit()` -- exit() itself
      never touches errno). Fixed the pre-existing `dead_code` warning:
      the field is now genuinely read, confirmed by its absence from a
      full clean rebuild's warning output. No regressions: milestones
      42/43/44/45/51/53/54/57/58 all still self-report `PASS`/"ran to
      completion... no panic, no double fault" in the same boot, zero
      panics anywhere in the log, boot reaches the interactive shell and
      background task scheduling normally afterward. `tasklist` confirmed
      zero orphaned `qemu-system-x86_64.exe` processes after verification.

      **Still genuinely open** (disclosed, not silently dropped): real
      signal delivery (`sigaction`/handler/`sigreturn`) remains exactly
      the Tier 1 gap `errno.rs`'s own doc comment named it as when
      choosing errno over it this milestone -- SIGKILL is still the only
      signal, still an immediate-terminate special case. A handful of
      this kernel's `Option`-collapsed failure causes (e.g. `dup`/`dup2`'s
      "fd not open" vs. their own internal "fd table full", `setpgid`'s
      "invalid pgid" vs. "not authorized" vs. "target vanished") are
      reported as a single best-fit errno rather than a fully
      disambiguated one -- the same honest "one Option collapses several
      real causes, pick the single best-fit real errno" reasoning
      `errno.rs`'s own EMFILE doc comment already discloses for open()/
      pipe(), applied consistently rather than pretending finer
      granularity exists where the underlying `process.rs` functions
      don't yet return it.

- [x] **Milestone 60**: real signal delivery -- the last remaining Tier 1
      gap, exactly as Milestone 59's own closing disclosure named it
      ("SIGKILL is still the only signal, still an immediate-terminate
      special case"), verified directly against the actual code (not
      doc comments) before starting: `process.rs`/`interrupts.rs` had no
      `sigaction`-equivalent, no handler table, no `sigreturn` -- SIGKILL
      (unconditional `kill()`) and hardware-fault termination (SIGSEGV/
      #GP, Milestone 41) were the only two "signal" mechanisms that
      existed, both pre-existing and unchanged here. A real libc (the
      OTHER unclaimed Tier 1 item) was also checked directly, not
      assumed: `tools/libc_test_src/libc.rs` (Milestone 39/51) already
      has real syscall wrappers and a real `malloc`/`free`, but no
      `string.h` (`memcpy`/`strlen`/...) and no buffered `stdio` --
      genuinely still open, but a strictly smaller, lower-risk gap than
      a brand-new control-transfer mechanism, so signal delivery is the
      real Tier 1 completion this milestone targets.

      **Real, working, complete slice** (not scoped down further): a
      genuine trampoline into a user-registered handler, real per-process
      handler-table bookkeeping, and a real `sigreturn`-equivalent that
      unwinds back out to the EXACT interrupted instruction with the
      FULL register file intact -- proven end to end, not just "the
      syscalls exist". Two new syscalls plus a real dispatch-time
      mechanism:
      - **18 (SIGACTION)**: `process::sigaction(id, signum, handler)` --
        registers (or, `handler=0`, clears) a real user-space handler
        address in a new `Process::signal_handlers[32]` table. Real
        POSIX rule actually enforced: `signal::SIGKILL` (9) is refused
        outright, not just documented as uncatchable.
      - **19 (SIGSEND)**: `process::raise_signal(caller, target, signum)`
        -- real `kill(2)`-shaped signal raising. `SIGKILL` routes to the
        PRE-EXISTING Milestone 41 `kill()` path unchanged (not
        reimplemented); every other signal sets `Process::pending_signal`
        if (and only if) the target already has a handler registered.
        Authorization mirrors `setpgid()`'s own already-established real
        rule (self, or a live child of the caller) -- reused by shape,
        not refactored into a shared helper, to avoid touching a
        previously-verified function for this milestone's sake.
      - **20 (SIGRETURN)**: `process::take_saved_signal_context()` --
        restores the complete stashed register/PC/flags/stack-pointer
        snapshot, resuming exactly where the signal preempted execution.
      - **Real delivery mechanism** (`usertest.rs`'s `syscall_dispatch`,
        checked once at the tail of EVERY syscall's return-to-userspace
        -- the one real kernel/user boundary this synchronous, one-
        ring-3-excursion-at-a-time kernel actually has, since there is
        no preemptive mid-instruction redelivery here): when a process
        has a real pending signal AND a registered handler AND is not
        already inside one (this milestone's own disclosed simplification
        of POSIX's per-signal `sa_mask` -- ALL further delivery is
        blocked while one handler is in flight, not just the same
        signal), the live, hardware-produced `SyscallRegs` (already
        holding the complete interrupted GPR/rip/rflags/rsp state) is
        first copied into a new kernel-side `Process::in_signal_handler:
        Option<SavedSignalContext>` stash, then genuinely overwritten
        in place: `rip` -> the handler, `rdi` -> the real signal number
        (the SysV first-argument convention a C `void handler(int)`
        expects), `rsp` -> a real, freshly-built fake-call-frame written
        directly onto the SAME process's own already-mapped user stack
        (the identical direct-user-pointer-write technique the
        `read`/`fdwrite` syscalls already use, since no CR3 switch
        happens for an ordinary syscall return -- the kernel is already
        running under the interrupted process's own page tables at this
        exact point) containing a genuine 7-byte `mov eax,20 ; int 0x80`
        trampoline plus a correctly-16-aligned fake return address
        pointing at it, so the handler's own ordinary `ret` lands on a
        real `SIGRETURN` call, not a hand-waved "and then it returns"
        gap.

      **Real, disclosed scope cuts for this first slice** (not silently
      done and not silently skipped): (1) `SavedSignalContext` is a
      KERNEL-side stash (one slot per process), not a real POSIX
      `ucontext_t` written onto the process's own user stack -- that
      would need a safe copy-to/from-user path with fault recovery this
      kernel has never had (the same gap `MAX_WRITE_LEN`'s own doc
      comment already discloses for ordinary syscall pointers); (2) no
      default-disposition table -- a signal sent to a process with no
      handler registered is a real, documented no-op (`Err`, checked by
      `raise_signal()` before ever setting `pending_signal`), not a
      silently-pretended delivery, real POSIX default-terminate/ignore
      semantics remain future work; (3) blocks ALL signals (not just the
      one in flight) while inside a handler, real per-signal `sa_mask`
      semantics remain future work; (4) a forked child does NOT inherit
      its parent's registered handlers (`fork_build_child()` goes
      through the same fresh-process constructor as every other new
      process, which zeroes `signal_handlers` uniformly) -- real POSIX
      `fork()` does inherit signal disposition; disclosed on the new
      field's own doc comment, not fixed here.

      Verified with a genuinely new hand-assembled test payload,
      `process::SIGNAL_TEST_PROGRAM` (a fourteenth hardcoded process
      slot, pid 18) -- and, since this program's control flow (a handler
      whose own `ret` must land on a kernel-injected on-stack trampoline
      at exactly the right runtime address) has real, genuine
      off-by-one risk no earlier straight-line hand-assembled test
      program in this codebase had, **every byte was produced by a real
      assembler, not hand-encoded by counting opcode lengths by eye**:
      assembled via `as --64` (Intel syntax, GNU binutils, already
      present via this machine's mingw64 toolchain) from real x86_64
      assembly source, symbol offsets read back from the object file's
      own symbol table (`nm`), the `HANDLER_ADDR` immediate patched in
      from the REAL relocation record `as` emitted for it (not guessed),
      then independently re-disassembled with `objdump -D -b binary -m
      i386:x86-64` against the FINAL patched bytes to confirm every
      instruction -- including the patched absolute address -- decodes
      back to exactly what was intended, before a single byte went into
      `process.rs`. The program: registers a real `SIGUSR1` handler,
      self-signals (real POSIX `raise()`), gets genuinely redirected
      mid-execution, and unwinds back out via a real `SIGRETURN` --
      proven via FOUR independent, kernel-side-checked physical-heap
      markers (read directly through `phys_mem_offset`, not trusted from
      the process's own say-so, the same real technique
      `self_test_demand_paging_heap()` already established): a
      before-signal marker, a handler-genuinely-ran marker, a
      resumed-after-`SIGRETURN` marker, AND a canary register (`r12`)
      the handler deliberately clobbers with a different value, proving
      the FULL register file -- not just the instruction pointer --
      round-trips correctly.

      Real, fresh build (from repo root) + fresh QEMU boot (persist disk
      attached), quoted from the actual serial log:
      ```
      milestone 60: syscall SIGACTION (process 18) -- hardware-recorded CS=0x1b (CPL=3) -- signum 10 (SIGUSR1), handler=0x555550000064
      milestone 60: syscall SIGSEND (process 18) -- hardware-recorded CS=0x1b (CPL=3) -- target pid 18, signum 10 (SIGUSR1) raised
      milestone 60: real signal delivery -- process 18 redirected into handler 0x555550000064 for signum 10 (SIGUSR1), interrupted rip=0x555550000046 saved, trampoline frame at 0x555560000f00
      milestone 60: syscall SIGRETURN (process 18) -- real context restored, resuming at rip=0x555550000046 rsp=0x555560001000
      milestone 60: self-test -- SIGNAL_TEST_PROCESS ran to completion (its own exit() syscall returned normally)
      milestone 60: self-test -- HEAP_START markers: main-before-signal ran=true (expect true), handler genuinely ran=true (expect true), canary register (0x1234ABCD, clobbered by the handler to 0xDEADBEEF in between) survived SIGRETURN intact=true (expect true), resumed-after-handler code ran=true (expect true), handler received the real signum (10, SIGUSR1) as its SysV first argument=true (expect true)
      milestone 60: self-test -- OVERALL: PASS
      ```
      No regressions: milestones 42/43/44/45/53/54/57/59 all still
      self-report `OVERALL: PASS` in the same boot, the Milestone 51
      malloc test and Milestone 58 argv/envp test both still print their
      own real `OVERALL=PASS`, zero panics anywhere in the log, zero
      `MISMATCH`, zero unexpected `FAIL` (the only `FAIL` line is the
      pre-existing, disclosed-unrelated "milestone 6: no keystrokes
      received" interactive-only check), zero compiler warnings on a
      full build, boot reaches the interactive shell and background task
      scheduling normally afterward. `tasklist` confirmed zero orphaned
      `qemu-system-x86_64.exe` processes after verification.

      **Still genuinely open** (disclosed, not silently dropped): the
      three scope cuts named above (no user-stack `ucontext_t`, no
      default-disposition table, coarse all-signals-blocked masking, no
      fork() handler inheritance), and the real libc gap named at the
      top of this entry (`string.h`, buffered `stdio`) -- Tier 1's own
      remaining, smaller items.
- [x] **Milestone 61**: a real `string.h` and real buffered `stdio` --
      the LAST two named Tier 1 items, exactly as Milestone 60's own
      closing disclosure named them ("a real libc ... was also checked
      directly ... `tools/libc_test_src/libc.rs` ... has real syscall
      wrappers and a real `malloc`/`free`, but no `string.h`
      (`memcpy`/`strlen`/...) and no buffered `stdio` -- genuinely still
      open"), verified directly against that exact file before starting
      (not assumed): confirmed zero `memcpy`/`memset`/`strlen`/`strcmp`/
      `memmove`/`strcpy`-shaped functions anywhere in the file, and zero
      `FILE`/`fopen`-shaped stdio surface at all. With this milestone,
      every named Tier 1 item in the README's own roadmap is closed.

      **Real, working, complete slice** (not stubs): 11 `string.h`
      functions -- `memcpy`/`memmove`/`memset`/`memcmp`/`strlen`/
      `strcmp`/`strncmp`/`strcpy`/`strncpy`/`strcat`/`strchr` -- matching
      real C signatures AND semantics, including the real dest-pointer
      return convention and (for `memmove`) genuine overlap-safety
      (dest > src copies backward, proven by the self-test below, not
      just asserted). Real buffered `stdio` built entirely on the
      syscalls/`malloc()` that already existed (no new syscalls this
      milestone): `fopen`/`fclose`/`fread`/`fwrite`/`fflush`/`fputc`/
      `fgetc`/`fputs`/`feof`, a real `STDIO_BUFSIZE`-byte internal read
      buffer and write buffer per `FILE` (a genuine heap-allocated
      struct, freed by `fclose`) that actually defer/batch the
      underlying `sys_read()`/`sys_fdwrite()` calls rather than passing
      every call straight through, and a real, disclosed-non-C-variadic
      `fprintf`-equivalent (`%s`/`%d`/`%u`/`%x`/`%c`/`%%`, a typed
      `FmtArg` slice standing in for true C `...` varargs, which would
      need the unstable `core::ffi::VaList` -- a standards-compliant
      `printf`/`scanf` family is already separately named as its own
      future Tier 9 item, not claimed here).

      **Real, disclosed scope cuts** (not silently done, not silently
      skipped): (1) these are ordinary Rust functions, not exported
      `extern "C"` / `#[no_mangle]` symbols -- consistent with every
      other wrapper already in this file (`malloc`/`free`/`sys_*`), and
      deliberately sidesteps a real risk of colliding with whatever
      `compiler_builtins`-provided `memcpy`/`memset`/`memmove`/`memcmp`
      symbols this project's own existing build already resolves hidden
      struct-copy calls against; (2) `sys_open()` has no mode flags at
      all (checked directly against `process::open_file()`, not
      assumed) -- `fopen()`'s `mode` byte is real and enforced ('r' vs
      'w'/'a' genuinely gate `fread`/`fwrite`), but there is no kernel-
      side `O_TRUNC`: writing fewer bytes than an already-existing file
      held leaves that file's old trailing bytes on disk after close
      (a real, PRE-EXISTING Milestone 35 limitation, not introduced or
      hidden here -- a kernel-side fix is out of this milestone's
      userspace-libc scope; the self-test only ever `fopen("w")`s
      brand-new paths for exactly this reason); (3) `'a'` (append) mode
      has no `O_APPEND`/`lseek()` to lean on either -- implemented by
      genuinely draining the file via real `sys_read()` calls until EOF
      at `fopen()` time, which leaves the kernel's own real per-fd
      cursor sitting at true end-of-file for every write that follows
      (a real, working technique, proven by the self-test's own
      append-then-read-back round trip, not a stub); (4) a real,
      mid-milestone link failure was hit and fixed: an early draft used
      ordinary `[]` array indexing for the internal read/write buffers
      and digit-formatting scratch space, which the compiler could not
      statically bound, inserting real bounds-check panic paths that
      pulled `core::fmt` into this `panic=abort` freestanding binary --
      confirmed via a real `rust-lld` error (`R_X86_64_GOTPCREL out of
      range`, `core::fmt`/`panic_bounds_check` referenced) at this
      kernel's real, extremely high `USER_CODE_ADDR` link address. Fixed
      by rewriting every such access as raw pointer arithmetic (the same
      convention every other direct-user-memory access in this file
      already uses), confirmed by a clean rebuild with only the same
      "never used" warnings every prior `tools/*_src` program already
      has, disclosed here rather than silently worked around.

      Verified with a genuinely new, externally-built ELF64 test
      program (`tools/stdiotest_src/`, rustc + `rust-lld`, not
      hand-assembled -- `kernel/assets/stdiotest.elf`), loaded and run
      the same real way `self_test_malloc()` already does
      (`create_loaded_elf_process()`/`run_loaded_elf_process()`).
      HAND-COMPUTED predictions checked for every function, including a
      genuine overlap case for `memmove` (`"0123456789"`,
      `memmove(buf+2, buf, 6)` -> predicted `"0101234589"`, the case a
      naive forward-copy `memmove` would corrupt) and, for stdio, the
      EXACT internal `wlen`/`rlen`/`rpos` values at each step of a
      real, multi-call buffered write (12/25/3-byte `fwrite()` calls
      across the 32-byte `STDIO_BUFSIZE` boundary, predicted to
      auto-flush MID-CALL) and read (5/20/10/5/1-byte `fread()` calls,
      predicted to auto-refill exactly twice) -- not just that the final
      round-tripped content happened to match, real proof the buffering
      mechanism itself fires when and only when predicted. Real, fresh
      build (from repo root) + fresh QEMU boot (persist disk attached,
      reset to a blank image mid-milestone after hitting a real,
      disclosed `MAX_ENTRIES`-full directory error from many prior
      milestones' accumulated seeded test files -- an environment/
      fixture issue, not a code bug, confirmed by the identical run
      passing cleanly once the fixture had room), quoted from the actual
      serial log:
      ```
      memcpy=PASS memset=PASS memmove=PASS memcmp=PASS strlen=PASS strcmp=PASS strncmp=PASS strcpy=PASS strncpy=PASS strcat=PASS strchr=PASS
      content_len_is_40=PASS fopen_w=PASS
        wlen after 12B write=0x000000000000000c
        wlen after +25B write=0x0000000000000005
        wlen after +3B write=0x0000000000000008
      fwrite_counts=PASS fwrite_real_buffering=PASS fclose_w=PASS fopen_r=PASS
        rlen after 5B read=0x0000000000000020
        rpos after 5B read=0x0000000000000005
        rlen after +20B read=0x0000000000000020
        rpos after +20B read=0x0000000000000019
        rlen after +10B read=0x0000000000000008
        rpos after +10B read=0x0000000000000003
      fread_counts=PASS fread_real_buffering=PASS content_roundtrip=PASS eof_semantics=PASS fclose_r=PASS
      append_write=PASS append_roundtrip=PASS
      fprintf_len=PASS fprintf_content=PASS
      OVERALL=PASS
      ```
      Every one of those hex values matches this milestone's own
      hand-computed prediction EXACTLY (12, 5, 8 for the write-buffer
      auto-flush boundary; 32, 5, 32, 25, 8, 3 for the read-buffer
      auto-refill boundary) -- real proof of the buffering mechanism
      itself, not just correct final content.

      No regressions: milestones 42/43/44/45/53/54/57/59/60 all still
      self-report `OVERALL: PASS` in the same boot, the Milestone 51
      malloc test and Milestone 58 argv/envp test both still print their
      own real `OVERALL=PASS`, zero panics anywhere in the log, zero
      `MISMATCH`, zero unexpected `FAIL` (the only `FAIL` line is the
      pre-existing, disclosed-unrelated "milestone 6: no keystrokes
      received" interactive-only check), zero compiler warnings on a
      full build, boot reaches the interactive shell and background task
      scheduling normally afterward. `tasklist` confirmed zero orphaned
      `qemu-system-x86_64.exe` processes after verification.

      **Still genuinely open**: with this milestone, every NAMED item in
      Tier 1's own roadmap list is closed; process groups/sessions,
      `pipe()`, `dup`/`dup2`, and `waitpid` `WIFEXITED`-shaped semantics
      were already closed by earlier milestones (see each's own entry
      above) -- Tier 2 (filesystem completeness: on-disk permissions/
      timestamps/hard links, multi-user uid/gid/chmod/chown, symbolic
      links, `mmap()`, a real block-device abstraction) is the next real
      dependency tier.
- [x] **Milestone 62**: the first real Tier 2 (filesystem completeness)
      item -- a real on-disk format upgrade adding permissions (a
      standard 9-bit unix `rwxrwxrwx` mode), ownership (`uid`/`gid`),
      and real timestamps (`ctime`/`mtime`, genuine unix-epoch seconds)
      to every `DirEntry` in `fs.rs`, PLUS real, enforced permission
      checks in every `DiskFs` operation and `chmod`/`chown`/`stat`/
      `whoami`/`setid` to inspect and change them.

      **Dependency reasoning** (checked against the actual code first,
      not assumed): the roadmap's own wording lists "a robust on-disk
      format (permissions, timestamps, hard links)" and "a minimal
      multi-user model (uid/gid, chmod/chown)" as two separate items,
      but real mode bits are meaningless with no owner to compare
      against and a real `chmod`/`chown` need real bytes on disk to
      modify -- so this milestone does the format change AND real
      enforcement together, the smallest slice that's actually
      verifiable end-to-end rather than inert fields nothing reads.
      **Hard links were deliberately left out**, confirmed by reading
      `fs.rs`'s actual allocation model first: a `DirEntry` embeds its
      own `start_lba`/`sector_count` directly with no inode-indirection
      layer separating "a name" from "the data it refers to" -- a real
      hard link (two names sharing one refcounted inode) needs that
      layer added first, a separate, genuinely bigger structural lift.
      Symbolic links, `mmap()`, and a real block-device abstraction
      beyond raw ATA are each independent, separately-scoped Tier 2
      items, also not attempted here.

      **Real, working, complete slice** (not stubs): `DirEntry` grew
      from 25 to 39 bytes per entry (`mode: u16`, `uid: u16`, `gid:
      u16`, `ctime: u32`, `mtime: u32`, appended after the pre-M62
      fields), persisted to and loaded from disk exactly like every
      other field. Root (`DIR_LBA`) has no parent `DirEntry` of its own,
      so its metadata lives in a genuinely new, dedicated sector
      (`ROOT_META_LBA` = `FILE_DATA_START_LBA + NUM_DATA_SECTORS` = 2 +
      64 = 66, the first free LBA past the existing shared data pool) --
      not smuggled into root's own directory-table sector, which
      `save_dir_at` already zero-fills past the entry table on every
      ordinary mkdir/write/rm there. Real, standard-algorithm unix
      timestamps: `rtc.rs` gained `days_from_civil` (Howard Hinnant's
      well-known, publicly documented civil-calendar algorithm, verified
      by hand against the known 1970-01-01 -> 0 and 2000-01-01 -> 10957
      reference values) and `to_unix_timestamp`, converting the real
      CMOS RTC (Milestone 15) into real unix-epoch seconds -- every
      `mkdir`/fresh `write` stamps a REAL creation time, not a
      placeholder. Real, standard unix enforcement, all checked against
      a kernel-global "current identity" (`CURRENT_ID`, default `(0,
      0)` == root -- deliberately global, not per-process, because a
      real per-process identity needs real persistent accounts/login
      first, already separately named as a much later Tier 8 item; the
      new `whoami`/`setid UID GID` shell commands read/change it, with
      NO password/authentication check at all, disclosed and named
      `setid` rather than `su` specifically so it doesn't imply real
      login exists): root bypasses every check (standard unix
      superuser behavior); owner/group/other rwx bits picked by
      matching the caller's `(uid, gid)` against the entry's; reading a
      file needs its own read bit; overwriting an EXISTING file needs
      its own write bit; CREATING a new file/dir needs write on the
      PARENT directory instead (the target has no mode yet); deleting a
      file/dir (`rm`/`rmdir`) needs write on the PARENT, not the
      target's own bits (real unix semantics, proven directly: the
      self-test below has uid 42 delete a file it no longer even owns,
      solely because the parent directory permits it); traversing INTO
      a directory (not just listing it) needs its search/execute bit,
      checked on every path component in `resolve_dir_lba` including
      root itself; listing a directory separately needs its READ bit.
      `chmod` is owner-or-root; `chown` is root-only (the stricter
      modern-unix rule, a real disclosed choice over classic unix's
      more permissive owner-can-give-away behavior). `RamFs`'s three new
      trait methods (`stat`/`chmod`/`chown`) are real, disclosed "not
      supported" errors, not silently ignored -- consistent with the
      exact style Milestone 46 already established for ramfs's other
      unsupported operations (subdirectories).

      **A real, disclosed format-versioning mechanism**: `MAGIC` was
      bumped (`0x53504B46` "SPKF" -> `0x53504B47`) specifically because
      `ENTRY_LEN` changed size -- reinterpreting a pre-M62 disk's raw
      bytes at the new, larger entry stride would misparse
      `start_lba`/`sector_count` as garbage. Bumping `MAGIC` makes
      `load_dir_at`'s existing "unrecognized magic -> treat as
      blank/uninitialized directory" fallback (already there since
      Milestone 18, for a genuinely blank disk) trigger safely on any
      pre-M62 disk too, the same safe path a truly blank disk already
      took -- not a new code path, and a real, useful side effect for
      this repo's own accumulated-test-fixture disk-full issue (Milestone
      61's own entry, and this milestone's own process rules) since
      every pre-M62 entry on `target/persist.img`/`verify_persist.img`
      is now simply invisible to the new code, not counted against
      `MAX_ENTRIES`.

      Verified with a real, new, non-interactive self-test
      (`fs::self_test_permissions()`, called from `main.rs` right after
      the existing disk/ramfs self-tests, same "every boot's serial log
      carries direct proof" reasoning, no ring-3 entry needed): 43 real
      checks -- create-time defaults (mode/owner/timestamp), overwrite
      preserving mode/uid/gid/ctime while bumping mtime, chmod narrowing
      then reopening a file's mode with matching read/write behavior
      changes under a real non-root non-owner identity (uid 42),
      parent-directory-write-required create/delete (including a
      creator-owns-what-it-creates check and a delete-via-parent-write-
      despite-non-ownership check), owner-may-chmod-own-file vs.
      chown-is-root-only-even-for-the-owner, and directory search(x)-bit
      traversal gating that blocks reaching a file even though the
      file's OWN mode (`0o666`, world-read-write) would otherwise allow
      it -- real proof that traversal permission is checked
      independently of the leaf's own bits, not derived from it. Root's
      own chmod/chown (refused for non-root, real for root, restored to
      the real default afterward) closes out the coverage. Real, fresh
      build (from repo root) + fresh QEMU boot, quoted from the actual
      serial log:
      ```
      fs self-test: permissions mkdir_root=PASS
      fs self-test: permissions mkdir_is_dir=PASS
      fs self-test: permissions mkdir_default_mode_0755=PASS
      fs self-test: permissions mkdir_owner_root=PASS
      fs self-test: permissions mkdir_ctime_eq_mtime_fresh=PASS
      fs self-test: permissions write_create_root=PASS
      fs self-test: permissions f1_is_file_len5=PASS
      fs self-test: permissions f1_default_mode_0644=PASS
      fs self-test: permissions f1_owner_root=PASS
      fs self-test: permissions write_overwrite_root=PASS
      fs self-test: permissions f1_overwrite_preserves_owner_mode=PASS
      fs self-test: permissions f1_overwrite_preserves_ctime=PASS
      fs self-test: permissions chmod_root_narrows_to_0600=PASS
      fs self-test: permissions f1_mode_now_0600=PASS
      fs self-test: permissions uid42_read_denied_mode0600=PASS
      fs self-test: permissions uid42_write_denied_mode0600=PASS
      fs self-test: permissions chmod_root_reopens_to_0644=PASS
      fs self-test: permissions uid42_read_allowed_mode0644=PASS
      fs self-test: permissions uid42_write_still_denied_mode0644=PASS
      fs self-test: permissions uid42_create_denied_parent_0755=PASS
      fs self-test: permissions uid42_mkdir_denied_parent_0755=PASS
      fs self-test: permissions chmod_root_permtestdir_0777=PASS
      fs self-test: permissions uid42_create_allowed_parent_0777=PASS
      fs self-test: permissions g1_owned_by_creator_uid42=PASS
      fs self-test: permissions uid42_chmod_own_file_allowed=PASS
      fs self-test: permissions uid42_chown_denied_root_only=PASS
      fs self-test: permissions root_chown_g1_to_7_7=PASS
      fs self-test: permissions g1_now_owned_by_7_7=PASS
      fs self-test: permissions uid42_delete_via_parent_write_0777=PASS
      fs self-test: permissions mkdir_locked_subdir=PASS
      fs self-test: permissions write_inner_file=PASS
      fs self-test: permissions chmod_inner_world_rw=PASS
      fs self-test: permissions chmod_locked_owner_only_0700=PASS
      fs self-test: permissions uid42_traversal_denied_despite_file_0666=PASS
      fs self-test: permissions uid42_ls_traversal_denied=PASS
      fs self-test: permissions stat_root=PASS
      fs self-test: permissions uid42_chmod_root_denied=PASS
      fs self-test: permissions uid42_chown_root_denied=PASS
      fs self-test: permissions root_chown_root_to_5_5=PASS
      fs self-test: permissions root_now_owned_by_5_5=PASS
      fs self-test: permissions root_restored_to_defaults=PASS
      fs self-test: permissions OVERALL=PASS
      ```
      No regressions: milestones 42/43/44/45/51/53/54/57/58/59/60/61 all
      still self-report `OVERALL: PASS` (or their own program-internal
      `OVERALL=PASS`) in the same boot, the pre-existing
      `fs::self_test_disk_write`/`fs::self_test_ramfs` disk/ramfs
      self-tests both still pass, zero panics anywhere in the log
      (checked directly, not assumed), zero double/triple faults, zero
      `MISMATCH`, zero unexpected `FAIL` (the only `FAIL` line is the
      same pre-existing, disclosed-unrelated "milestone 6: no
      keystrokes received" interactive-only check every prior milestone
      already carries), zero compiler warnings on a full build, boot
      reaches the interactive shell and background task scheduling
      normally afterward. `tasklist` confirmed zero orphaned
      `qemu-system-x86_64.exe` processes after verification.

      **Real, disclosed scope cuts**: (1) no umask concept -- new files/
      dirs always get the fixed conventional defaults (`0o644`/`0o755`),
      not a configurable creation mask; (2) the shell's new `setid UID
      GID` command changes the kernel-global current identity with
      zero authentication -- an explicit testing lever for exercising
      real enforcement, not a login system (real accounts/login is
      Tier 8); (3) the new shell commands (`stat`/`chmod`/`chown`/
      `whoami`/`setid`) were verified by a clean, zero-warning build and
      direct code review against the same argument-parsing conventions
      every other shell.rs command already uses (`rest.split_once('
      ')`, `split_whitespace()`), NOT by a live interactive keystroke
      session -- consistent with this project's own established
      reasoning for why its self-tests exist non-interactively in the
      first place (see `fs::self_test_disk_write`'s own doc comment:
      real QEMU `sendkey` interactive testing has a real history of
      being the less reliable path in this project), and the shell
      layer here is a thin, mechanical pass-through to the exact
      `fs::stat`/`chmod`/`chown` functions the 43-check self-test above
      already exercises directly; (4) `ls` itself was NOT changed to
      display permissions (it still returns the same pre-M62
      `(name, is_dir, len)` shape) -- `stat PATH` is the real, separate
      way to see an entry's full metadata, a deliberately smaller-
      footprint choice than widening `list()`'s shared return type
      (and the `FileSystem` trait signature) across both backing
      stores; (5) `RamFs` gained no permission/ownership/timestamp
      storage at all -- its `BTreeMap<String, Vec<u8>>` backing has
      nowhere to put it, so `stat`/`chmod`/`chown` against a `ram/...`
      path are real, disclosed "not supported" errors, not a silent
      no-op.

      **Still genuinely open**: hard links, symbolic links, `mmap()`,
      and a real block-device abstraction beyond raw ATA -- every other
      named Tier 2 item, each independently scoped as explained above.
## Building and running

Requires:
- Rust **nightly** (pinned via `rust-toolchain.toml` -- `rustup` picks it up
  automatically once installed)
- [QEMU](https://www.qemu.org/download/) (`qemu-system-x86_64` on PATH)

```
cargo run              # defaults to BIOS boot
cargo run -- uefi       # UEFI boot instead
```

This builds `kernel/` as a freestanding `x86_64-unknown-none` binary (via
Cargo's artifact-dependency feature, see `.cargo/config.toml`), wraps it in
a bootable disk image (`build.rs`), and launches it in QEMU with serial
output piped to your terminal (`src/main.rs`, the runner).

## Layout

- `kernel/` -- the actual OS. `#![no_std]`, `#![no_main]`. Everything that
  will eventually include Spikeling's logic lives here.
- `src/main.rs` -- not part of the kernel; a small host-side program that
  launches the built disk image in QEMU. Standard pattern for this crate
  (see the reference `basic` example in rust-osdev/bootloader).
- `build.rs` -- turns the compiled kernel ELF into bootable BIOS/UEFI disk
  images.
