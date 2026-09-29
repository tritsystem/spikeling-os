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

Showing Milestones 108 onward. **Full history for Milestones 1-107 is in
[MILESTONES.md](MILESTONES.md)** -- moved there because the combined file
exceeded GitHub's ~512KB markdown render limit, which silently truncated
the rendered README around Milestone 109 for anyone viewing the repo page.

- [x] **Milestone 108**: single-dimension fixed-size `int` array
      declarations, indexed reads, and indexed writes (`tools/cc_src/
      main.rs`) -- a real, deliberately narrow first slice of the
      longest-standing gap this file's own "still genuinely open"
      disclosures have named in every compiler-frontend milestone since
      Milestone 83 ("no arrays/pointers"), the same "narrow, honestly
      scoped first slice, not the whole feature at once" discipline
      `char` itself used across Milestone 102-105.

      **Grammar/AST**: two new lexer tokens (`TOK_LBRACKET`/
      `TOK_RBRACKET`, "[" / "]"); a new `parse_array_decl()` parser
      function for `"int" IDENT "[" INTLIT "]" ";"` (the size must be a
      real positive INTLIT, not a general `cond_expr` -- real C requires
      a constant expression for a plain array declarator's own bound
      too, so this is a genuine subset of real C grammar, not an
      invented restriction; `int arr[0];` is a new, real, disclosed
      `ParseError::BadArraySize`), deliberately its OWN standalone parse
      path rather than folded into `make_decl()`'s existing comma-
      separated multi-declarator/initializer chain -- no array
      initializer, no mixing an array declarator into an `int a, b[3],
      c;` list, no `char` arrays (`char` stays scalar-only, unchanged);
      a new `EXPR_INDEX` AST kind for `arr[idx]` as a read (parsed in
      `parse_factor()`'s own `TOK_IDENT` arm, alongside the pre-existing
      call-vs-plain-ident disambiguation, now three-way); and a new
      `STMT_ASSIGN_INDEX` AST kind for `arr[idx] = expr;` as a write --
      deliberately a NEW statement kind rather than a flag bolted onto
      `STMT_ASSIGN` (the same "new kind, not a reused kind with a flag"
      discipline Milestone 99's `STMT_DOWHILE` already chose over a flag
      on `STMT_WHILE`), so `STMT_ASSIGN`'s own codegen and every one of
      its existing callers stay completely untouched -- zero regression
      risk to the plain-scalar-assignment path every prior milestone
      already verified. `idx` is any real `cond_expr` in both positions,
      not just a literal -- CASE 89 (below) indexes with a real
      `while`-loop counter, the real proof this is genuine runtime
      addressing, not literal-index sugar.

      **Stack layout**: `collect_vars_rec()`'s own `STMT_DECL` branch now
      reads a real element count out of the STMT_DECL's own `else_body`
      field (0 for every scalar declarator, unchanged since Milestone 67
      -- reused rather than growing `StmtNode` by a field, the same
      discipline `then_body`'s own Milestone 102 `is_char` reuse already
      established) and reserves that many CONTIGUOUS 8-byte stack slots
      instead of one. Only the array's own first slot (element 0, its
      real base address) becomes a real NAMED `VarSlot` entry
      `find_var()` can match; elements 1..N-1 get real, explicitly-
      written BLANK filler entries (`name_len=0`, unmatchable by any
      real identifier this lexer could ever produce) rather than being
      left as uninitialized `malloc()`ed memory `find_var()`'s own
      linear scan would otherwise read -- real, live undefined behavior
      this milestone deliberately avoids introducing. Writing real blank
      fillers is what keeps `nvars` (this whole codegen's existing dual-
      purpose count -- both "how many real `VarSlot` table entries
      exist" AND "how many 8-byte stack slots this function's frame
      needs", see `gen_function()`/`gen_program()`'s own `frame_bytes =
      (nvars + 1) * 8`) an exactly accurate slot count for a function
      that declares an array, with ZERO changes needed to either of
      those two frame-sizing call sites. `MAX_VARS` (8) now caps total
      SLOTS, not just named variables -- a single array can legally
      consume the whole budget by itself (CASE 88 does, below).

      **Runtime address codegen -- the one genuinely new machine-code
      surface this milestone adds**: every prior variable access this
      codegen has ever emitted, since Milestone 68, used a compile-time-
      constant `disp8` straight off `rbp` (one variable, one fixed
      offset). An array index is a real runtime value, so `EXPR_INDEX`/
      `STMT_ASSIGN_INDEX` instead compute `&arr[0] - idx*8` at runtime
      (element `i`'s real address, since `collect_vars_rec()`'s own
      layout makes each successive element's offset MORE negative, the
      same direction subtracting a larger `idx*8` already moves) via
      four newly hand-verified `CodeBuf` encodings -- `emit_lea_rax_
      rbp_off()` (`LEA r64,[rbp+disp8]`, computing the array's own base
      ADDRESS rather than loading through it), `emit_shl_rcx_imm8()`
      (`SHL r/m64,imm8` applied to RCX, `idx * 8`), `emit_mov_rax_mem_
      rax()` (`MOV RAX,[RAX]`, the real load, reused by `EXPR_INDEX`),
      and `emit_mov_mem_rcx_rax()` (`MOV [RCX],RAX`, the real store,
      used by `STMT_ASSIGN_INDEX`) -- each individually checked against
      the Intel SDM's own encoding tables, the same discipline every
      `CodeBuf` method before it already used. The whole sequence uses
      only RAX/RCX, both already this codegen's own ordinary scratch
      registers (the same real stack-machine "push to save across a
      nested evaluation, pop back into a scratch register" idiom every
      other binary/call node already uses) -- no new callee-saved
      register is introduced anywhere, so this milestone needed zero
      prologue/epilogue changes.

      **New self-test cases** (see `main.rs`'s own top Milestone 108 doc
      comment for the full derivation): CASE 88 (`int arr[8]`, exactly
      `MAX_VARS`, no other locals in the function -- simultaneously the
      real position-correctness proof, the same base-10-digit-weighted-
      return technique CASE 26's `mix4` and CASE 86's `mix8` already
      established so a single scalar result proves every element landed
      in its own correct slot, AND the real `MAX_VARS` boundary-SUCCESS
      proof, one array alone using the entire real 8-slot budget)
      returns `12345678`; CASE 89 (`int arr[5]`, summed via a real
      `while`-loop counter as the index, realistic array usage and the
      real runtime-addressing proof named above) returns `150`; CASE 90
      (`int arr[9]`, `MAX_VARS + 1`, alone in its function) is a real,
      disclosed `CodeGenError::TooManyVars` -- the same "prove the cap
      is real, don't just exercise the happy path" discipline CASE 87
      already established for `MAX_PARAMS`, applied here to `MAX_VARS`
      for the first time ever: `TooManyVars` has existed since Milestone
      67/72 but (checked directly by grepping this file's own history
      for "TooManyVars" before this milestone) had never once been
      exercised by any self-test case in this file until CASE 90.

      **Scope, deliberately real and disclosed**: `int` arrays only (no
      `char[]`); no array initializer lists (`int arr[3] = {1,2,3};` is
      not in this grammar); no multi-dimensional arrays; no arrays as
      function parameters or return values (no array-to-pointer decay
      exists in this subset at all -- there being no pointer type yet
      either, the other half of the "arrays/pointers" gap this
      milestone's own name only partially closes); no compound-
      assignment operators on an array target (`arr[i] += 1;` is real,
      disclosed future work); no bounds checking (an out-of-range index
      reads/writes whatever real stack memory the computed address
      lands on, exactly the same real undefined behavior an unchecked C
      array access already has on genuine hardware -- not a new gap this
      codegen introduces, and consistent with this file's own long-
      standing "no bounds-check panic path, so no `core::fmt` gets
      pulled into this freestanding `panic=abort` binary" reason for
      never having runtime bounds checks anywhere in this codegen); a
      bare array name used as an expression with no index (`return
      arr;`) is not rejected at parse or codegen time -- it silently
      reads whatever is at the array's own element-0 slot as if it were
      an ordinary scalar, a real, disclosed simplification rather than a
      hard error, since detecting it would need a real symbol table
      tracking each name's own declared shape, which this codegen does
      not build (`find_var()`'s own linear scan resolves a name to an
      offset only).

      **No SNN/evidence-gate work this milestone, and correctly so**:
      this is a pure compiler-frontend milestone -- new grammar, new AST
      kinds, new machine-code encodings, no kernel-side subsystem
      decision anywhere in it for an evidence-gated hysteresis gate to
      meaningfully subscribe to. The same "pure compiler-frontend
      features legitimately don't need this" carve-out Milestone 107's
      own `MAX_PARAMS` fix already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero new warnings -- the two pre-existing `dead_code`
      field-never-read warnings on `CodeGenError::ArgCountMismatch(u64)`/
      `UndefinedLabel(u64)` predate this milestone, unchanged, same as
      every milestone since 107 disclosed). `kernel/assets/cc.elf`
      rebuilt via the project's own pinned-nightly `rustc` recipe
      (`tools/cc_src/README.md`, unchanged) -- 87808 bytes, up from
      Milestone 107's 84120; the real `PT_LOAD` `p_memsz` via `readelf
      -l` is 73792 bytes, 19 pages, up from Milestone 107's 17 --
      comfortably under the 64-page per-segment and 128-page total caps,
      no cap change needed. Two real QEMU boots against a genuinely
      fresh disk (`target/persist.img`/`persist2.img` deleted first),
      `cargo run -- bios`, fresh then reused (`m108_fresh_boot.log`/
      `m108_reused_boot.log`), no stray `qemu-system-x86_64.exe`
      process before or after either boot (checked via `tasklist`).
      `OVERALL_M108=PASS` on both boots -- CASE 88 returns `12345678`,
      CASE 89 returns `150`, CASE 90 correctly rejects the 9-element
      array with `TooManyVars`, identically on both boots. Every
      `OVERALL=`/`OVERALL_M*=` marker this kernel has ever produced (36
      total, Milestones 1-66's four aggregate checks through
      `OVERALL_M107` plus this milestone's own new `OVERALL_M108`)
      independently re-confirmed `PASS` on both boots -- checked with a
      script that extracts every real serial `WRITE` syscall's own
      payload from both logs in order and diffs the resulting marker
      list against Milestone 107's own fresh-boot marker list, not
      sampled or eyeballed: all 36 markers matched, all `PASS`, fresh
      and reused byte-identical. No regressions: the reused boot's own
      `fs self-test: permissions`/`symlinks` `FAIL` lines were diffed
      directly (`diff`, not eyeballed) against Milestone 107's own
      reused boot's FAIL-line set -- byte-for-byte identical, the same
      pre-existing, already-disclosed Milestone 62-era fixture-cleanup
      artifact every milestone since 70 has named; the fresh boot's own
      fs self-test passes both suites, matching every prior milestone's
      fresh-boot pattern. Zero `panicked at`/`kernel panicked` and zero
      `DOUBLE FAULT`/`TRIPLE FAULT` in either log (checked directly,
      `grep -c`, zero matches in both).

      **Still genuinely open**: the "arrays/pointers" gap is now only
      HALF closed -- arrays exist, real pointers (declaration,
      dereference, arithmetic) still do not, and neither does array-to-
      pointer decay (so no arrays as function parameters or return
      values yet either). Within arrays themselves: no `char[]`, no
      initializer lists, no multi-dimensional arrays, no compound-
      assignment on an array target, no bounds checking, and a bare
      array name with no index is silently treated as a scalar rather
      than rejected (see this entry's own "Scope" paragraph above for
      the full reasoning on each). `MAX_PARAMS` (8), no `sizeof`, `char`
      as the only non-`int` type, the compiler self-test's own per-
      compile leak, `heap_guard::admit()` covering only its original two
      call sites, and `evidence_gate.rs` still having exactly one real
      subscriber (Milestone 106) all remain exactly as Milestone 107
      left them -- unrelated to this milestone.

- [x] **Milestone 109**: single-level `int` pointer declaration,
      address-of, and dereference (both a real rvalue AND a real lvalue)
      (`tools/cc_src/main.rs`) -- the OTHER half of the "arrays/pointers"
      gap Milestone 108's own closing disclosure named as still open
      ("real pointers (declaration, dereference, arithmetic) still do
      not [exist]"), a real, deliberately narrow first slice, the same
      "narrow, honestly scoped slice, not the whole feature at once"
      discipline `char` (Milestone 102-105) and arrays (Milestone 108)
      both already used.

      **Grammar/AST**: zero new lexer tokens -- `*` and `&` both already
      existed (`TOK_STAR`, multiplication; `TOK_AMP`, bitwise AND, since
      Milestone 79), this milestone only adds new grammar POSITIONS for
      them. `int *p;` (new `parse_pointer_decl()`, its own standalone
      path off `TOK_INT`'s arm in `parse_stmt()`, checked before the
      IDENT is consumed since the `*` sits between the type keyword and
      the name -- the mirror image of the array declarator's own `[`,
      which comes AFTER the name) supports a single pointer declarator
      per statement with an optional `"=" <ternary>` initializer (so
      `int *p = &x;` is real, legal, one statement), reusing
      `finish_ident_assign()` completely unchanged. `&x` (new `EXPR_UNARY`
      op `OP_ADDR`) and `*p` as an rvalue (new `EXPR_UNARY` op
      `OP_DEREF`) both slot into `parse_unary()` at the exact same
      tightest-precedence unary position `-`/`~`/`!` already occupy;
      `&` is deliberately NARROWER than every other unary operator here
      -- its operand must be a single plain IDENT (`self.expect(TOK_IDENT)`
      directly, no recursion into `parse_unary()`/`parse_factor()` at
      all), so `&arr[i]`, `&*p`, and `&(expr)` are all real, disclosed
      non-grammar, not silently-wrong parses. `*` (dereference), by
      contrast, DOES recurse into `parse_unary()` exactly like
      `-`/`~`/`!`, so it composes with them and with itself (`-*p`,
      `!*p`, etc. all parse). Neither `*` nor `&` is ambiguous with
      their own pre-existing binary meanings (multiplication, bitwise
      AND): `parse_term()`/`parse_bit_and()` both always parse their
      LEFT operand (through `parse_unary()`) before ever checking for a
      following `*`/`&` as an infix operator, so a `*`/`&` reached at
      the very start of `parse_unary()` has, by construction, no left
      operand yet -- it can only be the prefix form. This is the exact
      same "resolved by grammar POSITION, not by the token alone"
      reasoning this file's own Milestone 76 doc comment already
      established for unary vs. binary `-`, now reused for two more
      operators rather than re-derived from scratch. `*p = expr;` (new
      statement kind `STMT_ASSIGN_DEREF`, the same "new kind over a
      reused kind with a flag" choice `STMT_ASSIGN_INDEX`/`STMT_DOWHILE`
      already made) is dispatched at the very START of a statement --
      unambiguous with `parse_unary()`'s own `*` handling, since
      `parse_stmt()`'s top-level match only ever sees the first token of
      a brand-new statement, while `parse_unary()`'s `*` case only ever
      runs from somewhere already inside expression parsing.

      **The real design choice this milestone hinges on**: `int *p;`
      produces an ORDINARY scalar `STMT_DECL`, byte-identical in shape to
      a plain `int p;` -- a pointer's own runtime VALUE is just another
      8-byte stack slot holding an address, exactly like an ordinary
      `int` holds a number. `collect_vars_rec()`/`find_var()`/`VarSlot`
      needed no new field and no new stack-layout logic at all (checked
      directly: this milestone touches ZERO lines in either function).
      The only genuinely new codegen lives entirely in `EXPR_UNARY`'s two
      new op cases and the new `STMT_ASSIGN_DEREF` statement kind, and
      **all three reuse Milestone 108's own three array-address CodeBuf
      encodings completely unchanged -- this milestone needed ZERO new
      x86_64 encodings**: `&x` emits exactly `emit_lea_rax_rbp_off()`
      (Milestone 108's own array-base-address LEA, an ordinary scalar's
      `rbp`-relative slot being exactly as valid an LEA operand as an
      array's element-0 slot already was); `*p` as an rvalue emits
      exactly `emit_mov_rax_mem_rax()` (Milestone 108's own
      array-element-load `MOV RAX,[RAX]`) after evaluating `p` for its
      value; `*p = expr;` emits `emit_mov_rax_rbp_off()` (read `p`'s own
      value, ordinary and pre-existing since Milestone 68) then
      `emit_mov_mem_rcx_rax()` (Milestone 108's own array-element-store).
      This is not a coincidence: a single-level pointer read/write is,
      mechanically, exactly an array-element read/write with the index
      computation folded away -- the target address comes directly from
      a variable's own value instead of a runtime `base - idx*8`
      computation -- so the exact three encodings Milestone 108 already
      hand-verified against the Intel SDM cover the complete real
      machine-code surface this milestone needs.

      **New self-test cases**, all four run in-process
      (`CodegenMode::Callable`), the same verification tier Milestone
      107/108 both already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle is added by this milestone: CASE
      91 (`int *p = &x; return *p;`, the combined declaration+initializer
      form, exercising address-of AND dereference-read together, expected
      `10`); CASE 92 (`*p = 99; return x;` -- the real, DISTINGUISHING
      proof that a dereference write mutates the pointee's own real stack
      slot, genuine aliasing, not an independent copy `p` happens to hold
      -- a shallow/buggy implementation could still pass CASE 91 via a
      fresh read back through the same pointer but would fail this one,
      since `x` itself would stay `5` if the write did not really land at
      `x`'s own address, expected `99`); CASE 93 (a real "swap two
      variables via two separate pointers" idiom -- `pa`/`pb` must each
      hold their own distinct address, combining address-of AND both
      dereference-read AND dereference-write in one program, the
      strongest realistic composition proof this milestone's self-test
      offers, expected `703`, i.e. `a` becomes 7 and `b` becomes 3); CASE
      94 (`return &5;` -- the real, disclosed proof that `&`'s own narrow
      "plain IDENT only" operand restriction is real, not just
      documented, a genuine `ParseError` since `parse_unary()`'s `&` arm
      calls `self.expect(TOK_IDENT)` directly rather than recursing into
      the general expression grammar -- the same "prove the scope
      restriction is real" discipline CASE 87/90 already established for
      `MAX_PARAMS`/`MAX_VARS`).

      **Scope, deliberately real and disclosed**: single-level pointers
      to `int` only (no `char *`, no `int **`); NO pointer arithmetic
      (`p + 1` compiles as an ordinary integer addition on a pointer's
      own raw address VALUE -- adds to it, UNSCALED, exactly the "real
      integer op, no type-aware scaling" behavior this codegen already
      gives every other value, since no symbol table anywhere in this
      file tracks a variable's own declared shape for `+`'s codegen to
      consult -- not rejected, not silently made "safe"; a real,
      disclosed gap, the exact same "no shape-tracking, so no
      shape-aware rejection" precedent Milestone 108's own bare-array-
      name simplification already established, not new here); NO
      array-to-pointer decay (so still no arrays as function parameters
      or return values); NO pointers as function parameters or return
      values either (a real, deliberate scope cut for THIS milestone --
      the calling-convention plumbing Milestone 72/107 built has no
      structural reason it couldn't carry a pointer value exactly like
      any other `int`-sized value, this just was not exercised or
      verified here, disclosed future work); `&`'s own operand restricted
      to a single plain IDENT (no `&arr[i]`, no `&*p`, no `&(expr)`); no
      `*p += 1;`-style compound assignment through a pointer (the same
      real gap Milestone 108 already left open for `arr[i] += 1;`, now
      disclosed again for the pointer case, not newly introduced here);
      no NULL-pointer concept and no bounds/validity checking of any kind
      on a dereference (an uninitialized or wild pointer value
      dereferences to whatever real memory the computed address lands
      on, the same real undefined behavior an unchecked C pointer
      dereference already has on genuine hardware -- consistent with
      this file's own long-standing "no bounds-check panic path, so no
      `core::fmt` gets pulled into this freestanding `panic=abort`
      binary" reason for never having any runtime safety check anywhere
      in this codegen, Milestone 108's own array-bounds disclosure being
      the most recent precedent, not a new reason invented here).

      **No SNN/evidence-gate work this milestone, and correctly so**:
      this is a pure compiler-frontend milestone -- new grammar, new AST
      kinds, new grammar POSITIONS for existing machine-code encodings,
      no kernel-side subsystem decision anywhere in it for an
      evidence-gated hysteresis gate to meaningfully subscribe to. The
      same "pure compiler-frontend features legitimately don't need this"
      carve-out Milestone 107/108 both already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 91104 bytes, up from Milestone 108's
      87808; the real `PT_LOAD` `p_memsz` via `readelf -l` is 76648
      bytes, still 19 pages (the same page count as Milestone 108, just
      more bytes within it) -- comfortably under the 64-page per-segment
      cap and 128-page total cap, no cap change needed. The rebuilt
      `cc.elf` was independently re-derived from the current `main.rs`
      via a from-scratch invocation of the exact same recipe and diffed
      byte-for-byte identical against the committed binary (`cmp`, not
      eyeballed), so the checked-in artifact is genuinely reproducible,
      not stale or hand-edited. Zero new compiler warnings: verified by
      reproducing the standalone `rustc` invocation against a `git stash`
      of this milestone's own changes (a real, clean Milestone 108
      checkout) and diffing the sorted warning list against this
      milestone's own build -- byte-for-byte identical 15-warning set
      (the two pre-existing `dead_code` field-never-read warnings, 12
      pre-existing `libc.rs` unused-function warnings including
      `sys_open` -- confirmed already unused at the Milestone 108
      baseline too, not a regression this milestone introduced -- and one
      pre-existing `unnecessary unsafe block` warning, all predating this
      milestone).

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m109_fresh_boot.log`/`m109_reused_boot.log`),
      no stray `qemu-system-x86_64.exe` process before or after either
      boot (checked via `tasklist`). `OVERALL_M109=PASS` on both boots --
      CASE 91 returns `10`, CASE 92 returns `99`, CASE 93 returns `703`,
      CASE 94 correctly rejects `&5`, identically on both boots. Every
      marker this kernel has ever produced was independently
      re-confirmed `PASS` on both boots via a real script (not sampled or
      eyeballed) that reconstructs the actual WRITE-syscall payload
      stream from each raw serial log (`milestone 31: syscall WRITE ...
      raw bytes >>> ... <<< end of write` blocks, concatenated in order)
      to recover every `OVERALL`/`OVERALL_M*=` marker cc.elf's own
      self-test ever prints, AND separately greps the raw log for the
      kernel's own older `milestone NN: self-test -- OVERALL: PASS`
      marker style (Milestone 42/43/44/45/53/54/57/59/60/64/65/66) that
      prints directly to the serial port rather than through a ring-3
      WRITE syscall -- 46 total distinct markers across both styles, all
      `PASS`, fresh and reused in exact agreement. No regressions: the
      reused boot's own `fs self-test: permissions`/`symlinks` FAIL-line
      set was diffed directly (`diff`, not eyeballed) against Milestone
      108's own reused boot's FAIL-line set -- byte-for-byte identical,
      54 lines, the same pre-existing, already-disclosed Milestone
      62-era fixture-cleanup artifact every milestone since 70 has named;
      the fresh boot's own fs self-test passes both suites cleanly (zero
      FAIL lines). `run_preemption_demo()` ran to completion on BOTH
      boots (fresh: `task0=2578 task1=2555 task2=2851`; reused:
      `task0=2651 task1=2519 task2=2920` -- real inter-boot timing
      jitter, consistent with every prior milestone's own two-boot
      pairs) -- zero `run_preemption_demo() SKIPPED` lines on either
      boot. Milestone 104's own heap-pressure evidence-gate state came
      out BYTE-IDENTICAL to Milestone 108's own recorded baseline on both
      boots: `samples_taken=68 gate_trip_episodes=0
      naive_instantaneous_hits=0 admissions_denied_so_far=0` -- zero
      drift, this milestone's own new pointer codegen adds no kernel-heap
      pressure of any kind (it is pure compiler-frontend work, no new
      kernel allocation path). Zero `panicked at`/`kernel panicked` and
      zero `DOUBLE FAULT`/`TRIPLE FAULT` in either log (checked directly,
      `grep -c`, zero matches in both).

      **Still genuinely open**: the "arrays/pointers" gap is now closed
      at a first-slice level on BOTH halves (arrays since Milestone 108,
      single-level `int` pointers since this milestone) but neither is
      complete real C: no `char *`, no `int **`/multi-level pointers, no
      pointer arithmetic, no array-to-pointer decay, no pointers (or
      arrays) as function parameters or return values, no `*p += 1;`-
      style compound assignment through a pointer, `&`'s own operand
      restricted to a single plain IDENT, no NULL-pointer concept, and no
      bounds/validity checking on any dereference (see this entry's own
      "Scope" paragraph above for the full reasoning on each). Within
      arrays specifically, everything Milestone 108 already disclosed
      remains open and untouched by this milestone. `MAX_PARAMS` (8), no
      `sizeof`, `char` as the only non-`int` scalar type, the compiler
      self-test's own per-compile leak, `heap_guard::admit()` covering
      only its original two call sites, and `evidence_gate.rs` still
      having exactly one real subscriber (Milestone 106) all remain
      exactly as Milestone 108 left them -- unrelated to this milestone.

- [x] **Milestone 110**: real array-to-pointer decay, and pointers --
      and, through decay, arrays -- crossing a real function-call
      boundary for the first time (`tools/cc_src/main.rs`) -- picking up
      exactly where Milestone 109's own closing disclosure left off ("no
      array-to-pointer decay ... no pointers as function parameters or
      return values either"), and directly following this project's own
      standing candidate list, which named this ordering explicitly
      ("likely depends on #1 [pointers] existing first").

      **Grammar delta**: `int *p` is now a legal parameter declarator
      (`parse_params()`) and `int *` is now a legal function return type
      (`parse_function()`) -- both accept a single optional `*` right
      after `int` (never after `char`, matching every other `char`-
      pointer restriction this file already carries). Neither needs a
      new `ParamInfo`/`FuncDef` field or any codegen change at all: a
      pointer value is register/stack-passed and returned exactly like
      an ordinary `int` (Milestone 109's own "a pointer's runtime value
      is just another 8-byte value" design point, now reaching two more
      real call sites), so accepting the grammar IS the complete feature
      on both sides.

      **The real new codegen -- a single new `VarSlot` field**:
      `is_array`, set by `collect_vars_rec()`'s own Milestone 108 branch
      (1 for an array's element-0 slot, 0 for every scalar, pointer,
      parameter, and array filler entry), threaded through `find_var()`'s
      now-three-element return tuple to all six of its call sites. Two
      real behavior changes follow: **(1) `EXPR_IDENT` now DECAYS a bare
      array name** -- computes the slot's own ADDRESS
      (`emit_lea_rax_rbp_off()`, Milestone 108's own array-base
      encoding, reused unchanged) instead of loading the 8-byte VALUE
      sitting in element 0. Since `EXPR_CALL`'s own argument codegen is
      ordinary `gen_expr()` recursion with zero special-casing, `foo(arr)`
      now passes a real address automatically -- no new call-site
      codegen needed anywhere. This closes, as a genuine side effect
      rather than a second deliberate change, Milestone 108's own
      disclosed "a bare array name silently reads its own element-0 slot
      as an ordinary scalar" simplification. **(2) `EXPR_INDEX`/
      `STMT_ASSIGN_INDEX` (`p[idx]`/`p[idx] = expr;`) now compute their
      base address two different ways**: an ARRAY's base is the slot's
      own address (`emit_lea_rax_rbp_off()`, unchanged from Milestone
      108); a POINTER's base is the slot's own VALUE
      (`emit_mov_rax_rbp_off()`, an ordinary scalar read, reused
      unchanged) -- real C's own `p[idx] == *(p+idx)` rule, now
      genuinely true here for a pointer variable too, not only a
      declared array, which is what lets a callee subscript a decayed
      array argument with ordinary `p[idx]` syntax. **Zero new x86_64
      encodings**: the fourth milestone in a row (108, 109, now 110) to
      add real new capability purely by branching among machine-code
      sequences Milestone 108 already hand-verified against the Intel
      SDM.

      **A subtlety worth naming plainly**: both index-arithmetic
      sequences still SUBTRACT `idx*8` from the base, unchanged from
      Milestone 108. This is not an oversight carried over by inertia --
      Milestone 108's own array layout grows toward LOWER real addresses
      as the index grows (a direct, already-disclosed consequence of
      this codegen's downward-growing stack frame, not real C's usual
      "addresses increase with index" convention), and since the ONLY
      real source of a multi-element address this subset can ever
      produce is exactly such a stack array (no heap/`malloc`-backed
      array exists anywhere in this language), a pointer's own value --
      whenever obtained by decaying a real array argument -- already
      points at memory obeying that same "index grows, address shrinks"
      layout. Subscripting through the pointer with the identical
      subtract-based arithmetic is therefore what makes `p[idx]` inside
      a callee land on the EXACT same bytes `arr[idx]` occupies inside
      the caller -- the two conventions made to agree by construction,
      not a coincidental reuse of convenient code.

      **New self-test cases**, all three run in-process
      (`CodegenMode::Callable`), the same verification tier Milestone
      107/108/109 all already used for their own new cases -- no new
      real `fork()`/`exec()`/`wait()` cycle is added by this milestone:
      CASE 95 (`sum3(arr)` -- a declared array decays to a real address
      at the call site, and the callee subscripts through the resulting
      pointer parameter with ordinary `p[idx]` reads, proving both
      halves of this milestone at once, expected `60`); CASE 96
      (`zero_out(arr)` writes through `p[idx]` inside the callee -- the
      real, distinguishing proof that decay hands the callee a genuine
      address into the CALLER's own stack frame, real aliasing across a
      function-call boundary rather than a copy: a shallow/buggy
      implementation could still pass CASE 95's read-only sum but would
      fail this one, since the caller's own array would stay unchanged
      if the callee's writes did not really land at its real addresses,
      expected `0`); CASE 97 (`*identity(&x)` -- a real pointer
      parameter passed straight through and handed back as a real
      pointer RETURN type, `int *identity(int *p) { return p; }`, then
      dereferenced directly off the call expression's own result,
      expected `42`).

      **A real, honest process note**: this milestone's own first draft
      of CASE 96 called `zero_out(arr);` as a bare statement, ignoring
      its return value -- and hit a real, pre-existing, already-
      disclosed limitation of this subset-C head-on: this grammar has
      never had an "expression statement" production at all (quoted
      directly from this file's own top doc comment, present since
      Milestone 67: "no function may be called as a bare expression-
      statement ... an existing gap since Milestone 67, unwidened
      here"), so `lex_and_parse_program()` correctly rejected it. Not a
      new finding and not a bug this milestone introduced -- fixed by
      capturing the call's result into an unused local (`r =
      zero_out(arr);`), exactly the same pattern every prior milestone's
      own call-expression self-test cases already use, and disclosed
      here rather than silently patched around without comment.

      **Scope, deliberately real and disclosed**: `int` only (no `char *`
      parameters/returns); a pointer parameter's own declarator is
      `int *name` only, no `int name[]` bracket-array parameter sugar
      (real C treats the two as equivalent; this subset accepts only the
      pointer spelling, a genuine, deliberate narrowing); no multi-level
      pointer parameters/returns (`int **`, unreachable anyway --
      Milestone 109 never added `int **` either); still no pointer
      arithmetic, so passing a partial array (e.g. `&arr[2]`, to start a
      callee midway through) is not reachable (`&arr[2]` itself remains
      real, disclosed non-grammar, Milestone 109's own `&` restriction,
      untouched here); no new error path this milestone adds (this
      codegen still has no type system of any kind, so passing a plain
      `int` where a pointer is semantically expected, or vice versa, is
      compiled without complaint exactly as it already silently was --
      the same "no shape-tracking, so no shape-aware rejection"
      limitation Milestone 108/109 both already disclosed, not new
      here); and a real, newly-disclosed ASYMMETRY this milestone did
      NOT resolve -- `arr = expr;` (assigning directly to a bare array
      name, always silently legal since Milestone 108, since
      `STMT_ASSIGN`'s own codegen never consults `is_array`) still
      overwrites element 0's raw VALUE unchanged, even though READING
      `arr` now decays to its address -- a real, honest gap for a future
      milestone to either reject at codegen time or otherwise address,
      not silently smoothed over here.

      **No SNN/evidence-gate work this milestone, and correctly so**:
      this is a pure compiler-frontend milestone -- new grammar
      positions, one new `VarSlot` field, new branching among already-
      hand-verified machine-code sequences, no kernel-side subsystem
      decision anywhere in it for an evidence-gated hysteresis gate to
      meaningfully subscribe to. The same "pure compiler-frontend
      features legitimately don't need this" carve-out Milestone
      107/108/109 all already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 92720 bytes, up from Milestone 109's
      91104; the real `PT_LOAD` `p_memsz` via `readelf -l` is 78264
      bytes, 20 pages, up from Milestone 109's own 19 -- comfortably
      under the 64-page per-segment cap and 128-page total cap, no cap
      change needed. The rebuilt `cc.elf` was independently re-derived
      from the current `main.rs` via a from-scratch invocation of the
      exact same recipe and diffed byte-for-byte identical against the
      committed binary (`cmp`, not eyeballed). Zero new compiler
      warnings: verified the same way Milestone 109 verified its own --
      reproducing the standalone `rustc` invocation against a `git
      stash` of this milestone's own changes (a real, clean Milestone
      109 checkout) and diffing the sorted warning list against this
      milestone's own build -- byte-for-byte identical 15-warning set.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m110_fresh_boot.log`/`m110_reused_boot.log`),
      no stray `qemu-system-x86_64.exe` process before or after either
      boot (checked via `tasklist`). `OVERALL_M110=PASS` on both boots --
      CASE 95 returns `60`, CASE 96 returns `0`, CASE 97 returns `42`,
      identically on both boots. Every marker this kernel has ever
      produced was independently re-confirmed `PASS` on both boots via
      the same real script Milestone 109 introduced (reconstructs the
      actual WRITE-syscall payload stream from each raw serial log to
      recover every `OVERALL`/`OVERALL_M*=` marker, plus a separate raw-
      text scan for the kernel's own older `milestone NN: self-test --
      OVERALL: PASS` style) -- 47 total distinct markers across both
      styles, all `PASS`, fresh and reused in exact agreement. No
      regressions: the reused boot's own `fs self-test: permissions`/
      `symlinks` FAIL-line set was diffed directly against Milestone
      109's own reused boot's FAIL-line set -- byte-for-byte identical,
      54 lines, the same pre-existing, already-disclosed Milestone
      62-era fixture-cleanup artifact every milestone since 70 has
      named; the fresh boot's own fs self-test passes both suites
      cleanly (zero FAIL lines). `run_preemption_demo()` ran to
      completion on BOTH boots (fresh: `task0=2600 task1=2402
      task2=2846`; reused: `task0=2646 task1=2505 task2=2769` -- real
      inter-boot timing jitter, consistent with every prior milestone's
      own two-boot pairs) -- zero `run_preemption_demo() SKIPPED` lines
      on either boot. Milestone 104's own heap-pressure evidence-gate
      state came out BYTE-IDENTICAL to Milestone 108/109's own recorded
      baseline on both boots: `samples_taken=68 gate_trip_episodes=0
      naive_instantaneous_hits=0 admissions_denied_so_far=0` -- zero
      drift, consistent with this milestone being pure compiler-frontend
      work with no new kernel allocation path. Zero `panicked at`/
      `kernel panicked` and zero `DOUBLE FAULT`/`TRIPLE FAULT` in either
      log (checked directly, `grep -c`, zero matches in both).

      **Still genuinely open**: no `char *` parameters/returns; no
      `int name[]` bracket-array parameter sugar (pointer spelling
      only); no multi-level pointers anywhere (`int **`); still no
      pointer arithmetic (so no partial-array arguments like `&arr[2]`);
      no type system of any kind, so no shape-aware rejection of a
      mismatched argument/return; and the real, newly-disclosed `arr =
      expr;` read/write asymmetry named above (reading decays, writing
      still overwrites element 0 directly). Everything Milestone 108/109
      already disclosed that this milestone didn't touch remains open:
      no `char[]`, no array initializer lists, no multi-dimensional
      arrays, no bounds checking, no `*p += 1;`/`arr[i] += 1;` compound
      assignment, no NULL-pointer concept. `MAX_PARAMS` (8), no
      `sizeof`, `char` as the only non-`int` scalar type, the compiler
      self-test's own per-compile leak, `heap_guard::admit()` covering
      only its original two call sites, and `evidence_gate.rs` still
      having exactly one real subscriber (Milestone 106) all remain
      exactly as Milestone 109 left them -- unrelated to this milestone.

- [x] **Milestone 111**: resolves the exact `arr = expr;` read/write
      asymmetry Milestone 110's own closing disclosure named as still
      open (`tools/cc_src/main.rs`) -- the second of the two candidates
      that milestone's own README entry offered next, `char *` pointers
      being the other. No grammar change at all: `arr = expr;` already
      parsed fine (a bare array name is an ordinary IDENT to the
      grammar), the bug was entirely in what `STMT_ASSIGN`'s codegen did
      with it -- since Milestone 108 it silently overwrote the array's
      own element-0 slot with the RHS's raw 8-byte value, even after
      Milestone 110 made READING that same bare name decay to its own
      ADDRESS. Real C's own answer to "assign to an array" is "not a
      modifiable lvalue, reject it" -- this milestone makes the
      identical call rather than inventing whole-array-copy semantics
      this subset's flat 8-byte store was never built to perform.

      **The real fix -- one new check in one existing codegen arm**:
      `STMT_ASSIGN`'s arm in `gen_stmt_list()` now looks up the target's
      `is_array` flag (`find_var()`'s own Milestone 110 third field,
      already threaded everywhere -- no new plumbing needed at all)
      BEFORE compiling the RHS, mirroring `STMT_ASSIGN_INDEX`'s own
      existing find-slot-first shape immediately below it in the same
      match. If `is_array != 0`, codegen now returns a new
      `Err(CodeGenError::ArrayNotAssignable(u64))` (carrying the array
      name's own source byte offset, the same "real Err path, carries a
      source offset" convention `UndeclaredVariable`/`UndefinedLabel`
      already use) instead of ever reaching the store -- and instead of
      first wastefully compiling the RHS only to throw the whole buffer
      away, the same real-not-cosmetic reordering benefit
      `STMT_ASSIGN_INDEX` already had. **Zero new x86_64 encodings**:
      this milestone doesn't add codegen, it REMOVES a reachable path
      through existing codegen -- the fifth milestone in a row (107
      through 111) to touch this file without a new machine-code
      sequence, though for once not because a new capability reuses old
      encodings, but because this one is purely subtractive.

      **A real, distinguishing bonus this fix reaches for free**:
      Milestone 84's nine compound-assignment operators (`+=`, `-=`, ...)
      desugar to an ordinary `STMT_ASSIGN` at PARSE time
      (`parse_assign_stmt()`, completely unchanged by this milestone) --
      so `arr += 1;` was ALSO silently corrupting element 0 before this
      fix, a second real instance of the same underlying bug this
      milestone's own top doc comment did not originally name but the
      fix closes as a genuine side effect of living in the one shared
      codegen arm, not a second deliberate change requiring its own
      check.

      **New self-test cases**, both run at the `lex_and_parse()` +
      `gen_function()` tier (`CodegenMode::Callable`), the same tier
      CASE 59 (`UndefinedLabel`) already used for its own new error
      variant -- no new real `fork()`/`exec()`/`wait()` cycle is added
      by this milestone: CASE 98 (`int arr[3]; arr[0] = 1; arr = 5;
      return arr[0];` -- a direct bare-array assignment, must be
      `Err(CodeGenError::ArrayNotAssignable)`, the same "prove the new
      Err path is real, don't just document it" discipline CASE 7/49/59
      already established for their own new error variants); CASE 99
      (`int arr[3]; arr[0] = 1; arr += 5; return arr[0];` -- the real,
      distinguishing proof the fix lives in `STMT_ASSIGN`'s one SHARED
      codegen arm rather than a hypothetical direct-assignment-only
      check this case alone would expose as incomplete: a fix bolted
      onto some separate, imagined compound-assignment code path -- there
      isn't one -- could still pass CASE 98 but would fail this one,
      since compound-assignment reaches codegen through the identical
      desugared `STMT_ASSIGN` node).

      **Scope, deliberately real and disclosed**: this milestone
      resolves exactly the one named asymmetry, nothing broader -- every
      other array/pointer gap Milestone 108/109/110 already disclosed
      remains open and untouched: no `char *`, no `int **`/multi-level
      pointers, no pointer arithmetic, no `int name[]` bracket-array
      parameter sugar, no `*p += 1;`/`arr[i] += 1;`-style compound
      assignment through a dereference or index (an ELEMENT write, e.g.
      `arr[0] += 1;`, is unaffected by this milestone -- it never went
      through `STMT_ASSIGN` at all, only `STMT_ASSIGN_INDEX`, which
      Milestone 110 already made correctly `is_array`-aware for its own
      base-address computation), no NULL-pointer concept, no
      bounds/validity checking on any dereference or index. This
      milestone also does not attempt to make an array a real assignable
      value via memberwise/whole-array copy (real C doesn't either) --
      rejection, not new copy semantics, is the deliberate, disclosed
      choice, matching the "narrow, honestly scoped slice" discipline
      every array/pointer milestone since 108 has used.

      **No SNN/evidence-gate work this milestone, and correctly so**:
      this is a pure compiler-frontend milestone -- one new `CodeGenError`
      variant, one new check in one existing codegen arm, no kernel-side
      subsystem decision anywhere in it for an evidence-gated hysteresis
      gate to meaningfully subscribe to. The same "pure compiler-frontend
      features legitimately don't need this" carve-out Milestone
      107/108/109/110 all already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 94032 bytes, up from Milestone 110's
      92720; the real `PT_LOAD` `p_memsz` via `readelf -l` is 79576
      bytes, still 20 pages (the same page count as Milestone 110, just
      more bytes within it) -- comfortably under the 64-page per-segment
      cap and 128-page total cap, no cap change needed. The rebuilt
      `cc.elf` was independently re-derived from the current `main.rs`
      via a from-scratch invocation of the exact same recipe and diffed
      byte-for-byte identical against the committed binary (`cmp`, not
      eyeballed). **One real, honestly-disclosed NEW compiler warning**,
      verified the same way Milestone 109/110 verified their own zero-new-
      warning result -- reproducing the standalone `rustc` invocation
      against a `git stash` of this milestone's own changes (a real,
      clean Milestone 110 checkout) and diffing the sorted warning list
      against this milestone's own build: 15 warnings at the Milestone
      110 baseline, 16 at this milestone's own build, the delta being
      exactly one new "field `0` is never read" `dead_code` warning on
      `CodeGenError::ArrayNotAssignable`'s own carried source-offset
      field -- the SAME warning shape `ArgCountMismatch`/`UndefinedLabel`
      already carry (both already disclosed as "real but not currently
      read anywhere," present since before this milestone), now a third
      instance of the identical, already-established pattern rather than
      a new kind of warning. Every other line in both sorted warning
      lists is byte-for-byte identical.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m111_fresh_boot.log`/`m111_reused_boot.log`),
      no stray `qemu-system-x86_64.exe` process before or after either
      boot (checked via `tasklist`). `OVERALL_M111=PASS` on both boots --
      CASE 98 correctly rejects `arr = 5;`, CASE 99 correctly rejects
      `arr += 5;` through the compound-assignment desugaring path,
      identically on both boots. Every marker this kernel has ever
      produced was independently re-confirmed `PASS` on both boots via a
      real script (not sampled or eyeballed) that reconstructs the
      actual WRITE-syscall payload stream from each raw serial log
      (concatenating every `milestone 31: syscall WRITE ... raw bytes
      >>> ... <<< end of write` block in file order) to recover every
      `OVERALL`/`OVERALL_M*=` marker cc.elf's own self-test ever prints,
      plus a separate raw-text scan for the kernel's own older
      `milestone NN: self-test -- OVERALL: PASS` style that prints
      directly to the serial port rather than through a ring-3 WRITE
      syscall -- 48 total distinct markers across both styles (up from
      Milestone 110's 47, the one new name being `OVERALL_M111` itself),
      all `PASS`, fresh and reused in exact agreement, both boots'
      reconstructed marker sets identical to each other. No regressions:
      the reused boot's own `fs self-test: permissions`/`symlinks`
      FAIL-line set was diffed directly (`diff`, not eyeballed) against
      Milestone 110's own reused boot's FAIL-line set -- byte-for-byte
      identical, 54 lines, the same pre-existing, already-disclosed
      Milestone 62-era fixture-cleanup artifact every milestone since 70
      has named; the fresh boot's own fs self-test passes both suites
      cleanly (zero FAIL lines). `run_preemption_demo()` ran to
      completion on BOTH boots (fresh: `task0=2651 task1=2576
      task2=2808`; reused: `task0=2564 task1=2537 task2=2824` -- real
      inter-boot timing jitter, consistent with every prior milestone's
      own two-boot pairs) -- zero `run_preemption_demo() SKIPPED` lines
      on either boot. Milestone 104's own heap-pressure evidence-gate
      state came out BYTE-IDENTICAL to Milestone 108/109/110's own
      recorded baseline on both boots: `samples_taken=68
      gate_trip_episodes=0 naive_instantaneous_hits=0
      admissions_denied_so_far=0` -- zero drift, consistent with this
      milestone being pure compiler-frontend work with no new kernel
      allocation path. Zero `panicked at`/`kernel panicked` and zero
      `DOUBLE FAULT`/`TRIPLE FAULT` in either log (checked directly,
      `grep -c`, zero matches in both).

      **Still genuinely open**: no `char *` parameters/returns; no
      `int **`/multi-level pointers anywhere; no pointer arithmetic; no
      `int name[]` bracket-array parameter sugar (pointer spelling only);
      no `*p += 1;`/`arr[i] += 1;`-style compound assignment through a
      dereference or index (element-level compound assignment remains a
      real, disclosed gap, unaffected by this milestone -- see this
      entry's own "Scope" paragraph above); no type system of any kind,
      so no shape-aware rejection of a mismatched argument/return
      (Milestone 108/109/110's own disclosed limitation, unrelated to
      this milestone's own new `is_array`-based rejection, which is a
      narrower, single-purpose check, not a general type system); no
      NULL-pointer concept; no bounds/validity checking on any
      dereference or index. `MAX_PARAMS` (8), no `sizeof`, `char` as the
      only non-`int` scalar type, the compiler self-test's own
      per-compile leak, `heap_guard::admit()` covering only its original
      two call sites, and `evidence_gate.rs` still having exactly one
      real subscriber (Milestone 106) all remain exactly as Milestone 110
      left them -- unrelated to this milestone.

- [x] **Milestone 112**: real `char *` pointers -- declaration and a real
      byte-narrowing dereference-READ (`tools/cc_src/main.rs`) -- the
      OTHER candidate Milestone 110's own closing disclosure named
      alongside the `arr = expr;` asymmetry Milestone 111 just resolved.
      A deliberately narrow first slice, the same discipline every prior
      array/pointer milestone (108 through 111) already used.

      **Grammar delta**: none beyond one new lookahead branch -- `*`/`&`
      already exist in every grammar position they need. `char`
      immediately followed by `*` is now ALSO a pointer declarator
      (`char *p ...;`), `parse_stmt()`'s `TOK_CHAR` arm gaining the exact
      one-token-lookahead check `TOK_INT`'s arm already had for `int *`.
      `parse_pointer_decl()` gains a new `is_char_pointee: bool`
      parameter, threaded into the produced `STMT_DECL`'s own `expr`
      field -- otherwise always dead for a pointer declarator (a
      pointer's optional initializer is handled by a separately chained
      `STMT_ASSIGN`, unchanged) -- so `collect_vars_rec()` can later
      recover it into a new `VarSlot` field, `is_char_ptr`. `&c` already
      worked unconditionally for any variable, `int` or `char` -- zero
      change needed there.

      **A genuinely separate property from `is_char`**: a `char *p;`'s
      own slot still has `is_char == 0` (reading `p` ITSELF, e.g. `q =
      p;`, must load the full 8-byte address unnarrowed -- a pointer's
      own value is an address, never narrowed) and `is_char_ptr == 1`
      (reading `*p` should narrow). Threaded out through `find_var()`'s
      now-four-element return tuple to a genuinely NEW, seventh call
      site -- `EXPR_UNARY`'s own `OP_DEREF` arm, which never called
      `find_var()` at all before this milestone (`int *` dereference
      never needed to distinguish anything about the pointee); all six
      pre-existing call sites just ignore the new field.

      **The one genuinely new x86_64 encoding**, unlike Milestone 109/110
      (which needed zero new encodings, every capability built purely by
      branching among sequences Milestone 108 already hand-verified):
      `emit_movsx_rax_mem_rax_byte()` (`MOVSX RAX, BYTE PTR [RAX]`,
      REX.W `0F BE` /r, ModRM `0x00`). Reasoning for why it's needed at
      all: a `char` variable's own stack slot is STILL a full 8 bytes
      (unchanged since Milestone 102 -- `char` never got a packed 1-byte
      slot, only a narrower READ), so the pre-existing `int *`-style
      `emit_mov_rax_mem_rax()` would read the pointee's RAW, un-narrowed
      8-byte value -- wrong for `char *`, which must read back the low
      byte, sign-extended, exactly like an ordinary `char` scalar's own
      read already does. The new encoding is a real, deliberate
      COMBINATION of two halves each already independently hand-verified
      against the Intel SDM by an earlier milestone:
      `emit_movsx_rax_rbp_off_byte()`'s own opcode bytes (Milestone 102)
      and `emit_mov_rax_mem_rax()`'s own `[RAX]` register-indirect ModRM
      byte (Milestone 108) -- neither half is new, but combining them
      (byte-sized, sign-extending source, addressed through a register-
      indirect operand instead of an rbp-relative one) is a real, checked
      claim, not an assumption by analogy.

      **The write side is deliberately UNCHANGED**: `*p = expr;`
      (`STMT_ASSIGN_DEREF`) always does the ordinary full 8-byte store,
      for both `int *` and `char *` alike, exactly matching the
      convention an ordinary `char` SCALAR's own store already uses
      (`char c = expr;` also always writes the full 8 bytes, narrowing
      only ever happening on read, since Milestone 102). Since the
      pointee, in every case this subset can reach, is always another
      `VarSlot`'s own full 8-byte stack slot (no packed multi-char buffer
      exists anywhere in this language -- no `char[]`, no `malloc`-backed
      byte buffer), writing all 8 bytes through the pointer lands on the
      identical bytes an ordinary direct write to that same variable
      already would -- a real, deliberate design choice, not an
      oversight, and it means this milestone needed zero new store-side
      codegen at all.

      **A real, deliberate, disclosed scope restriction**: `*p`'s
      char-narrowing on the read side is only recognized when `p` is
      syntactically a plain `EXPR_IDENT` (checked directly on `e.left`'s
      own AST node kind before the new `find_var()` lookup runs) -- any
      other shape (a nested dereference, a call result) falls through to
      the ordinary full-width load, the same "resolved by operand SHAPE,
      not attempted for every expression" precedent Milestone 109's own
      `&` restriction already established for this exact operator
      family. This costs nothing reachable in THIS milestone's own
      scope: `char *` cannot cross a function-call boundary (no `char *`
      parameters or return values, the same scope cut Milestone 109
      itself used for `int *` before Milestone 110 later closed it),
      there is no array-to-pointer decay to a `char *` (no `char[]`
      exists to decay from), and there is no pointer arithmetic -- so a
      local `char *` declaration, optionally initialized via `&c`, is
      the ONLY way this milestone's own grammar can ever construct a
      `char *` value, and every such value is always named by a plain
      IDENT at its one real use site. Not independently self-tested for
      that reason -- there is no reachable program in this milestone's
      own grammar that would exercise the fallback branch, and a
      fabricated one would test nothing real; honestly disclosed rather
      than papered over with a synthetic case.

      **New self-test cases**, all three run in-process
      (`CodegenMode::Callable`), the same tier Milestone 107 through 111
      all already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle added: CASE 100 (`char c; c =
      200; char *p = &c; return *p == -56;` -- the real, distinguishing
      proof that `*p` genuinely narrows to a signed byte: a buggy
      implementation reusing `int *`'s own full-width load would read
      back the raw 200, making `*p == -56` false; expected `1`); CASE
      101 (`char c; c = 5; char *p = &c; *p = 200; return c == -56;` --
      the real, distinguishing proof that a write through `char *` is
      genuine aliasing into the pointee's own real stack slot, combined
      with proof the write side deliberately does not narrow, reading
      `c` back through a DIFFERENT access path than the one that wrote
      it -- the same distinguishing shape Milestone 109's own CASE 92
      established for `int *` aliasing, now proven for `char *`;
      expected `1`); CASE 102 (`int x; x = 1000; int *ip = &x; char c; c
      = 200; char *cp = &c; return (*ip == 1000) + (*cp == -56) * 10;`
      -- the real, distinguishing proof `int *` and `char *`
      dereference-reads coexist in the SAME program with zero
      cross-contamination: a bug that narrowed every dereference, or
      neither, would still pass CASE 100/101 in isolation but fails
      here, since the combined result is only correct if BOTH are
      independently right at once; expected `11`).

      **No SNN/evidence-gate work this milestone, and correctly so**:
      this is a pure compiler-frontend milestone -- new grammar
      positions, one new `VarSlot` field, one new x86_64 encoding, no
      kernel-side subsystem decision anywhere in it for an
      evidence-gated hysteresis gate to meaningfully subscribe to. The
      same "pure compiler-frontend features legitimately don't need
      this" carve-out Milestone 107 through 111 all already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 95816 bytes, up from Milestone 111's
      94032; the real `PT_LOAD` `p_memsz` via `readelf -l` is 79776
      bytes, still 20 pages (the same page count as Milestone 111, just
      more bytes within it) -- comfortably under the 64-page per-segment
      cap and 128-page total cap, no cap change needed. The rebuilt
      `cc.elf` was independently re-derived from the current `main.rs`
      via a from-scratch invocation of the exact same recipe and diffed
      byte-for-byte identical against the committed binary (`cmp`, not
      eyeballed). **Zero new compiler warnings** -- verified the same way
      Milestone 109/110 verified their own zero-new-warning result:
      reproducing the standalone `rustc` invocation and diffing the
      sorted warning list against Milestone 111's own recorded baseline
      -- byte-for-byte identical 16-warning set (the same set Milestone
      111 itself introduced its own new warning into; this milestone
      adds none further).

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m112_fresh_boot.log`/`m112_reused_boot.log`),
      no stray `qemu-system-x86_64.exe` process before or after either
      boot (checked via `tasklist`). `OVERALL_M112=PASS` on both boots --
      CASE 100 returns `1`, CASE 101 returns `1`, CASE 102 returns `11`,
      identically on both boots. Every marker this kernel has ever
      produced was independently re-confirmed `PASS` on both boots via a
      real script (not sampled or eyeballed) that reconstructs the
      actual WRITE-syscall payload stream from each raw serial log to
      recover every `OVERALL`/`OVERALL_M*=` marker, plus a separate
      raw-text scan for the kernel's own older `milestone NN: self-test
      -- OVERALL: PASS` style -- 49 total distinct markers across both
      styles (up from Milestone 111's 48, the one new name being
      `OVERALL_M112` itself), all `PASS`, fresh and reused in exact
      agreement, both boots' reconstructed marker sets identical to each
      other. No regressions: the reused boot's own `fs self-test:
      permissions`/`symlinks` FAIL-line set was diffed directly (`diff`,
      not eyeballed) against Milestone 111's own reused boot's FAIL-line
      set -- byte-for-byte identical, 54 lines, the same pre-existing,
      already-disclosed Milestone 62-era fixture-cleanup artifact every
      milestone since 70 has named; the fresh boot's own fs self-test
      passes both suites cleanly (zero FAIL lines). `run_preemption_
      demo()` ran to completion on BOTH boots (fresh: `task0=2680
      task1=2487 task2=2750`; reused: `task0=2627 task1=2573
      task2=2833` -- real inter-boot timing jitter, consistent with
      every prior milestone's own two-boot pairs) -- zero
      `run_preemption_demo() SKIPPED` lines on either boot. Milestone
      104's own heap-pressure evidence-gate state came out
      BYTE-IDENTICAL to Milestone 108 through 111's own recorded
      baseline on both boots: `samples_taken=68 gate_trip_episodes=0
      naive_instantaneous_hits=0 admissions_denied_so_far=0` -- zero
      drift, consistent with this milestone being pure compiler-frontend
      work with no new kernel allocation path. Zero `panicked at`/
      `kernel panicked` and zero `DOUBLE FAULT`/`TRIPLE FAULT` in either
      log (checked directly, `grep -c`, zero matches in both).

      **Still genuinely open**: no `char **`/multi-level pointers of any
      kind; no `char *` function parameters or return values (the exact
      scope cut named above); no array-to-pointer decay to a `char *`
      (no `char[]` exists anywhere in this subset); no pointer
      arithmetic (the same standing gap `int *` has carried since
      Milestone 109); `p[idx]` byte-scaled indexing through a `char *`
      is a real, disclosed, NOT silently-corrected gap this milestone
      leaves open -- `EXPR_INDEX`/`STMT_ASSIGN_INDEX` are UNTOUCHED, so
      `char *cp; return cp[0];` would compile and run but still perform
      the pre-existing `int`-style 8-byte-scaled index arithmetic and an
      un-narrowed 8-byte load, a real asymmetry with `*p`'s own correct
      narrowing, honestly named rather than papered over; no `*p += 1;`-
      style compound assignment through a `char *` (the same standing
      gap `int *` already has); no NULL-pointer concept and no
      bounds/validity checking on any dereference. Everything Milestone
      108/109/110/111 already disclosed that this milestone didn't touch
      remains open: no `int **`, no `int name[]` bracket-array parameter
      sugar, no `*p += 1;`/`arr[i] += 1;` compound assignment through an
      `int *` dereference or index, no type system of any kind.
      `MAX_PARAMS` (8), no `sizeof`, `char` as the only non-`int` scalar
      TYPE (a `char *` is a pointer flavor, not a second scalar type --
      this milestone doesn't change that count), the compiler self-test's
      own per-compile leak, `heap_guard::admit()` covering only its
      original two call sites, and `evidence_gate.rs` still having
      exactly one real subscriber (Milestone 106) all remain exactly as
      Milestone 111 left them -- unrelated to this milestone.

- [x] **Milestone 113**: `char *` crossing a real function-call boundary --
      parameters AND return values -- for the first time
      (`tools/cc_src/main.rs`) -- the exact candidate Milestone 112's own
      closing disclosure named ("no `char *` function parameters or
      return values ... a future milestone extending `char *` across a
      call boundary ... would need to thread a real declared-parameter-
      type bit through `ParamInfo`, which does not exist today"), the
      same step Milestone 110 already took for `int *` after Milestone
      109.

      **Grammar delta**: both `*`s that were previously legal only after
      `int` are now ALSO legal after `char`: `param := ("int" "*"? |
      "char" "*"?) IDENT`, and a function's own leading return-type
      keyword accepts the identical optional `*`. `parse_params()`'s
      `char` arm and `parse_function()`'s `char` arm each gained the exact
      one-token-lookahead check their `int` siblings already had since
      Milestone 110.

      **The one genuinely new field -- `ParamInfo::is_char_ptr`**: a new
      `u8` on a struct that, until this milestone, had exactly one field
      (`is_char`). A `char *p` parameter must end up with `is_char == 0`
      (its OWN value is an address, never narrowed on read) and
      `is_char_ptr == 1` (a dereference `*p` through it should narrow) --
      the identical split `VarSlot` has carried for a LOCAL `char *` since
      Milestone 112, now reaching a parameter's own `ParamInfo` for the
      first time. `collect_vars_for_function()` threads `p.is_char_ptr`
      straight into the parameter's own `VarSlot` -- previously hardcoded
      0, per Milestone 112's own explicit "always 0 for a parameter"
      comment, now genuinely variable -- the ONLY codegen change this
      milestone makes anywhere. `int *` needs no equivalent field
      (unchanged from Milestone 110): an `int *` parameter's `VarSlot` is
      already `is_char: 0, is_char_ptr: 0`, the correct shape for an
      ordinary unnarrowed 8-byte pointer with no dedicated flag at all.

      **The return-type half needs no new field either**: `is_char_return:
      bool` already asked exactly the right question ("narrow this
      function's return value on the way out?") -- `char *f(...)` sets it
      `false` (a pointer return must never be narrowed, whether to `int`
      or `char`; narrowing would sign-extend the returned ADDRESS's own
      low byte into a corrupted, essentially random address, not merely
      give back a wrong value), the identical treatment `int *f(...)`
      already got from Milestone 110. `STMT_RETURN`'s own codegen,
      unchanged, still just checks the one boolean.

      **Zero new x86_64 encodings**: pure grammar-plus-one-field-threading,
      the same "real capability, zero new machine code" shape Milestone
      110 already achieved for `int *` crossing a call boundary -- every
      encoding this milestone's own new cases exercise (register/stack
      argument passing, RAX-based return, the narrowing
      `emit_movsx_rax_mem_rax_byte()` load) was already hand-verified by
      an earlier milestone (107, 110, 112 respectively). `EXPR_UNARY`'s
      own `OP_DEREF` arm (Milestone 112) already reads `is_char_ptr`
      generically through `find_var()`, with zero awareness of whether a
      given `VarSlot` came from a parameter or a local declaration, so a
      parameter's own `*p` narrows correctly with literally no change at
      that call site.

      **A real, disclosed, newly-REACHABLE instance of an old
      restriction**: Milestone 112's own `*p` narrowing is recognized only
      when the dereferenced operand is syntactically a plain `EXPR_IDENT`
      -- in Milestone 112's own scope this cost nothing reachable (no
      `char *` return value existed, so a call result could never even be
      the operand of `*`). This milestone makes `char *f(...)` a real
      return type, so `*f(&c)` -- dereferencing a call result DIRECTLY,
      with no intervening local-variable assignment -- is now a real,
      reachable program for the first time, and it falls through to the
      ordinary un-narrowed load exactly like any other non-`EXPR_IDENT`
      dereference operand already does. Not resolved here (narrowing a
      call result directly would need `gen_expr()`'s own `EXPR_CALL` arm
      to carry pointee-type information forward, real new plumbing out of
      scope) -- and, per this project's own "real, measured results only"
      discipline, proven with an actual running case rather than merely
      asserted in prose (CASE 106 below).

      **New self-test cases**, all four run in-process
      (`CodegenMode::Callable`), the same tier Milestone 107 through 112
      all already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle is added by this milestone: CASE
      103 (`read_narrow(p)` -- the real, distinguishing proof that a
      `char *` PARAMETER's own dereference-read still genuinely narrows,
      not merely that a local one does, the exact bug an unthreaded
      `ParamInfo::is_char_ptr` would produce; expected `1`); CASE 104
      (`write_it(p)` -- the real, distinguishing proof that a WRITE
      through a `char *` PARAMETER is genuine aliasing into the CALLER's
      own real stack slot, the same "read back through a different access
      path than the one that wrote it" shape Milestone 112's own CASE 101
      and Milestone 110's own CASE 96 already established, now proven for
      a `char *` parameter specifically; expected `1`); CASE 105
      (`identity_char(p)` returning `char *`, its result assigned to a
      local `char *q` and dereferenced through that plain IDENT -- the
      real, distinguishing proof that a `char *` return crosses a call
      boundary un-corrupted, a buggy `is_char_return` computation would
      corrupt the returned ADDRESS itself, not just a value, "caught, not
      silently wrong" the same way Milestone 110's own CASE 97 doc comment
      already reasoned for `int *`; expected `1`); CASE 106 (a real,
      deliberate, DISCLOSED negative case, not a regression --
      `*identity_char2(&c)` dereferences a call result DIRECTLY, proving
      the newly-reachable old restriction named above with an actual
      measured number rather than an assumed claim: expected `0`, the
      real un-narrowed fallback value, not the morally "correct" narrowed
      answer -- a real regression tripwire on the disclosed gap's exact
      numeric fingerprint, the same "diffed for equality across
      milestones" discipline the fs self-test's own known FAIL-line-count
      baseline already uses for a different pre-existing artifact).

      **No SNN/evidence-gate work this milestone, and correctly so**: this
      is a pure compiler-frontend milestone -- one new `ParamInfo` field,
      two new grammar-position checks, no kernel-side subsystem decision
      anywhere in it for an evidence-gated hysteresis gate to meaningfully
      subscribe to. The same "pure compiler-frontend features legitimately
      don't need this" carve-out Milestone 107 through 112 all already
      established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 97856 bytes, up from Milestone 112's
      95816; the real `PT_LOAD` `p_memsz` via `readelf -l` is 83192 bytes,
      21 pages, up from Milestone 112's own 20 -- comfortably under the
      64-page per-segment cap and 128-page total cap, no cap change
      needed. The rebuilt `cc.elf` was independently re-derived from the
      current `main.rs` via a from-scratch invocation of the exact same
      recipe and diffed byte-for-byte identical against the committed
      binary (`cmp`, not eyeballed). **Zero new compiler warnings** --
      verified the same way Milestone 109 through 112 verified their own:
      reproducing the standalone `rustc` invocation against a `git stash`
      of this milestone's own changes (a real, clean Milestone 112
      checkout) and diffing the sorted warning list against this
      milestone's own build -- byte-for-byte identical 16-warning set.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m113_fresh_boot.log`/`m113_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). `OVERALL_M113=PASS` on both boots -- CASE
      103 returns `1`, CASE 104 returns `1`, CASE 105 returns `1`, CASE
      106 returns `0`, identically on both boots. Every marker this kernel
      has ever produced was independently re-confirmed `PASS` on both
      boots via a real script (not sampled or eyeballed) that reconstructs
      the actual WRITE-syscall payload stream from each raw serial log
      (concatenating every `milestone 31: syscall WRITE ... raw bytes >>>
      ... <<< end of write` block in file order, matched against the
      real `\r\n` line endings this kernel's serial writer actually uses)
      to recover every `OVERALL`/`OVERALL_M*=` marker cc.elf's own
      self-test ever prints, plus a separate raw-text scan for the
      kernel's own older `milestone NN: self-test -- OVERALL: PASS` style
      that prints directly to the serial port rather than through a
      ring-3 WRITE syscall -- 50 total distinct markers across both styles
      (up from Milestone 112's 49, the one new name being `OVERALL_M113`
      itself), all `PASS`, fresh and reused in exact agreement, both
      boots' reconstructed marker sets identical to each other. No
      regressions: the reused boot's own `fs self-test: permissions`/
      `symlinks` FAIL-line set was diffed directly (`diff`, not eyeballed)
      against Milestone 112's own reused boot's FAIL-line set --
      byte-for-byte identical, 54 lines, the same pre-existing,
      already-disclosed Milestone 62-era fixture-cleanup artifact every
      milestone since 70 has named; the fresh boot's own fs self-test
      passes both suites cleanly (zero FAIL lines). `run_preemption_
      demo()` ran to completion on BOTH boots (fresh: `task0=2464
      task1=2479 task2=2736`; reused: `task0=2510 task1=2411
      task2=2790` -- real inter-boot timing jitter, consistent with every
      prior milestone's own two-boot pairs) -- zero `run_preemption_
      demo() SKIPPED` lines on either boot. Milestone 104's own
      heap-pressure evidence-gate state came out BYTE-IDENTICAL to
      Milestone 108 through 112's own recorded baseline on both boots:
      `samples_taken=68 gate_trip_episodes=0 naive_instantaneous_hits=0
      admissions_denied_so_far=0` -- zero drift, consistent with this
      milestone being pure compiler-frontend work with no new kernel
      allocation path. Zero `panicked at`/`kernel panicked` and zero
      `DOUBLE FAULT`/`TRIPLE FAULT` in either log (checked directly,
      `grep -c`, zero matches in both).

      **Still genuinely open**: no `char **`/multi-level pointers of any
      kind; no array-to-pointer decay to a `char *` (no `char[]` exists
      anywhere in this subset, so unlike `int *`/array parameters there is
      no equivalent of Milestone 110's own `sum3(arr)`-style decay case
      here); no pointer arithmetic (the same standing gap `int *` has
      carried since Milestone 109); no `int name[]`/`char name[]`
      bracket-array parameter sugar; `p[idx]` byte-scaled indexing through
      a `char *` remains the same real, disclosed, NOT silently-corrected
      gap Milestone 112 left open, now ALSO reachable through a parameter
      (`EXPR_INDEX`/`STMT_ASSIGN_INDEX` are just as untouched by THIS
      milestone as they were by 112); the newly-reachable `*f()`
      direct-call-result narrowing gap named above and proven by CASE 106
      (an intervening local-variable assignment, as CASE 105 uses, is the
      real, disclosed workaround, not a fix); no `*p += 1;`-style compound
      assignment through any pointer; no type system of any kind, so a
      mismatched `char *`/`int *` argument or return is accepted without
      complaint exactly as every other shape mismatch already silently is;
      no NULL-pointer concept and no bounds/validity checking on any
      dereference. `MAX_PARAMS` (8), no `sizeof`, `char` as the only
      non-`int` scalar TYPE, the compiler self-test's own per-compile
      leak, `heap_guard::admit()` covering only its original two call
      sites, and `evidence_gate.rs` still having exactly one real
      subscriber (Milestone 106) all remain exactly as Milestone 112 left
      them -- unrelated to this milestone.

- [x] **Milestone 114**: `p[idx]`/`p[idx] = expr;` through a `char *` now
      genuinely byte-scale (both arms) AND narrow (the read arm)
      (`tools/cc_src/main.rs`) -- closing the exact asymmetry Milestone
      112's own closing disclosure named ("`p[idx]` byte-scaled indexing
      through a `char *` is a real, disclosed, NOT silently-corrected
      gap") and Milestone 113's own closing disclosure re-named as newly
      reachable through a parameter too.

      **No grammar change at all**: `p[idx]` already parsed identically
      regardless of `p`'s own declared pointee type. The real fix lives
      entirely in what `EXPR_INDEX`/`STMT_ASSIGN_INDEX`'s existing
      address-scaling codegen does once `find_var()`'s own `is_char_ptr`
      field -- already threaded to both call sites since Milestone 112,
      simply never consulted by either arm before now -- comes back true:
      `EXPR_INDEX` skips `emit_shl_rcx_imm8(3)` (leaving the raw index
      UNSCALED in RCX -- real C's own "one `char` is one byte" rule,
      genuinely different from every `int`/`int *`/array index's own
      "one element is 8 bytes" convention) and loads through the
      resulting address with `emit_movsx_rax_mem_rax_byte()` instead of
      the ordinary full-width `emit_mov_rax_mem_rax()` -- the exact same
      narrowing encoding Milestone 112's own `*p` read already
      introduced, reused here completely UNCHANGED. `STMT_ASSIGN_INDEX`
      gets only the unscaled-index half (also skips
      `emit_shl_rcx_imm8(3)` when `is_char_ptr != 0`) -- the STORE itself
      stays deliberately full-width regardless, exactly matching
      `*p = expr;`'s own Milestone 112 write-side convention (narrowing
      only ever happens on READ). **Zero new x86_64 encodings**: this
      milestone is pure branching among already hand-verified sequences,
      the same shape Milestone 110 already achieved for `int *` crossing
      a call boundary.

      **The real, honest identity this closes**: at `idx == 0`, `p[0]`
      now computes the byte-identical address and narrowed load `*p`
      already does -- exactly the guarantee real C makes
      (`p[0] == *(p+0) == *p`), and the ONLY case this milestone
      independently hand-verifies (CASE 107/108 below). For a nonzero
      index, the byte-stride fix is structurally real and correct (this
      is what "`p[idx] == *(p+idx)`, byte-addressed" genuinely means),
      but this milestone does NOT fabricate a hand-computed nonzero-index
      test: no packed multi-char buffer exists anywhere in this language
      (no `char[]`, no `malloc`-backed byte buffer, the same standing gap
      Milestone 112 already named), so a real `char *` in this subset's
      own grammar only ever points at a single scalar `char` variable's
      own 8-byte `VarSlot` (via `&c`) -- `p[1]` steps one byte into
      whatever real memory happens to sit adjacent to that slot, the same
      real, disclosed, undefined-but-not-new "no bounds checking, wild
      address reads whatever is there" territory an out-of-range array
      index already occupies (Milestone 108's own disclosure), honestly
      named rather than papered over with a case that tests nothing real.

      **New self-test cases**, all three run in-process
      (`CodegenMode::Callable`), the same tier Milestone 107 through 113
      all already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle is added by this milestone: CASE
      107 (`p[0] == -56` through a `char *` -- the real, distinguishing
      proof that `p[0]` now narrows exactly like `*p` already does; a
      pre-fix build computes the SAME address at `idx == 0` but loads the
      full un-narrowed 8 bytes, so this is a real regression check against
      this milestone's own Milestone 113 baseline, not merely a fresh
      assertion; expected `1`); CASE 108 (`p[0] = 200;` through a
      `char *`, read back via `c` directly -- the real, distinguishing
      proof that a write through `p[idx]` is genuine aliasing into the
      pointee's own real stack slot, combined with proof the write side
      stays deliberately un-narrowed, the same shape Milestone 112's own
      CASE 101 already established for `*p = expr;`, now proven for the
      genuinely separate `STMT_ASSIGN_INDEX` codegen arm; expected `1`);
      CASE 109 (a real declared array, a decayed `int *`, and a `char *`
      all indexed in the SAME program -- the real, distinguishing proof
      that all three of `EXPR_INDEX`'s address-scaling shapes coexist
      with zero cross-contamination: `ip[1] == 222` is only true if
      `int *` indexing stayed 8-byte-scaled and un-narrowed while `cp[0]`
      independently narrowed; expected `111`).

      **No SNN/evidence-gate work this milestone, and correctly so**: this
      is a pure compiler-frontend milestone -- branching among already
      hand-verified encodings in two existing codegen arms, no kernel-side
      subsystem decision anywhere in it for an evidence-gated hysteresis
      gate to meaningfully subscribe to. The same carve-out Milestone 107
      through 113 all already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 99344 bytes, up from Milestone 113's
      97856; the real `PT_LOAD` `p_memsz` via `readelf -l` is 84448 bytes,
      still 21 pages (the same page count as Milestone 113, just more
      bytes within it) -- comfortably under the 64-page per-segment cap
      and 128-page total cap, no cap change needed. The rebuilt `cc.elf`
      was independently re-derived from the current `main.rs` via a
      from-scratch invocation of the exact same recipe and diffed
      byte-for-byte identical against the committed binary (`cmp`, not
      eyeballed). **Zero new compiler warnings** -- verified the same way
      Milestone 109 through 113 verified their own: reproducing the
      standalone `rustc` invocation against a `git stash` of this
      milestone's own changes (a real, clean Milestone 113 checkout) and
      diffing the sorted warning list against this milestone's own build
      -- byte-for-byte identical 16-warning set.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m114_fresh_boot.log`/`m114_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). `OVERALL_M114=PASS` on both boots -- CASE
      107 returns `1`, CASE 108 returns `1`, CASE 109 returns `111`,
      identically on both boots. Every marker this kernel has ever
      produced was independently re-confirmed `PASS` on both boots via the
      same real reconstruction script Milestone 113 introduced (matching
      this kernel's own real `\r\n` serial line endings) -- 51 total
      distinct markers across both styles (up from Milestone 113's 50,
      the one new name being `OVERALL_M114` itself), all `PASS`, fresh and
      reused in exact agreement, both boots' reconstructed marker sets
      identical to each other. No regressions: the reused boot's own
      `fs self-test: permissions`/`symlinks` FAIL-line set was diffed
      directly (`diff`, not eyeballed) against Milestone 113's own reused
      boot's FAIL-line set -- byte-for-byte identical, 54 lines, the same
      pre-existing, already-disclosed Milestone 62-era fixture-cleanup
      artifact every milestone since 70 has named; the fresh boot's own fs
      self-test passes both suites cleanly (zero FAIL lines).
      `run_preemption_demo()` ran to completion on BOTH boots (fresh:
      `task0=2575 task1=2507 task2=2768`; reused: `task0=2582 task1=2588
      task2=2890` -- real inter-boot timing jitter, consistent with every
      prior milestone's own two-boot pairs) -- zero `run_preemption_
      demo() SKIPPED` lines on either boot. Milestone 104's own
      heap-pressure evidence-gate state came out BYTE-IDENTICAL to
      Milestone 108 through 113's own recorded baseline on both boots:
      `samples_taken=68 gate_trip_episodes=0 naive_instantaneous_hits=0
      admissions_denied_so_far=0` -- zero drift, consistent with this
      milestone being pure compiler-frontend work with no new kernel
      allocation path. Zero `panicked at`/`kernel panicked` and zero
      `DOUBLE FAULT`/`TRIPLE FAULT` in either log (checked directly,
      `grep -c`, zero matches in both).

      **Still genuinely open**: no `char **`/multi-level pointers; no
      array-to-pointer decay to a `char *`; no pointer arithmetic (`p + 1`/
      `p++` remain entirely unimplemented, so `p[idx]`'s own internal
      address arithmetic is the ONLY byte-scaled `char *` addressing this
      codegen performs anywhere); no `int name[]`/`char name[]`
      bracket-array parameter sugar; the newly-reachable `*f()`
      direct-call-result narrowing gap Milestone 113 disclosed and proved
      (CASE 106 there) is unrelated to and untouched by this milestone; no
      `*p += 1;`/`p[idx] += 1;`-style compound assignment through any
      pointer; no type system of any kind, so a mismatched `char *`/
      `int *` argument or return is accepted without complaint exactly as
      every other shape mismatch already silently is; no NULL-pointer
      concept and no bounds/validity checking on any dereference or index.
      `MAX_PARAMS` (8), no `sizeof`, `char` as the only non-`int` scalar
      TYPE, the compiler self-test's own per-compile leak,
      `heap_guard::admit()` covering only its original two call sites, and
      `evidence_gate.rs` still having exactly one real subscriber
      (Milestone 106) all remain exactly as Milestone 113 left them --
      unrelated to this milestone.

- [x] **Milestone 115**: real pointer arithmetic -- `p + n`, `n + p`, and
      `p - n` (`EXPR_BINARY`'s own `+`/`-` codegen, `tools/cc_src/main.rs`)
      now genuinely scale the integer operand by the pointer's own pointee
      size instead of doing a plain, meaningless raw-address-plus-raw-
      integer add -- closing the exact gap Milestone 109's own original
      closing disclosure first named ("no pointer arithmetic") and every
      pointer/array milestone since (110 through 114) re-confirmed still
      open, and the first of this milestone's own two listed candidates
      actually taken.

      **Zero grammar change**: `p + 1`/`1 + p`/`p - 1` already parsed as an
      ordinary `EXPR_BINARY` with no pointer awareness at all -- the entire
      real fix lives in what that arm's existing codegen does BEFORE the
      pre-existing add/sub instruction runs.

      **The one genuinely new piece of plumbing, and the real surprise
      found while scoping it**: unlike `char *` (unambiguously
      identifiable since Milestone 112 via `STMT_DECL.expr`), an `int *`
      variable had NO signal distinguishing it from a plain `int`
      ANYWHERE in this AST before this milestone. Milestone 109's own
      deliberate "a pointer's runtime value is just another 8-byte stack
      slot, no new `VarSlot` field needed" design choice was correct for
      every pointer/array milestone since -- reading a pointer's own value,
      or dereferencing it, works identically whether the compiler can tell
      it apart from a plain `int` or not -- but it meant `int *p;` and
      `int p;` produced a BYTE-IDENTICAL `StmtNode` (`then_body: 0, expr:
      0` either way). Real, TYPE-AWARE arithmetic scaling is the first
      feature that actually needs to tell them apart. The fix: a new
      sentinel (`then_body: 2`, written only by `parse_pointer_decl()`'s
      own pre-existing `int *` branch -- its `char *` branch is
      deliberately left alone, since `expr == 1` already identifies it
      unambiguously) plus a new `VarSlot` field, `is_int_ptr`, threaded
      through `find_var()`'s now-five-element return tuple (`(stack_off,
      is_char, is_array, is_char_ptr, is_int_ptr)`, all seven pre-existing
      call sites updated to ignore the new field) to a new classifier
      helper, `ptr_operand_scale()`: `Some(1)` for a `char *` IDENT,
      `Some(8)` for an `int *` IDENT or a decayed array base (`is_array`),
      `None` otherwise (a plain scalar, an undeclared name, or any
      non-`EXPR_IDENT` shape -- the same "resolved by operand SHAPE, not
      attempted for every expression" restriction `OP_ADDR`/`OP_DEREF`
      already established for `&`/`*`, Milestone 109/112, applied here to
      `+`/`-`).

      **One genuinely new x86_64 encoding**: `emit_shl_rax_imm8()`
      (`SHL RAX, imm8`, REX.W `C1` `/4` `E0`), the RAX-targeted sibling of
      the pre-existing `emit_shl_rcx_imm8()` -- needed because `n + p` (the
      pointer on the RIGHT, real C's own commutative spelling for `p + n`)
      leaves the integer operand in RAX rather than RCX after the
      existing left-push/right-pop preamble (unchanged, still evaluates
      left into RAX, pushes, evaluates right into RAX, moves it to RCX,
      pops left back into RAX -- real evaluation order is completely
      untouched by this milestone). Not a genuinely new encoding FAMILY:
      both halves were independently hand-verified by an earlier
      milestone already -- the `C1` immediate-shift opcode by
      `emit_shl_rcx_imm8()` itself (Milestone 108), the `0xE0` ModRM byte
      (RAX, SHL's own opcode-extension digit) by `emit_shl_rax_cl()`
      (Milestone 79) -- the same "combine two independently-verified
      halves" real new instruction Milestone 112's own
      `emit_movsx_rax_mem_rax_byte()` doc comment already described. For
      `p + n` (pointer on the left, the common case), scaling instead
      reuses `emit_shl_rcx_imm8()` completely UNCHANGED, the identical
      encoding `EXPR_INDEX`'s own address computation already uses for
      "scale an index by 8" (Milestone 108/114). When both operands
      classify as pointers (real C defines no meaning for `pointer +
      pointer`) or neither does (an ordinary `int + int`), no scaling is
      applied at all -- the plain, unscaled `add`/`sub` runs exactly as it
      always has, a real, disclosed restriction, not a silently
      fabricated meaning for an operator real C itself leaves undefined.

      **A real, honestly-derived, genuinely surprising consequence, NOT
      papered over**: this codegen's own stack frame grows toward LOWER
      addresses as MORE variables are declared, and its own array-element
      layout ALSO grows toward lower addresses as the index grows
      (Milestone 108's own `off - 8*idx` rule) -- so real, hardware-
      correct, address-INCREASING `p + n` reaches a DIFFERENT real stack
      slot than `p[n]` does for a declared array's own decayed base:
      `arr[n]` is reached by `arr - n`, NOT `arr + n`, in THIS compiler
      specifically. This milestone does not "fix" that disagreement by
      secretly making `+` subtract to restore the identity real C
      guarantees between `p[n]` and `*(p+n)` -- that would make pointer
      arithmetic itself backwards relative to real address semantics, a
      strictly worse bug than the disclosed inconsistency. CASE 112 (below)
      hand-verifies this exact, real, honest behavior rather than
      asserting a "morally correct" identity that does not actually hold
      for this compiler's own array layout.

      **New self-test cases**, all four run in-process
      (`CodegenMode::Callable`), the same tier Milestone 107 through 114
      all already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle is added by this milestone: CASE 110
      (`*(p + 1) == 111` through an `int *` -- the real, distinguishing
      proof `p + n` scales by 8 with the pointer on the LEFT, the common
      spelling, scaling RCX via the pre-existing `emit_shl_rcx_imm8()`; a
      pre-fix build would compute an unaligned address one byte off,
      reading garbage instead of a clean `111`; expected `1`); CASE 111
      (`*(1 + p) == 111`, the same real address, reached via the
      COMMUTATIVE spelling -- the real, distinguishing proof for the
      genuinely separate RAX-scaling code path CASE 110 alone does not
      exercise, catching a bug that scaled only the pointer-on-the-left
      shape; expected `1`); CASE 112 (`(*(arr - 1) == 2) + (*(arr - 2) ==
      3) * 10` -- the real, distinguishing proof a decayed ARRAY base
      (`is_array`, a genuinely separate `ptr_operand_scale()` branch from
      CASE 110/111's `int *` variable) also scales by 8, through `-`
      specifically, reaching `arr[1]`/`arr[2]` via this compiler's own
      real, disclosed downward-growing array-address convention; expected
      `11`); CASE 113 (`*(cp + 8) == 200` through a `char *` -- the real,
      decisive regression tripwire that `char *` arithmetic scales by 1,
      NOT 8 (a wrongly-8-scaled build would compute `rbp+48`, real memory
      belonging to neither operand variable at all, not merely a
      differently-wrong number reachable by coincidence), combined with a
      second real, disclosed observation this same case proves with an
      actual measured number rather than mere prose: `*(cp + 8)` does NOT
      narrow to a signed byte (reads back the un-narrowed `200`, not
      `-56`) because `cp + 8` is a real `EXPR_BINARY`, not the plain
      `EXPR_IDENT` shape `OP_DEREF`'s own Milestone 112 narrowing check
      requires -- the same "resolved by operand SHAPE" restriction
      Milestone 113's own CASE 106 already proved for a `char *`-returning
      call result, newly reachable here through pointer arithmetic too,
      and not attempted to be closed by this milestone; expected `1`).

      **No SNN/evidence-gate work this milestone, and correctly so**: this
      is a pure compiler-frontend milestone -- one new `VarSlot` field, one
      new parse-time sentinel, one new classifier helper, one new
      immediate-shift encoding combining two already-verified halves, no
      kernel-side subsystem decision anywhere in it for an evidence-gated
      hysteresis gate to meaningfully subscribe to. The same carve-out
      Milestone 107 through 114 all already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 101800 bytes, up from Milestone 114's
      99344; the real `PT_LOAD` `p_memsz` via `readelf -l` is 86720 bytes,
      22 pages, up from Milestone 114's own 21 -- comfortably under the
      64-page per-segment cap and 128-page total cap, no cap change
      needed. The rebuilt `cc.elf` was independently re-derived from the
      current `main.rs` via a from-scratch invocation of the exact same
      recipe and diffed byte-for-byte identical against the committed
      binary (`cmp`, not eyeballed). **Zero new compiler warnings** --
      verified the same way Milestone 109 through 114 verified their own:
      reproducing the standalone `rustc` invocation against a `git stash`
      of this milestone's own changes (a real, clean Milestone 114
      checkout) and diffing the sorted warning list against this
      milestone's own build -- byte-for-byte identical 16-warning set.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m115_fresh_boot.log`/`m115_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). `OVERALL_M115=PASS` on both boots -- CASE
      110 returns `1`, CASE 111 returns `1`, CASE 112 returns `11`, CASE
      113 returns `1`, identically on both boots. Every marker this kernel
      has ever produced was independently re-confirmed `PASS` on both
      boots via a real reconstruction script (not sampled or eyeballed,
      written fresh this session per this project's own standing note that
      the prior session's equivalent script lives only in ITS OWN
      scratchpad, never committed) that concatenates every real
      `milestone 31: syscall WRITE ... raw bytes >>> ... <<< end of write`
      payload in file order to recover cc.elf's own fragmented
      `OVERALL`/`OVERALL_M*`/`caseNNN_..._returns_*` text (each is written
      one syscall per `w()`/digit call, so a naive `grep` on the raw log
      only ever sees fragments), PLUS a separate scan of the kernel's own
      unfragmented direct-to-serial markers (`ata.rs`/`loader.rs`/
      `process.rs`'s own colon-style `milestone NN: self-test -- OVERALL:
      PASS`, disambiguated by their own milestone-number prefix since that
      literal text has no unique suffix of its own; `heap_guard.rs`/
      `hetero_ensemble.rs`/`hetero_stdp.rs`/`spiking_logic.rs`'s own
      named `OVERALL_M104`/`M77`/`M78`/`M80`/`M81`; `fs.rs`'s own
      "permissions"/"symlinks"-named `OVERALL=` lines) -- 59 total
      distinct OVERALL-family markers and 121 total distinct per-case
      checks (180 markers overall), every one `PASS` on the fresh boot,
      and identical in both marker SET and VALUE on the reused boot with
      exactly one real, already-expected exception: `permissions
      OVERALL`/`symlinks OVERALL` read `FAIL` on the reused boot only --
      the same pre-existing, already-disclosed Milestone 62-era fixture-
      cleanup artifact every milestone since 70 has named, confirmed here
      by diffing the real 54-line `fs self-test: ... FAIL` line set
      directly against Milestone 114's own committed `m114_reused_boot.log`
      -- byte-for-byte identical (`diff`, not eyeballed). The fresh boot's
      own fs self-test passes both suites cleanly (zero FAIL lines beyond
      five unrelated, pre-existing, deliberately-negative-path syscall
      tests this kernel has always logged as "FAILED" outside the fs
      self-test entirely). `run_preemption_demo()` ran to completion on
      BOTH boots (fresh: `task0=2564 task1=2434 task2=2681`; reused:
      `task0=2521 task1=2451 task2=2786` -- real inter-boot timing jitter,
      consistent with every prior milestone's own two-boot pairs) -- zero
      `run_preemption_demo() SKIPPED` lines on either boot. Milestone 104's
      own heap-pressure evidence-gate state came out BYTE-IDENTICAL to
      Milestone 108 through 114's own recorded baseline on both boots:
      `samples_taken=68 gate_trip_episodes=0 naive_instantaneous_hits=0
      admissions_denied_so_far=0` -- zero drift, consistent with this
      milestone being pure compiler-frontend work with no new kernel
      allocation path. Zero `panicked at`/`kernel panicked` and zero
      `DOUBLE FAULT`/`TRIPLE FAULT` in either log (checked directly,
      `grep -c`, zero matches in both).

      **Still genuinely open**: no `char **`/`int **`/multi-level pointers
      of any kind (Milestone 115's own second listed candidate,
      untouched); no `p++`/`p--` -- this lexer has no `++`/`--` tokens AT
      ALL, a real, larger, separate feature, not merely this milestone's
      own narrower scope cut; pointer arithmetic does NOT cross a
      function-call boundary -- `ParamInfo` has no `is_int_ptr` equivalent,
      the same real scope cut `is_char_ptr` itself carried for exactly one
      milestone (112) before 113 closed it, so `p + 1` inside a function
      taking `int *p`/`char *p` silently falls through to the ordinary
      UNSCALED add, exactly as it always has, with zero compile-time
      warning; pointer-pointer subtraction (real C: an element-count
      difference) is not implemented, falling through to a plain,
      meaningless raw-address subtract; `int - pointer` (not legal C to
      begin with) is given no special meaning either; when BOTH operands
      of `+` classify as pointers, no scaling is applied to either (falls
      through to the same plain, meaningless add); pointer-arithmetic
      scaling is recognized only when the pointer-like operand is
      syntactically a plain `EXPR_IDENT`, the same restriction `OP_ADDR`/
      `OP_DEREF`/`p[idx]` already carry; no `*p += 1;`/`p[idx] += 1;`-style
      compound assignment through any pointer (unrelated to and untouched
      by this milestone); `arr +/- n` reaching a DIFFERENT real address
      than `arr[n]` for a nonzero-vs-zero-index-style identity (this
      compiler's own downward-growing layout, disclosed above, not a bug
      this milestone could or should silently correct); no type system of
      any kind, so a mismatched `char *`/`int *` argument or return, or a
      nonsensical `pointer + pointer` expression, is accepted without
      complaint exactly as every other shape mismatch already silently is;
      no NULL-pointer concept and no bounds/validity checking on any
      dereference, index, or arithmetic result. `MAX_PARAMS` (8), no
      `sizeof`, `char` as the only non-`int` scalar TYPE, the compiler
      self-test's own per-compile leak, `heap_guard::admit()` covering
      only its original two call sites, and `evidence_gate.rs` still
      having exactly one real subscriber (Milestone 106) all remain
      exactly as Milestone 114 left them -- unrelated to this milestone.

- [x] **Milestone 116**: real pointer arithmetic (Milestone 115) now
      crosses a real function-call boundary -- an `int *` PARAMETER is
      recognized by `ptr_operand_scale()` too (`tools/cc_src/main.rs`),
      closing the exact gap Milestone 115's own closing disclosure named
      ("pointer arithmetic does NOT cross a function-call boundary ...
      `p + 1` inside a function taking `int *p`/`char *p` silently falls
      through to the ordinary UNSCALED add").

      **A real, disciplined mirror of Milestone 113's own identical fix
      for `char *`**: a new `ParamInfo` field, `is_int_ptr`, set by
      `parse_params()`'s own PRE-EXISTING `int *` branch (the `*` token
      right after `int` was already consumed since Milestone 110 -- "the
      token is simply consumed; nothing downstream needs to know it was
      there," that milestone's own doc comment said at the time -- now it
      finally does), threaded into the parameter's own `VarSlot.is_int_ptr`
      by `collect_vars_for_function()` -- previously hardcoded `0`, per
      Milestone 115's own explicit "always 0 for a parameter" disclosure,
      now genuinely variable. `ptr_operand_scale()` itself -- the
      classifier `EXPR_BINARY`'s own `+`/`-` codegen calls, Milestone 115
      -- needs and receives ZERO code change: it already reads
      `is_int_ptr` generically through `find_var()`, with no awareness of
      whether a given `VarSlot` came from a parameter or a local
      declaration, the same "accepting the grammar plus threading the one
      flag IS the complete feature" shape Milestone 110/113 both already
      established for the analogous `int *`/`char *` cross-call cases.
      **Zero new x86_64 encodings** -- pure field-threading, reusing
      Milestone 115's own scaling codegen completely unchanged.

      **New self-test cases**, all three run in-process
      (`CodegenMode::Callable`), the same tier Milestone 107 through 115
      all already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle is added by this milestone: CASE 114
      (`read_next(int *p) { return *(p + 1); }`, called with a real local
      `int *` -- the real, distinguishing proof `p + 1` both scales
      correctly AND genuinely aliases the real CALLER-side stack slot from
      inside the callee, the same cross-function aliasing Milestone
      96/104's own CASE 96/104 already established for `p[idx]`/`*p`, now
      proven for arithmetic-computed addresses; a pre-fix build -- this
      milestone's own committed Milestone 115 baseline -- would compute an
      unaligned, off-by-one-byte address instead, reading garbage rather
      than the clean `111` this case expects; expected `1`); CASE 115
      (`read_prev(int *p) { return *(p - 1); }`, called with a DECAYED
      ARRAY base -- the real, distinguishing proof a parameter recognizes
      `is_array`-classified arguments too, not only a caller's own `int *`
      local variable, via `-` reaching `arr[1]`'s own real address through
      this compiler's own downward-growing array layout, Milestone 115's
      own disclosure, unchanged; expected `1`); CASE 116 (`at_next(char
      *p) { return *(p + 8); }` -- a real, deliberate VERIFICATION case,
      NOT a new fix: `is_char_ptr` was already threaded into a parameter's
      own `VarSlot` since Milestone 113, so `char *` pointer arithmetic
      through a parameter was already reachable the moment Milestone 115
      landed, just never exercised by a dedicated case; this case closes
      that real, disclosed verification gap AND proves this milestone's
      own `is_int_ptr` threading did not regress the already-correct
      `char *` path -- `at_next` deliberately returns `int`, not `char`,
      so the function's own return-narrowing cannot interact with this
      case's own result; expected `1`).

      **No SNN/evidence-gate work this milestone, and correctly so**: this
      is a pure compiler-frontend milestone -- one new `ParamInfo` field,
      one field-threading line, no kernel-side subsystem decision anywhere
      in it for an evidence-gated hysteresis gate to meaningfully
      subscribe to. The same carve-out Milestone 107 through 115 all
      already established.

      **Verified**: `cargo build` clean from the repo root (kernel + host
      runner, zero warnings). `kernel/assets/cc.elf` rebuilt via the
      project's own pinned-nightly `rustc` recipe (`tools/cc_src/
      README.md`, unchanged) -- 103256 bytes, up from Milestone 115's
      101800; the real `PT_LOAD` `p_memsz` via `readelf -l` is 88176 bytes,
      still 22 pages (the same page count as Milestone 115, just more
      bytes within it) -- comfortably under the 64-page per-segment cap
      and 128-page total cap, no cap change needed. The rebuilt `cc.elf`
      was independently re-derived from the current `main.rs` via a
      from-scratch invocation of the exact same recipe and diffed
      byte-for-byte identical against the committed binary (`cmp`, not
      eyeballed). **Zero new compiler warnings** -- verified the same way
      Milestone 109 through 115 verified their own: reproducing the
      standalone `rustc` invocation against a `git stash` of this
      milestone's own changes (a real, clean Milestone 115 checkout) and
      diffing the sorted warning list against this milestone's own build
      -- byte-for-byte identical 16-warning set.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m116_fresh_boot.log`/`m116_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). `OVERALL_M116=PASS` on both boots -- CASE
      114 returns `1`, CASE 115 returns `1`, CASE 116 returns `1`,
      identically on both boots. Every marker this kernel has ever
      produced was independently re-confirmed `PASS` on both boots via the
      same real reconstruction script Milestone 115 introduced this
      session (concatenating every real ring-3 WRITE-syscall payload in
      file order, plus a separate scan of the kernel's own unfragmented
      direct-to-serial markers) -- 60 total distinct OVERALL-family
      markers and 124 total distinct per-case checks (184 markers
      overall), every one `PASS` on the fresh boot, and identical in both
      marker SET and VALUE on the reused boot with exactly the same one
      real, already-expected exception as every milestone since 70:
      `permissions OVERALL`/`symlinks OVERALL` read `FAIL` on the reused
      boot only, the same pre-existing Milestone 62-era fixture-cleanup
      artifact, confirmed here by diffing the real 54-line `fs self-test:
      ... FAIL` line set directly against Milestone 114's own committed
      `m114_reused_boot.log` -- byte-for-byte identical (`diff`, not
      eyeballed). The fresh boot's own fs self-test passes both suites
      cleanly. `run_preemption_demo()` ran to completion on BOTH boots
      (fresh: `task0=2718 task1=2576 task2=2856`; reused: `task0=2549
      task1=2455 task2=2763` -- real inter-boot timing jitter, consistent
      with every prior milestone's own two-boot pairs) -- zero
      `run_preemption_demo() SKIPPED` lines on either boot. Milestone
      104's own heap-pressure evidence-gate state came out BYTE-IDENTICAL
      to Milestone 108 through 115's own recorded baseline on both boots:
      `samples_taken=68 gate_trip_episodes=0 naive_instantaneous_hits=0
      admissions_denied_so_far=0` -- zero drift, consistent with this
      milestone being pure compiler-frontend work with no new kernel
      allocation path. Zero `panicked at`/`kernel panicked` and zero
      `DOUBLE FAULT`/`TRIPLE FAULT` in either log (checked directly,
      `grep -c`, zero matches in both).

      **Still genuinely open**: no `char **`/`int **`/multi-level pointers
      of any kind (still fully untouched); no `p++`/`p--` (this lexer has
      no `++`/`--` tokens at all, unrelated to and untouched by this
      milestone); pointer-pointer subtraction (real C: an element-count
      difference) is still not implemented, falling through to a plain,
      meaningless raw-address subtract; `int - pointer` (not legal C to
      begin with) is still given no special meaning; when BOTH operands of
      `+` classify as pointers, no scaling is applied to either; pointer-
      arithmetic scaling is still recognized only when the pointer-like
      operand is syntactically a plain `EXPR_IDENT`; no `*p += 1;`/
      `p[idx] += 1;`-style compound assignment through any pointer; `arr
      +/- n` still reaches a DIFFERENT real address than `arr[n]` for a
      nonzero-index-style identity (this compiler's own downward-growing
      layout, disclosed by Milestone 115, not a bug this milestone touches
      either); no type system of any kind, so a mismatched `char *`/
      `int *` argument or return, or a nonsensical `pointer + pointer`
      expression, is accepted without complaint exactly as every other
      shape mismatch already silently is; no NULL-pointer concept and no
      bounds/validity checking on any dereference, index, or arithmetic
      result. `MAX_PARAMS` (8), no `sizeof`, `char` as the only non-`int`
      scalar TYPE, the compiler self-test's own per-compile leak,
      `heap_guard::admit()` covering only its original two call sites, and
      `evidence_gate.rs` still having exactly one real subscriber
      (Milestone 106) all remain exactly as Milestone 115 left them --
      unrelated to this milestone.
- [x] **Milestone 117**: `evidence_gate.rs`'s SECOND real subscriber --
      `scheduler.rs`'s own evidence-gated task-selection fairness
      intervention. The standing "SNN + self-healing every direction"
      instruction (in effect since Milestone 101) explicitly flagged that
      the compiler track had just run 15 consecutive milestones (M102
      through M116) and asked whether a real Tier 13 subsystem milestone
      -- a genuine second `evidence_gate.rs` subscriber -- was now the
      more honest "next" choice, PROVIDED a real, measurable signal to
      gate on actually exists (not fabricated). Milestone 106 had
      explicitly declined to invent one at the time. This milestone found
      one for real, by simulating `scheduler.rs`'s OWN existing dynamics
      host-side (Python, before writing a single line of new kernel code)
      rather than assuming: the SSH-bond-coupled task-slot bank
      (Milestone 4/48) has a real, pre-existing structural asymmetry --
      boundary slots (index 0 and n-1) have only one neighbor, and
      `kill()`-ing an interior slot permanently reshapes its surviving
      neighbors' own coupling -- that measurably grows real fire-count
      unfairness over time, with or without a defect ever being injected.

      **The real numbers that led here** (host-side simulation of this
      file's own pre-Milestone-117 `step()`/`select_winner_ternary`
      logic, reused verbatim in the Rust port below -- confirmed
      byte-identical against the real in-kernel boot log's own numbers,
      see Verified below): Milestone 4's own real scenario (N=8, g=0.6,
      kill slot 3 after a 400-tick warmup, 2000 more ticks) reaches a
      fire-count spread of 77 (min=289, max=366, fairness=0.790) under
      the existing `TIE_EPSILON=0.01` tiebreak. The SAME asymmetry shows
      up with NO kill at all: N=3 (this kernel's real `tasks.rs` boot-time
      task count), 5000 ticks, spread 139 (fairness=0.833) purely from
      the two boundary slots' one-sided coupling.

      **The real intervention -- widening an existing knob, not inventing
      a new mechanism**: `scheduler.rs`'s `select_winner_ternary()` (the
      real ternary tiebreak Milestone 48 already established) now takes
      its tie window (`epsilon`) as a real parameter instead of reading
      the module constant directly. `step()` passes `TIE_EPSILON`
      unchanged -- zero behavior change for the pre-Milestone-117 path.
      The new `step_fairness_gated()` samples the CURRENT real fire-count
      spread among alive slots (via the new `fire_count_spread()`, taken
      BEFORE each tick's own accumulate/select, so a sample reflects the
      state the PREVIOUS tick's selection actually produced) into
      `evidence_gate::record_sample()` as a hit against
      `FAIRNESS_HIT_SPREAD=15` (chosen from the real growth curve above,
      not tuned to make a demo pass) -- the exact same fast-EMA-then-
      hysteresis machinery `heap_guard.rs` already subscribes to, with
      `GATE_ON_THRESHOLD`/`GATE_OFF_THRESHOLD` (0.5/0.2) and `fast_tau`
      (7) reused VERBATIM from `network.rs`/`heap_guard.rs` rather than
      re-tuned (`fast_tau=7` specifically because, unlike `heap_guard.rs`'s
      per-teardown-event sampling, this gate samples every scheduler
      TICK -- the exact per-tick cadence `network.rs`'s own neurons
      already use that constant for). While the gate reads conservative
      (sustained real evidence, not one noisy tick), selection uses
      `FAIRNESS_BOOST_EPSILON=0.1` in place of `TIE_EPSILON` -- MORE
      near-tied candidates get adjudicated by the existing "fewest
      fire_count wins" fairness rule instead of a hairline potential
      difference, pulling a lagging slot back toward parity. `GateKind`
      gains `SchedulerFairness` (`GATE_COUNT` 1 -> 2); `evidence_gate.rs`'s
      own closed-enum design (one array slot per real subscriber, no
      runtime registry) needed exactly the one new variant + one new
      `index()` arm its own module doc already promised.

      **`step_fairness_gated()` -- not `step()` -- is what `tasks.rs`'s
      real timer-driven `timer_tick_switch()` calls as of this
      milestone**: a genuine LIVE production-scheduler behavior change,
      the same "real code path, not just a benchmark demo" standard
      `heap_guard::admit()` set at Milestone 104 (`tasks::spawn()`/
      `run_preemption_demo()` calling it for real, not a trial calling a
      copy). `step()` itself is byte-for-byte unchanged and still what
      main.rs's own deterministic boot-time trial exercises directly for
      the honest "conventional, ungated" comparison -- the same role it
      already played for Milestone 48's ternary-vs-binary trial.
      `main.rs`'s new trial reuses Milestone 4's own EXACT scenario
      (`N_SLOTS`/`DEFECT_SLOT`/`WARMUP_TICKS`/`POST_DEFECT_TICKS`, g=0.6)
      and Milestone 4's own already-computed `(min_t, max_t)` as the
      ungated baseline directly -- no second baseline run duplicated.

      **Honestly, not uniformly a win** (same host-side simulation,
      config unchanged): Milestone 4's own post-defect scenario -- spread
      77 -> 16, fairness 0.790 -> 0.953, gate trips once (tick 71) and
      stays conservative for 1929 of the remaining 2000 ticks. N=3, no
      defect -- spread 139 -> 15, fairness 0.833 -> 0.979. Both real
      wins. But N=8, no defect, 5000 ticks -- spread 167 -> 135, fairness
      0.749 -> 0.795: a real, much SMALLER improvement. The two boundary
      slots' one-sided coupling is a structural asymmetry the tie window
      alone does not fully close, no matter how sustained the evidence --
      an honest limit of this specific intervention, not a bug, and a
      real echo of this author's own separately-documented SSH
      edge-vs-bulk asymmetry findings elsewhere (edge sites behave
      fundamentally differently from bulk sites under this exact
      dimerization pattern) -- disclosed plainly rather than only
      reporting the scenarios that flatter the result.

      **Verified, in the real kernel, not just the host-side model**: the
      real fresh-boot log's own Milestone 117 trial reproduced the
      simulation's numbers exactly -- `min=324 max=340 fairness=0.9529 (7
      slots alive)`, `samples_taken=2400 trip_episodes=1` (2400 =
      `WARMUP_TICKS`(400) + `POST_DEFECT_TICKS`(2000) exactly, confirming
      every real tick fed the shared counters, none skipped), gate
      transition logged for real: `milestone 117: scheduler-fairness gate
      -- entering CONSERVATIVE mode (evidence=0.692, widening tie window
      0.01 -> 0.1)`. A second real transition was observed LATER in the
      SAME boot, during the REAL production `run_preemption_demo()` (N=3,
      60 real ticks): `milestone 117: scheduler-fairness gate -- leaving
      CONSERVATIVE mode (evidence=0.069, tie window back to 0.01)` --
      because `evidence_gate::GateState` is a real, single, whole-boot-
      cumulative store (never reset mid-boot, the same counter philosophy
      `heap_guard.rs` already established), the boot-time trial's own
      throwaway scheduler instance and the REAL, separate, live
      `tasks.rs` scheduler instance genuinely share the SAME
      `SchedulerFairness` counters -- disclosed here explicitly, not
      glossed over, since it means this milestone's own trial is the
      first real thing to touch that gate each boot, not an isolated
      demo. `OVERALL_M117` is a real, deterministic correctness self-test
      -- NOT gated on which way the fairness comparison came out (that's
      an honest empirical finding, reported either way, same discipline
      as Milestone 4/48's own trials): `samples_correct` (exact tick-count
      match), `gate_engaged` (`trip_episodes >= 1`, proving the mechanism
      is real and reachable, not dead code), `alive_correct` (same
      `DEFECT_SLOT` kill left the same 7 slots alive as Milestone 4's own
      ungated trial, proving `step_fairness_gated()`'s accumulate/select/
      fire_count bookkeeping stayed correct under the gate).

      **`cargo build` clean from the repo root (kernel + host runner,
      zero warnings)**. `tools/cc_src/main.rs` and `kernel/assets/cc.elf`
      were NOT touched this milestone (a Tier 13 subsystem milestone, not
      a compiler-frontend one) -- `cc.elf` was independently rebuilt
      anyway via the project's own pinned-nightly `rustc` recipe purely
      as a sanity check, not because anything changed: 103256 bytes,
      byte-for-byte identical to the committed binary (`cmp`), and the
      same 16-warning set as Milestone 116 (no `git stash` diff needed --
      the source producing those warnings is provably unmodified, not
      merely unchanged in effect).

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m117_fresh_boot.log`/`m117_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). Every marker was verified via a real
      reconstruction script written this session (not committed, matching
      every prior session's own disclosed practice) that concatenates
      every real ring-3 WRITE-syscall payload (`usertest.rs`'s syscall 0
      handler's own `>>> ... <<< end of write (N bytes)` framing) in real
      file order, separately from the kernel's own direct `writeln!`
      lines -- this session found a real, concrete case proving the
      concatenation step is NOT optional: the compiler self-test's own
      `OVERALL_M101=` marker is genuinely split across TWO separate real
      WRITE syscalls (`"OVERALL_M101="` as its own complete 13-byte
      payload, `"PASS"`/`"FAIL"` arriving in a separate syscall
      immediately after) -- a naive per-line grep over the raw log would
      never match it. 190 total distinct OVERALL-family + per-case
      markers found on the fresh boot (62 OVERALL-family, 128 real
      per-case `write_check()` results from the compiler's own self-test),
      every one `PASS`; the reused boot shows the identical 190-marker
      set with only the same two pre-existing, already-disclosed
      exceptions every milestone since 70 has recorded (`permissions
      OVERALL`/`symlinks OVERALL` reading `FAIL` on the reused boot only,
      the Milestone-62-era fixture-cleanup artifact). Run through the
      SAME script, Milestone 116's own committed fresh boot log produces
      189 markers; diffing the two marker-name sets directly shows the
      ONLY difference is the addition of `OVERALL_M117=PASS` itself --
      zero markers missing, zero newly `FAIL`ing -- a real, precise
      regression check, not eyeballed. Milestone 4's own trial numbers
      (`min=289 max=366` at g=+0.6, `fairness=0.790`) and Milestone 48's
      own (`ternary`/`binary` both `fairness=0.7486`) came out byte-
      identical to Milestone 116's own committed values, confirming zero
      disturbance to those untouched code paths. Milestone 104/106's own
      heap-pressure gate came out byte-identical across all four boots
      checked (this milestone's fresh/reused plus Milestone 116's own
      committed fresh/reused): `samples_taken=68 gate_trip_episodes=0
      naive_instantaneous_hits=0 admissions_denied_so_far=0`.
      `run_preemption_demo()` completed on both boots with all three real
      task counters > 0 (fresh: `task0=2352 task1=2247 task2=2631`;
      reused: `task0=2301 task1=2298 task2=2549` -- real inter-boot
      timing jitter, consistent with every prior milestone's own two-boot
      pairs, now additionally perturbed by this milestone's own extra
      per-tick `evidence_gate` lock/sample, an expected, harmless real
      cost). Zero `panicked at`/`kernel panicked` and zero `DOUBLE
      FAULT`/`TRIPLE FAULT` in either log (`grep -c`, zero matches in
      both).

      **Still genuinely open**: `FAIRNESS_BOOST_EPSILON` (0.1) was chosen
      from one host-side simulation sweep over one scenario family, not
      an exhaustive search -- the N=8-no-defect finding above already
      shows this specific value has real limits against the boundary-slot
      asymmetry; no systematic sweep of `FAIRNESS_HIT_SPREAD`/
      `FAIRNESS_BOOST_EPSILON` against a wider range of N/g/defect
      combinations has been done. `evidence_gate.rs` now has exactly TWO
      real subscribers (`KernelHeapPressure`, `SchedulerFairness`) -- a
      third (SNN-driven page eviction, a flaky-driver recovery policy) is
      still deliberately not invented, per the same "real signal or
      nothing" discipline this milestone itself just followed to find
      one. The compiler track's own open items (`int **`/multi-level
      pointers, `p++`/`p--`, pointer-pointer subtraction, etc.) are
      entirely untouched by this milestone, exactly as Milestone 116 left
      them. Everything else Milestone 116 disclosed as open (`MAX_PARAMS`
      8, no `sizeof`, `char` as the only non-`int` scalar type, the
      compiler self-test's own per-compile leak, `heap_guard::admit()`
      covering only its original two call sites) remains unchanged,
      unrelated to this milestone.
- [x] **Milestone 118**: real TWO-LEVEL `int` pointers -- `int **p;` local
      declaration, `*p`/`**p` as rvalues, and `**p = expr;` as an lvalue.
      Both Milestone 116's and Milestone 117's own closing disclosures
      named `int **`/multi-level pointers as the well-scoped next compiler
      slice; this milestone takes it, returning to the compiler track
      after Milestone 117's own real Tier 13 subsystem milestone, the same
      "not every milestone needs SNN/evidence-gate work, and correctly so"
      carve-out Milestone 107 through 116 already established for pure
      compiler-frontend work.

      **A real, honest surprise found while building this**: the RVALUE
      half of the feature -- both `*p` (yields the inner `int *` value)
      and `**p` (yields the pointed-to `int`) -- needed ZERO new codegen
      once `int **p;` declarations parse at all. `parse_unary()`'s own `*`
      case has recursed into itself since Milestone 109 (`unary := "*"
      unary | ...`, built so `*` composes with `-`/`~`/`!`, never
      specifically for double dereference), so `**p` already parses as
      `OP_DEREF(OP_DEREF(EXPR_IDENT(p)))` with no parser change at all;
      and `EXPR_UNARY`'s own `OP_DEREF` codegen arm (Milestone 109/112)
      already falls through to the ordinary full-width
      `emit_mov_rax_mem_rax()` load whenever its operand is anything other
      than a plain `EXPR_IDENT` naming a `char *` variable -- exactly the
      shape the OUTER `OP_DEREF` of `**p` always has (its own operand is
      the INNER `OP_DEREF` node, not an `EXPR_IDENT`), so it was already,
      silently, doing the right thing -- that arm's own Milestone 112 doc
      comment even names "a nested dereference `**q`" explicitly as one of
      the shapes that falls through correctly, written five milestones
      before anything could actually construct one.

      **The two genuinely new pieces of work**: (1) parser acceptance of
      the `int **p ...;` declarator grammar -- `parse_double_pointer_decl()`,
      a real sibling of `parse_pointer_decl()`, not a reuse of it with a
      level parameter, since an `int **` declarator needs its own
      distinct `StmtNode.then_body` sentinel (`3`, collision-free against
      the pre-existing `0`/`1`/`2` values) and its own new
      `VarSlot.is_int_ptr_ptr` flag -- `then_body: 2`/`is_int_ptr` already
      mean something different and specific ("single-level `int *`"); and
      (2) the LVALUE write side, `**p = expr;`, which genuinely does need
      new codegen -- the new `STMT_ASSIGN_DEREF2` statement kind. Its
      codegen is `STMT_ASSIGN_DEREF`'s own single-level write sequence
      with exactly one extra instruction inserted at the front: load `p`'s
      own value (`emit_mov_rax_rbp_off()`), dereference it ONE more time
      (`emit_mov_rax_mem_rax()`, the same instruction `EXPR_UNARY`'s
      `OP_DEREF` arm already uses for `*p` as an rvalue) to reach the real
      target `int`'s own address, THEN push/evaluate-the-RHS/pop/store
      exactly as before. Zero new x86_64 encodings anywhere in this
      milestone -- every instruction this feature needs
      (`emit_mov_rax_rbp_off()`, `emit_mov_rax_mem_rax()`,
      `emit_push_rax()`, `emit_pop_reg()`, `emit_mov_mem_rcx_rax()`,
      `emit_mov_rbp_off_rax()`) already existed before it, the same
      "combine, don't invent" discipline Milestone 115's own
      `emit_shl_rax_imm8()` paragraph already named as an ideal, just
      taken one step further here -- this milestone needs no new
      instruction at all, only a new arrangement of old ones.

      `find_var()`'s own return tuple grows to SIX elements (the new
      `is_int_ptr_ptr` field); all NINE of its call sites are updated for
      the new arity, every one of which just ignores the new field -- the
      same "ignore the field you don't need" precedent every prior
      addition (Milestone 110/112/115) already established. This milestone
      adds no new `find_var()` call site at all -- `STMT_ASSIGN_DEREF2`'s
      own codegen resolves `p` via the ordinary `stack_off` field every
      call site has always used, the same way `STMT_ASSIGN_DEREF` already
      does.

      **New self-test cases**, all three run in-process
      (`CodegenMode::Callable`), the same tier Milestone 107 through 116
      all already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle is added by this milestone: CASE 117
      (`int x; int *p; int **pp; x = 42; p = &x; pp = &p; return **pp;` --
      the real, distinguishing proof `**pp` genuinely reaches `x`'s own
      value through two real levels of indirection, AND a real proof the
      new declarator grammar itself landed, since a pre-fix build would
      reject this program at PARSE time, not merely compute the wrong
      answer; expected `42`); CASE 118 (the same shape, but
      `**pp = 99; return x;` -- the real, distinguishing proof of the one
      genuinely new codegen surface, `STMT_ASSIGN_DEREF2`, reaching THROUGH
      both levels to land on `x`'s own real stack slot; expected `99`);
      CASE 119 (a real, decisive ALIASING tripwire -- two separate `int *`
      locals `p1`/`p2`, only `p2`'s address ever taken into `pp` via
      `int **pp = &p2;`, a combined declarator+initializer this milestone
      also exercises for the first time; `**pp = 77; return a + b;` must
      come out `87`, not `30`, proving `is_int_ptr_ptr`'s own new sentinel
      landed on the CORRECT declared slot rather than merely A slot, the
      same standard Milestone 96/104/114's own cross-slot cases already
      established; expected `87`).

      **`cargo build` clean from the repo root (kernel + host runner, zero
      warnings)**. `kernel/assets/cc.elf` rebuilt via the project's own
      pinned-nightly `rustc` recipe (`tools/cc_src/README.md`, unchanged)
      -- 105352 bytes, up from Milestone 116's 103256 (Milestone 117 did
      not touch this file at all); the real `PT_LOAD` `p_memsz` via
      `readelf -l` is 90784 bytes, 23 pages (up one page from Milestone
      116's own 22) -- comfortably under the 64-page per-segment cap and
      128-page total cap, no cap change needed. The rebuilt `cc.elf` was
      independently re-derived from the current `main.rs` via a
      from-scratch invocation of the exact same recipe and diffed
      byte-for-byte identical against the committed binary (`cmp`, not
      eyeballed). **Zero new compiler warnings** -- verified the same way
      Milestone 109 through 116 verified their own: reproducing the
      standalone `rustc` invocation against a `git stash` of this
      milestone's own changes (a real, clean Milestone 117 checkout, since
      Milestone 117 itself never touched this file either) and diffing the
      sorted warning list against this milestone's own build -- byte-for-
      byte identical 16-warning set.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m118_fresh_boot.log`/`m118_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). Every marker was verified via a real
      reconstruction script written this session (not committed, matching
      every prior session's own disclosed practice) that byte-precisely
      slices each real ring-3 WRITE-syscall payload out of the raw log
      using that write's own declared `len=N` (rather than trusting line
      boundaries) and concatenates them in real file order -- correctly
      reassembling markers fragmented across two separate WRITE syscalls,
      the exact real quirk Milestone 117's own session first found and
      disclosed -- plus a separate scan of the kernel's own direct
      `writeln!` lines. Fresh boot: 300 total distinct markers, every one
      `PASS`, zero `FAIL`. Reused boot: the identical 53-line `fs self-test`
      FAIL set every milestone since 70 has recorded (the Milestone-62-era
      fixture-cleanup artifact), byte-for-byte identical in NAME and VALUE
      to Milestone 117's own committed `m117_reused_boot.log` when diffed
      directly through the same script -- zero markers missing, zero newly
      failing anywhere else. Diffing this milestone's own fresh boot
      against Milestone 117's own committed `m117_fresh_boot.log` (296
      markers) through the same script shows the ONLY difference is the
      addition of `OVERALL_M118=PASS` plus the three new case markers
      (296 -> 300, exactly +4) -- a real, precise regression check, not
      eyeballed. Milestone 104/106's own heap-pressure gate came out byte-
      identical to every milestone since 108's own recorded baseline on
      both boots: `samples_taken=68 gate_trip_episodes=0
      naive_instantaneous_hits=0 admissions_denied_so_far=0`. Milestone
      117's own scheduler-fairness gate came out byte-identical to its own
      committed value on both boots too: `samples_taken=2400
      trip_episodes=1`. `run_preemption_demo()` completed on both boots
      with all three real task counters > 0 (fresh: `task0=2647
      task1=2504 task2=2804`; reused: `task0=2663 task1=2467
      task2=2726` -- real inter-boot timing jitter, consistent with every
      prior milestone's own two-boot pairs), zero `SKIPPED` lines on
      either boot. Zero `panicked at`/`kernel panicked` and zero `DOUBLE
      FAULT`/`TRIPLE FAULT` in either log (checked directly, the four
      literal "panicked at"-adjacent substring matches in each log are all
      pre-existing "no panic, no double fault" SELF-TEST CONFIRMATION
      lines Milestone 51/58/61/67 each already print, not real panics --
      confirmed by inspecting each match directly, not merely counting).

      **Still genuinely open**: no `char **` (only `int **` -- mirrors
      `char *`'s own later, separate arrival after `int *`); no `int ***`
      or deeper (exactly one extra level of indirection is recognized, via
      one new dedicated flag, not a general level-counting scheme); no
      `int **` PARAMETER or return value -- `collect_vars_for_function()`
      unconditionally writes `is_int_ptr_ptr: 0` for every parameter this
      milestone, the same real, disclosed one-milestone scope cut
      `is_char_ptr` (112->113) and `is_int_ptr` (115->116) each already
      went through, a real, well-scoped candidate for Milestone 119; no
      pointer arithmetic on an `int **` itself (`p + 1` where `p` is
      `int **` is NOT recognized by `ptr_operand_scale()` -- it silently
      falls through to the ordinary UNSCALED add, the exact same "not yet
      reached" gap `int *` itself had between Milestone 109 and 115); no
      `int **p, *q;`-style mixed-declarator list (`parse_double_pointer_
      decl()` is its own standalone single-declarator statement, the same
      restriction every other declarator shape besides plain
      `make_decl()`'s comma-chain already has); no `p++`/`p--` (this lexer
      still has no `++`/`--` tokens at all); pointer-pointer subtraction
      still not implemented. Everything else Milestone 116/117 disclosed
      as open (`MAX_PARAMS` 8, no `sizeof`, `char` as the only non-`int`
      scalar type, the compiler self-test's own per-compile leak,
      `heap_guard::admit()` covering only its original two call sites,
      `evidence_gate.rs` still having exactly two real subscribers)
      remains unchanged, unrelated to this milestone.
- [x] **Milestone 119**: real two-level `int` pointers (Milestone 118) now
      cross a real function-call boundary -- an `int **` PARAMETER is
      recognized by `parse_params()`/`collect_vars_for_function()` too,
      closing the exact gap Milestone 118's own closing disclosure named
      ("no `int **` PARAMETER or return value").

      **The fix is a real, disciplined mirror of Milestone 113/116's own
      identical fix for `is_char_ptr`/`is_int_ptr`**: a new
      `ParamInfo::is_int_ptr_ptr` field, set by `parse_params()`'s own
      extended lookahead -- a SECOND immediate `*` after the pre-existing
      single-`*` check (mirrored directly from `parse_stmt()`'s own
      identical declaration-site extension, Milestone 118), never checked
      after `char *` (there is no `char **` in this subset, mirroring
      `parse_double_pointer_decl()`'s own `int`-only restriction) --
      threaded into the parameter's own `VarSlot.is_int_ptr_ptr` by
      `collect_vars_for_function()`, previously hardcoded `0` per
      Milestone 118's own explicit scope cut, now genuinely variable.
      `EXPR_UNARY`'s own `OP_DEREF` arm and `STMT_ASSIGN_DEREF2` (both
      Milestone 118) need -- and receive -- ZERO code change: neither one
      has ever cared whether the `VarSlot` it resolves came from a
      parameter or a local declaration, the same "accepting the grammar
      plus threading the one flag IS the complete feature" shape
      Milestone 110/113/116 all already established. Zero new x86_64
      encodings.

      **New self-test cases**, all three run in-process
      (`CodegenMode::Callable`), the same tier Milestone 107 through 118
      all already used for their own new cases -- no new real
      `fork()`/`exec()`/`wait()` cycle is added by this milestone: CASE 120
      (`int read_through(int **pp) { return **pp; }`, called with `&p`
      where `p` is a real local `int *` -- the real, distinguishing proof
      `**pp` correctly reaches the CALLER's own real value THROUGH BOTH
      real levels of indirection from inside the callee, the same real
      cross-function aliasing Milestone 96/104/114 each already
      established for one level, now proven for two; expected `1`); CASE
      121 (`int write_through(int **pp) { **pp = 88; return 0; }` -- the
      write-side mirror, the new `STMT_ASSIGN_DEREF2`'s own first real
      exercise through a function-call boundary, proving it reaches
      THROUGH both levels and lands on the CALLER's own real stack slot,
      not merely inside the callee's own copy of the address; expected
      `88`); CASE 122 (`int combo(int *p, int **pp) { return *p + **pp; }`
      -- a real, deliberate VERIFICATION case, not a new fix, declaring
      BOTH a single-level `int *` parameter and a two-level `int **`
      parameter in the SAME function signature and using both in one
      expression, proving `parse_params()`'s own extended two-star
      lookahead did not regress the pre-existing single-star `int *`
      parameter path (Milestone 116) or leak state across iterations of
      the comma-separated parameter loop; expected `30`).

      **`cargo build` clean from the repo root (kernel + host runner, zero
      warnings)**. `kernel/assets/cc.elf` rebuilt via the project's own
      pinned-nightly `rustc` recipe (`tools/cc_src/README.md`, unchanged)
      -- 106784 bytes, up from Milestone 118's 105352; the real `PT_LOAD`
      `p_memsz` via `readelf -l` is 91704 bytes, still 23 pages (the same
      page count as Milestone 118, just more bytes within it) --
      comfortably under the 64-page per-segment cap and 128-page total
      cap, no cap change needed. The rebuilt `cc.elf` was independently
      re-derived from the current `main.rs` via a from-scratch invocation
      of the exact same recipe and diffed byte-for-byte identical against
      the committed binary (`cmp`, not eyeballed). **Zero new compiler
      warnings** -- verified the same way Milestone 109 through 118
      verified their own: reproducing the standalone `rustc` invocation
      against the project's own pre-Milestone-118 committed baseline and
      diffing the sorted warning list against this milestone's own build
      -- byte-for-byte identical 16-warning set.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m119_fresh_boot.log`/`m119_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). Every marker was verified via the same
      real reconstruction script this session wrote for Milestone 118
      (not committed, matching every prior session's own disclosed
      practice) -- byte-precise WRITE-syscall payload concatenation using
      each write's own declared length. Fresh boot: 304 total distinct
      markers, every one `PASS`, zero `FAIL`. Reused boot: the identical
      53-line `fs self-test` FAIL set every milestone since 70 has
      recorded (the Milestone-62-era fixture-cleanup artifact),
      byte-for-byte identical in NAME and VALUE to this session's own
      committed `m118_reused_boot.log` when diffed directly through the
      same script -- zero markers missing, zero newly failing anywhere
      else. Diffing this milestone's own fresh/reused boots against this
      session's own committed Milestone 118 fresh/reused logs through the
      same script shows the ONLY difference on either boot is the addition
      of `OVERALL_M119=PASS` plus the three new case markers (300 -> 304
      fresh, exactly +4) -- a real, precise regression check, not
      eyeballed. Milestone 104/106's own heap-pressure gate and Milestone
      117's own scheduler-fairness gate both came out byte-identical to
      their own recorded baselines on both boots (`samples_taken=68
      gate_trip_episodes=0 naive_instantaneous_hits=0
      admissions_denied_so_far=0`; `samples_taken=2400 trip_episodes=1`).
      `run_preemption_demo()` completed on both boots with all three real
      task counters > 0 (fresh: `task0=2446 task1=2518 task2=2876`;
      reused: `task0=2507 task1=2489 task2=2778` -- real inter-boot timing
      jitter, consistent with every prior milestone's own two-boot pairs),
      zero `SKIPPED` lines on either boot. Zero real `panicked at`/`kernel
      panicked`/`DOUBLE FAULT`/`TRIPLE FAULT` in either log -- the four
      literal substring matches per log are, as in Milestone 118, all
      pre-existing "no panic, no double fault" self-test confirmation
      lines (Milestone 51/58/61/67), confirmed by inspecting each match
      directly and excluding them, not merely counting raw hits.

      **Still genuinely open**: no `char **` (only `int **`, PARAMETER
      included -- `parse_params()`'s own new lookahead is deliberately
      never checked after `char *`); no `int ***` or deeper, as a
      parameter or otherwise; no `int **` RETURN value -- this compiler's
      return type is still exactly `int`/`char`, unrelated to and
      untouched by this milestone's own parameter-only fix; no pointer
      arithmetic on an `int **` itself, as a parameter or a local (`p + 1`
      where `p` is `int **` is still not recognized by
      `ptr_operand_scale()`, the exact same real, disclosed gap Milestone
      118 named, now also true of the parameter case this milestone adds);
      no `int **p, *q;`-style mixed-declarator list; no `p++`/`p--`;
      pointer-pointer subtraction still not implemented. Everything else
      Milestone 118 disclosed as open (no `char **`, no `int ***`, no
      `MAX_PARAMS`/`sizeof`/self-test-leak items, `evidence_gate.rs` still
      having exactly two real subscribers) remains unchanged, unrelated to
      this milestone.

- [x] **Milestone 120**: an `int **` RETURN value -- `int **f(...)` -- is
      now real (`tools/cc_src/main.rs`), closing the exact other half of the
      gap Milestone 119's own closing disclosure named ("an `int **` RETURN
      value is still not implemented, this compiler's return type still
      being exactly `int`/`char`").

      **The fix is one more token of lookahead in one grammar position,
      nothing else**: `parse_function()`'s own `int` return-type branch
      already accepted an optional single `*` since Milestone 110 (`int
      *f(...)`); this milestone extends it to accept a SECOND immediate `*`
      too, mirroring `parse_stmt()`'s own two-star declaration-site
      lookahead (Milestone 118) and `parse_params()`'s own two-star
      parameter lookahead (Milestone 119) at the one remaining place in the
      grammar that had not yet been extended. Deliberately narrow, matching
      every earlier pointer-return step: `int`-only (no `char **` -- there
      is no `char **` anywhere in this subset, the same restriction
      `parse_double_pointer_decl()` and Milestone 119's own parameter fix
      both already carry), and a third `*` (`int ***f(...)`) is not consumed
      here -- it falls straight through to `self.expect(TOK_IDENT)`, which
      rejects it as an ordinary `UnexpectedToken` parse error, the same "no
      `int ***`, no deeper" restriction Milestone 118/119 already
      established for locals and parameters, now also true of the
      return-type position.

      **No new `FuncDef` field, and -- genuinely, not just "we got lucky" --
      zero codegen change anywhere, on EITHER side of the call boundary**:
      `STMT_RETURN`'s own codegen has only ever asked one question,
      `is_char_return`, and that question was already answered correctly
      for two levels of pointer indirection the moment it was answered
      correctly for one (Milestone 110's own reasoning transfers completely
      unchanged -- a pointer return of any depth must never be narrowed, RAX
      already holds the right raw 8-byte value regardless of how many stars
      its declared type has). And on the CALLING side, `EXPR_CALL`'s own
      codegen has never consulted the callee's `is_char_return` at all --
      only the callee's OWN `STMT_RETURN` does, purely for its own narrowing
      decision -- so a caller storing a call's result into a local `int
      **q` (Milestone 118's own declarator) and reading or writing through
      it with `**q` (Milestone 118's own `STMT_ASSIGN_DEREF2`) was ALREADY
      fully correct the instant `int **` locals and `int **` parameters
      both existed as real features; this milestone's only real, new work
      is the missing token of lookahead. Zero new x86_64 encodings.

      **New self-test cases**, all three run in-process
      (`CodegenMode::Callable`), the same tier every prior pointer milestone
      used for its own new cases -- no new real `fork()`/`exec()`/`wait()`
      cycle is added by this milestone: CASE 123 (`int **identity_pp(int
      **pp) { return pp; }`, its result stored into a local `int **q`, then
      `**q` read as an RVALUE -- the real, distinguishing proof that an
      `int **` RETURN value crosses the call boundary un-corrupted and
      reaches the caller's own real value through TWO real levels of
      indirection, the return-type mirror of Milestone 113's own CASE 105
      for a `char *` return, one level; expected `77`); CASE 124 (the
      write-side mirror -- `**q = 99;` through a local `int **q` populated
      from an `int **` RETURN value must reach THROUGH both real levels of
      indirection and land on the caller's own real stack slot, not merely
      inside a copy; expected `99`); CASE 125 (a real, deliberate
      VERIFICATION case, not a new fix -- an `int *`-returning function
      AND an `int **`-returning function declared in the SAME program, both
      results used together in one expression, proving
      `parse_function()`'s own extended two-star lookahead did not regress
      the pre-existing single-star `int *` return path, the return-type
      mirror of CASE 122's own parameter-side coexistence check; expected
      `20`).

      **No SNN/evidence-gate work this milestone, and correctly so**: this
      is pure compiler-frontend work -- one extra token of lookahead in one
      grammar position, zero new fields, zero new codegen -- with no kernel
      subsystem decision anywhere in it for an evidence-gated hysteresis
      gate to meaningfully subscribe to. The same carve-out Milestone 107
      through 119's own pure-frontend milestones already established.

      **`cargo build` clean from the repo root (kernel + host runner, zero
      warnings)**. `kernel/assets/cc.elf` rebuilt via the project's own
      pinned-nightly `rustc` recipe (`tools/cc_src/README.md`, unchanged) --
      108512 bytes, up from Milestone 119's 106784; the real `PT_LOAD`
      `p_memsz` via `readelf -l` is 93432 bytes, still 23 pages (the same
      page count as Milestone 119, just more bytes within it) --
      comfortably under the 64-page per-segment cap and 128-page total cap,
      no cap change needed. The rebuilt `cc.elf` was independently
      re-derived from the current `main.rs` via a from-scratch second
      invocation of the exact same recipe and diffed byte-for-byte identical
      against the committed binary (`cmp`, not eyeballed). **Zero new
      compiler warnings** -- verified the same way every prior milestone
      verified its own: reproducing the standalone `rustc` invocation
      against the project's own pre-Milestone-120 committed baseline (via
      `git stash`) and diffing the sorted warning list against this
      milestone's own build -- the same 16 warnings, same names, same
      messages; only their line numbers shifted (this milestone's own ~90
      lines of new top-of-file doc comment pushed every later line down),
      confirmed by diffing the warning TEXT with line numbers stripped, not
      merely eyeballed.

      Two real QEMU boots against a genuinely fresh disk (`target/
      persist.img`/`persist2.img` deleted first), `cargo run -- bios`,
      fresh then reused (`m120_fresh_boot.log`/`m120_reused_boot.log`), no
      stray `qemu-system-x86_64.exe` process before or after either boot
      (checked via `tasklist`). Every marker was verified via a real script
      this session wrote (not committed, matching every prior session's own
      disclosed practice) that reconstructs the exact WRITE-syscall payload
      stream from each raw serial log (concatenating every `milestone 31:
      syscall WRITE ... raw bytes >>> ... <<< end of write` block using each
      write's own declared byte length, correctly handling the real
      split-marker fragmentation this project's own house style discloses
      -- `OVERALL_M120=` and its own trailing `PASS` were confirmed to
      arrive as two SEPARATE WRITE syscalls in this session's own fresh
      boot, reassembled correctly), plus a direct raw-text scan for the
      older direct-serial `milestone NN: self-test -- OVERALL: PASS` /
      `fs self-test: <name> OVERALL=PASS` styles. Fresh boot: 198 total
      distinct markers, every one `PASS`, zero `FAIL` -- CASE 123
      returns `77`, CASE 124 returns `99`, CASE 125 returns `20`, all three
      matching their hand-computed expectations exactly, confirmed by
      reading the real `returned=... (expected ...)` text out of the
      reconstructed stream, not merely trusting the boolean `case*_ok` flag.
      Reused boot: the identical 54-line `fs self-test` FAIL set every
      milestone since 70 has recorded (the Milestone-62-era
      fixture-cleanup artifact), byte-for-byte identical in NAME and VALUE
      to this session's own committed `m119_reused_boot.log` when diffed
      directly (`diff`, not eyeballed) -- zero markers missing, zero newly
      failing anywhere else. Diffing this milestone's own fresh/reused
      boots against this session's own committed Milestone 119 fresh/reused
      logs through the same script shows the ONLY difference on either boot
      is the addition of `OVERALL_M120=PASS` plus the three new case
      markers (194 -> 198 total markers on both boots, exactly +4) -- a
      real, precise regression check, not eyeballed. Milestone 104/106's
      own heap-pressure gate and Milestone 117's own scheduler-fairness gate
      both came out byte-identical to their own recorded baselines on both
      boots (`samples_taken=68 gate_trip_episodes=0
      naive_instantaneous_hits=0 admissions_denied_so_far=0`;
      `samples_taken=2400 trip_episodes=1`). `run_preemption_demo()`
      completed on both boots with all three real task counters > 0 (fresh:
      `task0=2512 task1=2567 task2=2827`; reused: `task0=2671 task1=2529
      task2=2646` -- real inter-boot timing jitter, consistent with every
      prior milestone's own two-boot pairs), zero `SKIPPED` lines on either
      boot. Zero real `panicked at`/`kernel panicked`/`DOUBLE FAULT`/`TRIPLE
      FAULT` in either log -- confirmed by direct `grep -c`, zero matches in
      both (the five other raw `FAIL`/`FAILED` substring hits on the fresh
      boot -- syscall-error-path self-tests for `READ`/`WAIT`/`SBRK`/
      `GETPGID` plus Milestone 6's own "no keystrokes received" interactive
      -shell line -- are the same pre-existing, already-disclosed lines
      Milestone 119's own committed fresh boot log has too, byte-for-byte
      identical content, not a new regression).

      **Still genuinely open**: no `char **` of any kind (only `int **`,
      return value included -- this branch is deliberately `int`-only,
      mirroring `parse_double_pointer_decl()`'s own `int`-only restriction);
      no `int ***` or deeper, as a return type, parameter, or local (a third
      `*` here falls through to `self.expect(TOK_IDENT)` and is rejected as
      an ordinary parse error, not silently accepted or truncated); no
      pointer arithmetic on an `int **` of any kind, return value included
      (`ptr_operand_scale()` still does not recognize `is_int_ptr_ptr` --
      the same real, disclosed gap Milestone 118/119 each already named,
      now also true of a returned `int **`); no `int **p, *q;`
      mixed-declarator list; no `p++`/`p--`; pointer-pointer subtraction
      still not implemented. Everything else Milestone 119 disclosed as
      open (`MAX_PARAMS`/`sizeof`/self-test-leak items, `evidence_gate.rs`
      still having exactly two real subscribers) remains unchanged,
      unrelated to this milestone.

- [x] **Milestone 121**: real pointer arithmetic on an `int **` itself --
      `pp + n` and `n + pp` -- closing the exact gap Milestone 118 first
      named and Milestone 119/120 each re-confirmed still open. **The fix
      is one new disjunct** in `ptr_operand_scale()`'s own classification
      match arm: `is_int_ptr_ptr != 0` is added alongside the pre-existing
      `is_array != 0 || is_int_ptr != 0` check that already selects
      `Some(8)`. Zero new x86_64 encodings, zero `EXPR_BINARY` `+`/`-`
      codegen change -- it already consumes the `Option<u8>` scale
      generically, unaware which `VarSlot` field produced `Some(8)`.
      Honest observation from the investigation: `is_int_ptr_ptr` was
      already fully threaded everywhere `ptr_operand_scale()` reads from
      (`VarSlot.is_int_ptr_ptr` for a local since M118, `ParamInfo` since
      M119), unlike `is_int_ptr`, which needed two separate milestones
      (M115 local, M116 parameter) to reach the same place -- so this one
      fix unlocks `int **` arithmetic for a local, a parameter, AND
      (transitively, via a returned `int **`, M120) all in the same
      milestone. New self-test cases: CASE 126 (`pp + 1`, pointer on the
      left, expected `1`), CASE 127 (`1 + pp`, pointer on the right, the
      genuinely distinct `emit_shl_rax_imm8` path, expected `1`), CASE 128
      (verification: an `int **` PARAMETER, `**(pp + 1)` across a real
      function-call boundary, expected `111`), CASE 129 (verification: a
      `char *` scale-1 and an `int **` scale-8 pointer arithmetic in the
      SAME expression, proving the new disjunct did not disturb the
      `is_char_ptr` branch, expected `11`). **Verified**: `cargo build`
      clean; `cc.elf` rebuilt via the pinned-nightly recipe, 110616 bytes
      (up from M120's 108512), 24 pages, independently re-derived
      byte-for-byte identical; compiler warnings byte-identical to the
      M120 baseline via `git stash` diff. Two real QEMU boots
      (`m121_fresh_boot.log`/`m121_reused_boot.log`) against a fresh disk:
      fresh boot CASE 126/127/128/129 return `1`/`1`/`111`/`11` exactly as
      hand-computed, all four markers PASS, `OVERALL_M121=PASS`, zero FAIL
      anywhere; reused boot has the same pre-existing 37-line fs
      self-test FAIL set every milestone since 70 has recorded, plus
      `OVERALL_M121=PASS`; every pre-existing `OVERALL_Mxxx` marker
      byte-identical to the committed M120 logs on both boots; heap-
      pressure gate (M104/106) and scheduler-fairness gate (M117) both at
      their recorded baselines; zero real panics or double/triple faults.
      **Still genuinely open**: `int **` pointer-pointer SUBTRACTION
      (`pp1 - pp2`) is still unimplemented, the same gap M115 named for
      `int *`/`char *` and never closed for any depth; `pp[1]` was not
      touched or newly verified this milestone; no `char **` of any kind;
      no `int ***` or deeper; no `int **p, *q;` mixed-declarator list; no
      `p++`/`p--`. Pure compiler-frontend work -- correctly no
      SNN/evidence-gate angle this milestone.

- [x] **Milestone 122**: real pointer `-` pointer subtraction (`p1 - p2`,
      real C's element-count-difference meaning), closing the exact gap
      Milestone 115 first named and Milestone 116 through 121 each left
      standing. **One new disjunct** in `EXPR_BINARY`'s own `-` codegen:
      when `ptr_operand_scale()` classifies BOTH operands as pointers
      (`(Some(sc), Some(_))`, previously routed to a plain unscaled byte
      subtract via the `_` fall-through), the raw byte `sub` still runs,
      then RAX is divided by the pointee size with `emit_sar_rax_imm8(3)`
      -- this milestone's one genuinely new x86_64 encoding (`SAR r/m64,
      imm8`, REX.W `C1` /7; both halves already independently
      SDM-verified separately by M108/115 and M79). `sc == 1` (both
      `char *`) takes no shift -- the byte difference already IS the
      element count. **Arithmetic, not logical, shift is load-bearing**: a
      valid `p1 - p2` with `p1 < p2` is a real negative count, and only
      `SAR` sign-extends the byte difference correctly. Zero new parser
      work -- `p1 - p2` has always parsed as an ordinary `EXPR_BINARY`,
      identical node shape to `p - 1` since M115, only the codegen's
      reading of a both-operands-are-pointers `-` changes. Honest
      observation: since `is_int_ptr_ptr` was already wired to
      `Some(8)` by M121, this ONE `-` disjunct reaches `int **`
      pointer-pointer subtraction (`pp1 - pp2`) in the same milestone as
      the `int *`/decayed-array case, with no `int **`-specific line
      anywhere. New self-test cases: CASE 130 (`int *` ptr-ptr, raw byte
      diff 24 then /8, expected `3`), CASE 131 (`char *` ptr-ptr, `sc ==
      1`, no shift, expected `8`), CASE 132 (the SIGNED case, `p1 < p2`,
      expected `1`, which a logical `SHR` would fail and only `SAR`
      passes), CASE 133 (`int **` ptr-ptr, verification that the same one
      disjunct reaches `int **` too, expected `1`), CASE 134 (a
      regression guard, structural twin of the pre-existing CASE 112,
      expected `3`). **Verified**: `cargo build` clean; `cc.elf` rebuilt,
      112920 bytes (up from M121's 110616), 24 pages (same page count),
      independently re-derived byte-for-byte identical; compiler warnings
      byte-identical to the M121 baseline. Two real QEMU boots
      (`m122_fresh_boot.log`/`m122_reused_boot.log`) against a genuinely
      fresh disk, run under WHPX acceleration with `kernel-irqchip=off`
      (this box's QEMU 11.0.50 dev build hangs milestone 66's IDE
      slave-device probe with no slave present; a real blank slave at ide
      index 3 lets it PASS on both boots): fresh boot CASE 130-134 all
      PASS, `OVERALL_M122=PASS`, `OVERALL=PASS`, milestone 66 PASS, the
      same four deliberate negative-path syscall self-tests as M121's own
      committed fresh log (byte-parity); reused boot `OVERALL_M122=PASS`,
      milestone 66 PASS, the identical pre-existing 54-line fs self-test
      FAIL set every milestone since 70 has recorded (`OVERALL=FAIL` on
      the reused boot for that reason alone, exactly as M121). **Still
      genuinely open**: no `char **` of any kind; no `int ***` or deeper;
      no `int **p, *q;` mixed-declarator list; no `p++`/`p--`; `pp[1]`
      (`EXPR_INDEX` through an `int **`) still has no dedicated proving
      case. Pure compiler-frontend work -- correctly no SNN/evidence-gate
      angle this milestone.

- [x] **Milestone 152**: `evidence_gate.rs`'s THIRD real subscriber --
      the NIC's ARP wait budget (`GateKind::NetArpUnreachable`), the
      first milestone since M123 with an SNN / self-healing angle
      (Milestones 124-151 were compiler-frontend only, against the
      standing "SNN + self-healing every direction" instruction). **Found,
      not invented** (the rule M106 set): `arp_resolve()`/`icmp_ping()`/
      `udp_send_resolved()` waited up to 200 poll windows of ~1,000,000
      spins for a reply on EVERY call, with no memory that the previous
      call to a dead host had just burned the same budget (`no retry/
      backoff` since M47). Measured, ungated: 600 polls for three
      resolves to 10.0.2.99 (nothing on QEMU slirp answers) =
      ~2.5e11 TSC cycles (2.55e11 fresh boot, 2.54e11 reused boot), i.e.
      ~8.3e10 cycles per dead-host resolve. **Mechanism**: same
      EMA-then-hysteresis state machine as M104/M117 (`fast_tau=1`, ON
      0.4 / OFF 0.2; `hit` = no reply). Conservative mode shrinks the
      wait to a 20-poll PROBE (never a give-up: every call still tries,
      so a peer that returns is noticed on the next call). **Pre-registered
      predictions, all met exactly on both boots**: A) three dead resolves
      healthy cost 600 polls and trip the gate on the 3rd (evidence
      0.488); B) three more cost 60 polls (2.5e10 cycles, ~10x cheaper,
      matching the 10x poll cut); C) the real gateway still resolves while
      conservative, in 1 poll (budget 20); D) six further successes
      release the gate (evidence 0.193) and the next dead resolve costs
      200 again -- not sticky, and the self-test leaves the gate released.
      Baseline is the ungated phase A itself (gate cannot trip before its
      3rd sample), not an assumed number. **Honest limits**: one global
      gate, not per-host -- a healthy host mixed into a stream of dead ones
      keeps the gate conservative, and a genuinely SLOW live peer needing
      more than 20 polls would be missed in conservative mode (the slirp
      gateway needs 1; no slow peer exists to measure). ICMP/UDP receive
      waits are unchanged. Two real QEMU boots (`m152_fresh_boot.log`/
      `m152_reused_boot.log`): 72 aggregate `OVERALL` markers all PASS on
      both, M47/M55/M56 NIC self-tests unchanged, zero panics or faults.

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
