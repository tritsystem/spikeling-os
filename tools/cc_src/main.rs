// MILESTONE 67/68/69 test payload -- Tier 3's first three real slices,
// all in this one program:
//   - MILESTONE 67: a real lexer + minimal recursive-descent parser for a
//     genuinely tiny C subset, producing a real AST via malloc()-allocated
//     nodes with pointer links. See README.md's own Milestone 67 entry
//     for the overall Tier 3 toolchain strategy this milestone sets.
//   - MILESTONE 68: real x86_64 machine-code generation from that AST --
//     CodegenMode::Callable emits code invoked in-process through a plain
//     Rust function pointer (no assembler/linker/exec() needed).
//   - MILESTONE 69: a real, from-scratch ELF64 *writer*
//     (build_elf64_standalone(), below) wraps CodegenMode::Standalone
//     output (ending in a real sys_exit() sequence instead of `leave;
//     ret`) in a real ELF64 image, written to the real on-disk filesystem
//     and run through the kernel's own real, pre-existing exec() syscall
//     -- see README.md's own Milestone 69 entry for the full reasoning.
// Built with this project's own pinned Rust nightly toolchain
// (rustc --target x86_64-unknown-none + rust-lld, same recipe as every
// other tools/*_src build) and run as a REAL ring-3 spikeling-os
// process via the existing exec()/ELF-loading path -- not a host-side
// module that never actually executes on the target OS.
//
// Subset-C grammar this milestone actually implements (intentionally
// tiny -- a single function, integer variables, and the four
// arithmetic operators, nothing else):
//   program     := function
//   function    := "int" IDENT "(" ")" "{" stmt* "}"
//   stmt        := decl_stmt | assign_stmt | return_stmt
//   decl_stmt   := "int" IDENT ";"
//   assign_stmt := IDENT "=" expr ";"
//   return_stmt := "return" expr ";"
//   expr        := term (("+" | "-") term)*
//   term        := factor (("*" | "/") factor)*
//   factor      := INTLIT | IDENT | "(" expr ")"
//
// MILESTONE 70 grows this grammar with comparison operators and
// if/else statements -- the next real increment in dependency order,
// checked directly against the grammar above before picking it:
// this file's own expression grammar (unchanged since Milestone 67)
// had NO comparison operators at all, only the four arithmetic ones,
// so an if-statement built first would have nothing meaningful to
// branch on beyond raw arithmetic truthiness. Comparisons come first;
// if/else (needing real conditional-jump codegen: cmp/test + Jcc, and
// real forward-reference jump-target patching) rides on top of them in
// the SAME milestone, since a comparison operator with no consumer and
// an if-statement with no comparisons to test are each individually a
// weaker, less-honest slice than the two shipped together. While-loops
// (backward jumps, reusing this same patch machinery) are the natural
// next Tier 3 increment after this one -- not attempted here, see this
// milestone's own "still genuinely open" section.
//   cond_expr   := expr (relop expr)?
//   relop       := "==" | "!=" | "<" | ">" | "<=" | ">="
//   assign_stmt := IDENT "=" cond_expr ";"
//   return_stmt := "return" cond_expr ";"
//   if_stmt     := "if" "(" cond_expr ")" "{" stmt* "}"
//                  ("else" "{" stmt* "}")?
//   stmt        := decl_stmt | assign_stmt | return_stmt | if_stmt
//   factor      := INTLIT | IDENT | "(" cond_expr ")"
// (factor's parenthesized case now recurses through cond_expr, not the
// narrower expr, so a parenthesized comparison like `(a < b)` is a
// legal sub-expression, e.g. as an operand of arithmetic on its own
// 0/1 result -- a real, deliberate widening, not an oversight.)
// No comparison chaining (`a < b < c`) and no else-if without its own
// braces (`else { if (...) ... }` works; a bare `else if` does not) --
// both real, disclosed scope cuts to keep this one slice honestly
// small; see this milestone's own closing disclosure.
//
// MILESTONE 71 grows this grammar with while-loops -- exactly the next
// increment Milestone 70's own closing disclosure named ("while-loops
// (backward jumps, reusing this same forward-patch machinery) are the
// natural next Tier 3 grammar increment"), picked over function
// parameters/calls because it is the smaller, cleanly-dependency-ordered
// step: the forward-patched jz/jmp machinery Milestone 70 built for
// if/else already covers a while-loop's own "jump past the body when
// the condition is false" exit edge, and the ONE genuinely new piece --
// an unconditional jump BACKWARD to a target that was already emitted
// (the condition re-check, at the top of each iteration) -- is a real
// but small addition to that same CodeBuf machinery, not a new codegen
// strategy. Function parameters/calls need a real calling convention
// and per-function stack frames, a substantially bigger step, left for
// its own milestone.
//   while_stmt := "while" "(" cond_expr ")" "{" stmt* "}"
//   stmt       := decl_stmt | assign_stmt | return_stmt | if_stmt
//                 | while_stmt
// No `break`/`continue` (a real, disclosed scope cut -- see this
// milestone's own closing disclosure): the only way out of a
// Milestone-71 while-loop is its own condition going false, or an
// early `return` from inside the body (already legal -- return_stmt is
// an ordinary stmt, usable anywhere stmt* appears, including inside a
// while body, and already emits its own real epilogue on the spot).
//
// MILESTONE 72 grows this grammar with function parameters and function
// calls -- exactly the "next genuinely bigger step" Milestone 71's own
// closing disclosure named, now that arithmetic/comparisons/if-else/
// while-loops together already let a SINGLE function express real,
// nontrivial control flow, but there was still no way to factor any of
// it into more than one function. Needs a real x86_64 calling convention
// (this milestone picks integer arguments 1-4 in RDI/RSI/RDX/RCX,
// extending this kernel's own real syscall-ABI register ordering rather
// than inventing a new one -- see param_reg()'s own doc comment further
// below), real per-function stack frames (a callee's own prologue spills
// each incoming argument register to its own rbp-relative slot, sharing
// the same flat slot scheme this AST's local `int` declarations already
// use), and real call-site codegen (evaluate+push every argument, pop
// them back in reverse into the argument registers, a real `call rel32`,
// the callee's own ordinary `leave; ret` bringing control back with its
// result already in RAX, exactly gen_expr()'s own existing
// "every node leaves its value in RAX" postcondition).
//   program     := function+
//   function    := "int" IDENT "(" params? ")" "{" stmt* "}"
//   params      := "int" IDENT ("," "int" IDENT)*
//   factor      := INTLIT | IDENT | IDENT "(" args? ")" | "(" cond_expr ")"
//   args        := cond_expr ("," cond_expr)*
// `program` (a real production for the first time -- Milestone 67-71 only
// ever had ONE function per source, parsed directly via parse_function())
// is `function+`, up to MAX_FUNCS (4); a function's own `params` is
// `"int" IDENT` repeated, up to MAX_PARAMS (4), each becoming an ordinary
// declared variable inside that function's own body (real, deliberate
// field-reuse: a parameter and a local both end up as the exact same
// VarSlot shape). `factor`'s IDENT case is now genuinely ambiguous
// between a variable reference (Milestone 67's original shape, unchanged)
// and a call expression -- resolved with one token of lookahead after
// the IDENT itself. A real, disclosed, deliberate scope cut for this
// first slice (see gen_program()'s own doc comment further below for the
// full reasoning): a callee must be defined at or before its own caller
// in source order -- no forward calls, no mutual recursion; direct self-
// recursion is mechanically reachable by this codegen but NOT tested or
// verified by this milestone's own self-test cases. No function may be
// called as a bare expression-statement (a call may only appear as part
// of an assign_stmt/return_stmt/another call's own argument -- this
// subset still has no "expression statement" production at all, an
// existing gap since Milestone 67, unwidened here).
//
// MILESTONE 73 adds NO new grammar at all -- it closes the one real,
// explicitly disclosed UNKNOWN Milestone 72 left behind: "direct self-
// recursion is mechanically reachable by this codegen but NOT tested or
// verified" (gen_program()'s own doc comment, unchanged in shape by this
// milestone). Checked directly against gen_program() before picking this
// as the next slice (not just re-reading the README's own prior wording):
// a function's own `FuncSym` (name, code_off, param_count) is written into
// `funcs_ptr` BEFORE that function's own body is compiled, so a call
// inside a function's own body to ITS OWN name resolves against an
// already-real, already-written table entry with zero special-casing --
// the exact same `find_func()`/`emit_call()` path an ordinary call to an
// earlier-defined sibling function already takes. Nothing in this file was
// grammar-blind to self-recursion; the gap was purely evidentiary --
// "the mechanism looks right" was never actually exercised end-to-end.
// This milestone closes that gap with two new self-test cases (30, 31)
// using ordinary, already-existing grammar (if/else, arithmetic, a single
// int parameter) with no new AST node kind, no new CodeGenError variant,
// and no new CodeBuf encoding -- purely a verification slice, honestly
// scoped as exactly that, not dressed up as a grammar addition. One real,
// disclosed boundary this milestone does NOT close: this kernel gives
// each process exactly one 4KiB stack page (see kernel/src/process.rs),
// and neither this codegen nor the kernel itself enforces any stack-depth
// guard on a self-recursive call chain -- both self-test cases below use
// small, hand-verified recursion depths (well under 4KiB of real frame
// usage) precisely because deep self-recursion has a real, unmeasured,
// undisclosed-by-this-milestone risk of silently overrunning that single
// page; this milestone verifies CORRECTNESS at shallow depth, not SAFETY
// at unbounded depth, and says so plainly rather than implying the latter.
//
// MILESTONE 74 grows codegen (not grammar -- the same "no new AST node
// kind" shape Milestone 73 already established, one level down the
// stack) to remove the "callee must be defined at or before its own
// caller in source order" restriction Milestone 72's own doc comment
// above named and Milestone 73's own closing disclosure re-confirmed
// still open: real FORWARD calls, and by direct consequence real MUTUAL
// recursion (two functions each calling the other, which is only
// possible once at least one direction of that pair no longer needs its
// callee already compiled). Picked over the other real, dependency-
// ordered candidates named in Milestone 73's own closing disclosure
// (growing the grammar itself -- unary minus, `&&`/`||`; raising
// MAX_FUNCS/MAX_PARAMS) because it is the one that unlocks a genuinely
// new CLASS of program this subset could not express AT ALL before today
// (no amount of reordering source lines makes mutual recursion
// expressible under the old restriction), not just a wider version of
// something already possible, and because gen_program()'s own existing
// FuncSym table + CodeBuf's own existing emit-placeholder/patch_rel32
// machinery (built for if/else's forward jumps, Milestone 70) already
// carry almost the whole real mechanism needed -- checked directly
// against both before starting, not assumed from the README's own prior
// wording.
//
// The real two-pass restructuring: gen_program() now walks the function
// list TWICE. Pass one (new) registers every function's own name and
// param_count into `funcs_ptr` up front, before any body is compiled,
// with a real sentinel `UNRESOLVED_CODE_OFF` (u64::MAX -- never a real
// in-buffer offset, this subset's CodeBuf is capped at 2048 bytes) in
// each entry's own `code_off` field standing in for "not compiled yet".
// Pass two is the original per-function compile loop, unchanged in
// shape, except it now OVERWRITES each function's own already-present
// table entry with its real `code_off` right before compiling that
// function's body (same "own entry is real before own body compiles"
// property Milestone 73 already verified for self-recursion, now also
// true for every OTHER function in the table from the very start of pass
// two, not just those compiled so far). A call site's own find_func()
// lookup (extended to also return the callee's own table INDEX, not
// just its code_off/param_count) now always finds a real name/param_count
// match anywhere in the program regardless of source order; if the
// matched entry's `code_off` is still the unresolved sentinel (a genuine
// forward reference -- the callee hasn't been compiled yet), gen_expr()
// emits a new `call rel32` PLACEHOLDER (`CodeBuf::emit_call_placeholder()`,
// the exact same "opcode + 4 zero bytes, return the field's own offset"
// shape emit_jz_placeholder()/emit_jmp_placeholder() already established
// for if/else, generalized to CALL's own 0xE8 opcode) and records a real
// pending-patch entry (the field's own offset, the callee's own table
// index) in a new, small, fixed-cap list (`MAX_PENDING_CALLS`, real,
// disclosed, deliberately small headroom, the same "small real headroom,
// not maximal generality" discipline MAX_FUNCS/MAX_PARAMS/MAX_VARS above
// already established) threaded through gen_expr()/gen_stmt_list() the
// same inert-when-unused way funcs_ptr/nfuncs already are. An ordinary
// call to an ALREADY-compiled function (backward reference, or self)
// still takes the exact original single-instruction `emit_call()` path
// with no placeholder and no patch-list entry at all -- this milestone
// is a real, additive capability, not a rewrite of the fast path
// Milestone 72 already verified. Once gen_program()'s own per-function
// loop finishes (every function's own real code_off is now known, full
// stop), a new, final backpatch pass walks the pending list and writes
// each placeholder's real rel32 displacement against its own callee's
// now-final code_off (`CodeBuf::patch_call_rel32()`, new -- unlike
// `patch_rel32()` above, which always patches against the CURRENT end of
// the buffer, this one patches against a CALLER-SUPPLIED target offset
// recorded earlier, since a forward call's own real target is emitted
// later in the buffer than the patch-list bookkeeping itself, not at the
// moment patching happens).
//
// This is genuinely a "grow codegen, not grammar" milestone: `program`,
// `function`, `factor`'s call production, `args` are all completely
// unchanged from Milestone 72; zero new AST node kinds, zero new lexer/
// parser productions. One new CodeGenError variant
// (`TooManyForwardCalls`, the pending-list's own real, disclosed,
// deliberately small cap being exceeded) and two new CodeBuf encodings
// (`emit_call_placeholder()`, `patch_call_rel32()`) are the entire real
// surface-area growth.
//
// Still genuinely open after this milestone (see its own closing
// disclosure further below for the complete list): up to MAX_FUNCS (4)
// functions and MAX_PARAMS (4) parameters/arguments, both unraised;
// deep-recursion stack-depth safety, still real, unmeasured, and
// undisclosed-as-safe (Milestone 73's own open item, completely
// untouched by this milestone -- forward/mutual calls carry the exact
// same single-4KiB-stack-page risk direct self-recursion already does,
// no better and no worse); no unary minus, no `&&`/`||`, no arrays/
// pointers, no additional C types (all unchanged scope cuts from prior
// milestones).
//
// MILESTONE 75 grows NEITHER grammar nor codegen in this file -- it
// closes the other real, explicitly disclosed UNKNOWN Milestone 73's
// own closing disclosure named and Milestone 74's own closing
// disclosure re-confirmed still open, twice in a row: "this kernel
// gives each process exactly one 4 KiB stack page, and neither this
// codegen nor the kernel itself enforces any stack-depth guard on a
// self-recursive/forward/mutually-recursive call chain... a real,
// unmeasured, undisclosed-as-safe risk". Weighed directly against the
// other real dependency-ordered candidates (unary minus/`&&`/`||`;
// raising MAX_FUNCS/MAX_PARAMS) before picking this: growing the
// grammar again a THIRD time in a row without ever closing the one
// significant OPEN SAFETY item on the books would be defaulting to the
// easy path, not the most valuable one -- and unlike a pure grammar
// slice, this item genuinely needed kernel-side work
// (`kernel/src/interrupts.rs`'s `page_fault_handler`), breaking the
// Milestone 74/73/72-and-earlier-since-67 streak of pure userspace
// toolchain changes on purpose.
//
// What this milestone actually found, checked directly against
// `kernel/src/interrupts.rs`/`kernel/src/process.rs` before writing
// any new code (not assumed from this file's own prior wording): a
// ring-3 stack overflow in this kernel was ALREADY safe, as a real,
// previously-un-exercised BYPRODUCT of the virtual address layout, not
// because any code path ever checked for it on purpose.
// `USER_STACK_ADDR` (0x_5555_6000_0000) sits a real, computed
// ~256 MiB above `USER_CODE_ADDR`'s own single mapped page
// (0x_5555_5000_0000), and nothing in this kernel -- not the heap
// (`HEAP_START`, ABOVE the stack), not mmap -- ever maps so much as
// one byte of that gap. A stack push that runs off the bottom of the
// single real mapped stack page therefore produces an ordinary
// NOT-PRESENT `#PF` against unmapped memory, which already fell
// through this kernel's own real heap/mmap demand-paging attempts
// (Milestone 57/64) straight into the SAME unconditional
// SIGSEGV-and-terminate path (Milestone 41) every other invalid ring-3
// access already used -- `WaitOutcome::Signaled` (Milestone 53) and
// all. This milestone's real kernel-side change
// (`STACK_GUARD_REGION_SIZE` in `interrupts.rs`) does not alter that
// control flow AT ALL -- it only recognizes a NOT-PRESENT fault inside
// a real, conservative 1 MiB band immediately below `USER_STACK_ADDR`
// and logs it as a distinct "STACK OVERFLOW" line instead of the
// generic SIGSEGV wording, so this specific, previously-unmeasured
// failure mode is diagnosable at a glance rather than indistinguishable
// from any other wild-pointer fault. It deliberately does NOT add a
// software recursion-depth counter: this kernel has no visibility into
// arbitrary ring-3 `call`/`ret` depth without per-call instrumentation
// this codegen has never emitted, and a hard depth counter would cap
// the SAME real thing the guard-page mechanism already catches, just
// less precisely (a shallow-but-huge-stack-frame function could still
// overflow well under any depth counter's own threshold; the address-
// based guard catches the real resource exhaustion directly, the same
// way a page fault always has for ordinary memory).
//
// Real, end-to-end verified for the first time by a new CASE 34 below:
// a real subset-C program with genuine, UNCONDITIONAL, unbounded
// self-recursion (`int spin(int n) { return spin(n + 1); }`, no base
// case at all -- it can only ever terminate one of two ways: run off
// the single stack page, or hang forever, and this milestone's own
// kernel-side change plus the pre-existing guard-gap mechanism above
// guarantee the former), run through the real on-disk-ELF + kernel
// `exec()` + `wait()` path Milestone 69 established -- the strongest
// verification tier, same as every milestone since. Deliberately NOT
// run via the in-process Callable path (`compile_and_run_program_
// callable`, CASE 5/6/11-16/30/32's own path): that executes the
// freshly-compiled code on cc.elf's OWN current stack, in cc.elf's OWN
// process -- an unbounded overflow there would take down THIS
// self-test harness itself before `OVERALL_M75` could ever print, not
// just the intended child. The real, on-disk-ELF child is expected to
// be reaped with `WaitOutcome::Signaled` (bit 17 of the real `wait()`
// encoding usertest.rs's own syscall dispatch documents), NOT
// `WaitOutcome::Exited` -- a program with no base case at all reaching
// its own `exit()` would itself be a serious, distinguishing bug
// (either a corrupted return address landing somewhere that happens to
// call `sys_exit`, or a mis-generated `spin` that silently stopped
// recursing), not a plausible success path.
//
// Still genuinely open after this milestone: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each, unraised); no unary minus, no `&&`/`||`, no arrays/
// pointers, no additional C types (all unchanged scope cuts from prior
// milestones); this kernel still gives every process exactly ONE 4 KiB
// stack page -- that has NOT changed, and never will just from a
// diagnostic-only kernel change -- what changed is that running off
// the bottom of it is now verified, end-to-end, on real hardware, to
// be a clean, disclosed, recoverable `SIGSEGV`/`Signaled` termination
// rather than an unmeasured risk of silent corruption, a triple fault,
// or a kernel panic.
//
// MILESTONE 76 grows the grammar itself -- unary minus and the two real
// short-circuit logical operators `&&`/`||` -- picked over the other real
// dependency-ordered candidate (raising MAX_FUNCS/MAX_PARAMS past 4) after
// checking both against this file's own actual current code, not just its
// prior wording: MAX_FUNCS/MAX_PARAMS=4 has not yet blocked a single real
// test program this toolchain has tried to express, so raising it now
// would be speculative headroom with no concrete program driving it,
// while unary minus and `&&`/`||` are grammar gaps every one of Milestones
// 73/74/75's own "still genuinely open" disclosures has independently
// named, unchanged, three milestones running -- the older and more
// consistently-deferred of the two real candidates. Also picked over
// arrays/pointers (real memory-addressing codegen beyond simple rbp-
// relative locals, a substantially bigger step needing its own honestly
// scoped slice) and additional C types (this subset's `int` is this
// kernel's own native 64-bit machine word -- see gen_expr()'s own
// "value in RAX" postcondition throughout -- so a second type would need
// real width-tracking through every AST node and codegen path, not a
// small addition either). Grammar addition:
//   unary       := "-" unary | factor
//   term        := unary (("*" | "/") unary)*
//   logic_and   := cond_expr ("&&" cond_expr)*
//   logic_or    := logic_and ("||" logic_and)*
// `factor` is completely UNCHANGED (INTLIT | IDENT | IDENT "(" args? ")" |
// "(" cond_expr ")"); `unary` slots in directly above it, so unary minus
// binds tighter than `*`/`/` and, transitively, tighter than binary
// `+`/`-` too -- checked against real C's own operator-precedence table
// before picking this exact insertion point. `cond_expr` (single,
// deliberately non-chaining relop, Milestone 70's own shape) is likewise
// completely UNCHANGED; `logic_and`/`logic_or` wrap it as two new, real
// top-level layers -- `&&` binds tighter than `||`, both bind LOOSER than
// any comparison, the same real C precedence ordering. `parse_logic_or()`
// (not the narrower `parse_cond_expr()` Milestone 70 originally wired
// them to) is now what every assign_stmt/return_stmt/if_stmt/while_stmt
// condition, call argument, and parenthesized sub-expression actually
// calls -- see `parse_logic_or()`'s own doc comment below for the
// complete real call-site list.
//
// The one real codegen subtlety this milestone did NOT shortcut: `&&`/
// `||` genuinely SHORT-CIRCUIT (gen_expr()'s own EXPR_BINARY arm below),
// reusing the exact same emit_jz_placeholder()/emit_jmp_placeholder()/
// patch_rel32() forward-patch machinery Milestone 70/71 already built
// for if/else and while, rather than the easy-but-wrong fake -- evaluate
// both operands unconditionally, then combine with a bitwise AND/OR.
// That shortcut would have been WRONG C semantics, not just an
// unoptimized correct answer: a right operand containing a call must
// genuinely not execute when the left operand already decides the
// result, checked directly against the C standard's own "&&/|| are
// sequence points" rule before writing this, not assumed. Unary minus
// needed one new real CodeBuf encoding (`emit_neg_rax()`, `neg r/m64`);
// `&&`/`||` needed zero new CodeBuf encodings, only this file's existing
// jz/jmp/patch_rel32 primitives and the `setne al`/`movzx eax, al`
// boolean-normalize idiom OP_NE already established, composed into two
// new real branching shapes -- see gen_expr()'s own inline comments for
// both, worked out and hand-verified against the Intel SDM the same way
// every prior milestone's own encodings were.
//
// Two new self-test cases verify this milestone for real: CASE 35 (the
// in-process Callable path, unary minus combined with `&&` and a
// comparison) and CASE 36 (the real on-disk-ELF + kernel exec()+wait()
// path -- this milestone's own strongest verification tier, same
// precedent as every milestone since 69 -- `||`, unary minus as a call
// argument, and function calls together). See both cases' own inline
// comments below for the exact hand-computed expected results.
//
// Still genuinely open after this milestone: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each, unraised, real candidate for the next slice); no arrays/
// pointers, no additional C types (both unchanged scope cuts, real
// bigger steps, not attempted here); no bitwise operators (`&`, `|`,
// `^`, `~`, `<<`, `>>` -- a real, newly-disclosed gap this milestone's
// own lexer change makes slightly more visible, since `&`/`|` are now
// lexed at all, just only as the FIRST half of `&&`/`||` -- a lone `&`
// or `|` is still a real, deliberate UnknownChar); no logical NOT (`!`
// alone is still only legal as the first half of `!=`, unchanged since
// Milestone 70 -- `!x` is not expressible, though `x == 0` already
// covers the same real ground for this subset's own boolean-as-int
// convention); this kernel still gives every process exactly ONE 4 KiB
// stack page, completely untouched by this milestone (a pure userspace
// toolchain change, unlike Milestone 75's own kernel-side diagnostic).
//
// MILESTONE 79 closes the bitwise-operator gap Milestone 76 named above --
// picked over the other two real candidates (MAX_FUNCS/MAX_PARAMS, still
// unblocked by any concrete test program; arrays/pointers, a substantially
// bigger memory-addressing step) as the one most consistently and
// specifically flagged across the prior three milestones' own disclosures.
// Grammar addition (real C's own full bitwise precedence table, checked
// against it before writing this, not guessed):
//   unary     := ("-" | "~") unary | factor
//   bit_and   := cond_expr ("&" cond_expr)*
//   bit_xor   := bit_and ("^" bit_and)*
//   bit_or    := bit_xor ("|" bit_xor)*
//   logic_and := bit_or ("&&" bit_or)*
//   shift_expr:= expr (("<<" | ">>") expr)*
//   cond_expr := shift_expr (relop shift_expr)?
// `expr` (additive) and `term` (`*`/`/`) are completely UNCHANGED;
// `unary` gains its second operator in the exact slot Milestone 76's own
// OP_NEG comment already anticipated ("a second unary operator could be
// added here"). `cond_expr` now calls `shift_expr` instead of `expr`
// directly (`<<`/`>>` sit between additive and relational, real C's own
// ordering); `bit_and`/`bit_xor`/`bit_or` are three new layers wrapping
// the pre-existing `cond_expr`, and `logic_and` now calls `bit_or`
// instead of `cond_expr` directly -- the same "wrap the existing top
// layer with one more" widening Milestone 76's own `parse_logic_and()`/
// `parse_logic_or()` already established as precedent, applied four more
// times. `&`/`^`/`|` deliberately bind LOOSER than every comparison
// (`a & b == c` parses as `a & (b == c)`) and `<<`/`>>` deliberately bind
// LOOSER than `+`/`-` but tighter than any comparison (`1 << 2 + 1`
// parses as `1 << (2 + 1)`) -- both real, classic C precedence traps,
// gotten right on purpose and verified by a real discriminating test
// case (CASE 38 below), not merely asserted in this comment.
//
// Codegen: AND/OR/XOR reuse the exact left-in-RAX/right-in-RCX stack-
// machine convention every arithmetic/comparison operator already uses
// (three new one-instruction CodeBuf encodings, same ALU-opcode family
// as ADD/SUB/CMP). SHL/SAR need their shift COUNT in CL specifically (a
// real x86 ABI requirement for shift-by-register) -- which the existing
// convention already leaves sitting in RCX's own low byte, a genuinely
// free fit, not engineered to look that way. `~` reuses unary minus's
// exact "evaluate operand, transform in place, no branch machinery"
// shape with one new real encoding (`emit_not_rax()`, same F7
// opcode-extension-digit family `emit_neg_rax()` already uses). See
// gen_expr()'s own inline comments and each new CodeBuf method's own
// doc comment for the full derivations.
//
// Two new self-test cases verify this milestone for real: CASE 37 (the
// in-process Callable path -- all five binary operators plus unary `~`,
// combined over real variables) and CASE 38 (the real on-disk-ELF +
// kernel exec()+wait() path -- this milestone's own strongest
// verification tier, same precedent as every milestone since 69 -- a
// REAL precedence-regression test, deliberately built so a wrong
// precedence insertion would produce a different, distinguishable
// numeric result, not one that coincidentally still passes). See both
// cases' own inline comments below for the exact hand-computed expected
// results.
//
// Still genuinely open after this milestone: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each, still unraised, still unblocked by any concrete test
// program); no arrays/pointers, no additional C types (unchanged real
// scope cuts); no logical NOT (`!` alone still only legal as the first
// half of `!=`, unchanged since Milestone 70 -- `x == 0` still covers
// the same real ground, and now `~x` covers a genuinely different one,
// bitwise rather than logical complement); compound assignment operators
// (`&=`, `|=`, `^=`, `<<=`, `>>=`, and their arithmetic siblings `+=` etc.)
// are a real, newly-visible gap this milestone's own operator set makes
// more apparent, not attempted here; SAR vs. SHR is genuinely
// UNDISTINGUISHED by either of this milestone's own two test cases (both
// only ever shift a non-negative value, where the two encodings produce
// identical results) -- a real, disclosed verification gap, not hidden;
// this kernel still gives every process exactly ONE 4 KiB stack page,
// completely untouched by this milestone.
//
// MILESTONE 83 closes the logical-NOT gap Milestone 76's and Milestone
// 79's own "still genuinely open" disclosures both named ("no logical
// NOT (`!` alone) ... `!x` is not expressible") -- the last remaining
// unary-operator gap, picked over the other two standing candidates
// (MAX_FUNCS/MAX_PARAMS, still unblocked by any concrete test program;
// compound assignment `+=`/`&=`/... , a wider slice touching every
// assign_stmt) as the smallest cleanly-dependency-ordered increment
// that also completes a set: unary `-` (M76), `~` (M79), and now `!`
// are the whole real unary operator surface for this subset.
// Grammar addition:
//   unary := ("-" | "~" | "!") unary | factor
// `!` slots into the exact same `unary` production `-` and `~` already
// share, so it binds at the same (tightest) precedence -- `!a * b` is
// `(!a) * b`, `!a < b` is `(!a) < b` -- and composes with the other two
// and itself through the same recursive-unary call (`!!x`, `!-x`,
// `~!x`). `cond_expr`/`shift_expr`/the bit layers/`logic_and`/
// `logic_or` are all COMPLETELY UNCHANGED. The lexer gains one token
// (`TOK_BANG`) for a lone `!`; `!=` was and stays a two-char token
// checked first, so `a != b` is untouched.
//
// Codegen: `!` is a real BOOLEAN-producing operator (result is exactly
// 0 or 1), so unlike `-`/`~` (which transform RAX in place via the F7
// opcode family) it reuses the exact `test rax, rax` + `setcc` + `movzx
// eax, al` normalize idiom OP_NE and every comparison arm already use --
// here with `sete` (AL := 1 iff the operand was 0). ZERO new CodeBuf
// encodings -- `emit_test_rax_rax()`, `emit_setcc()`, `emit_movzx_eax_al()`
// all already exist (Milestone 70/76). One new EXPR_UNARY op sentinel
// (`OP_LOGNOT`), the exact extension point OP_NEG's/OP_BITNOT's own
// comments named.
//
// Two new self-test cases verify this for real: CASE 39 (the in-process
// Callable path -- `!` on zero and nonzero operands, doubled `!!`, on a
// comparison result, and as `&&`'s left operand, over real variables)
// and CASE 40 (the real on-disk-ELF + kernel exec()+wait() path -- this
// milestone's strongest tier, a REAL precedence-regression test built so
// a wrong `!`-vs-`*` binding gives a different, distinguishable result).
//
// Still genuinely open after this milestone: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each, still unraised, still unblocked by any concrete program);
// compound assignment operators (`+=`, `-=`, `*=`, `/=`, `&=`, `|=`,
// `^=`, `<<=`, `>>=` -- now the oldest-standing named grammar gap, a
// real candidate for the next slice); no arrays/pointers, no additional
// C types (unchanged real scope cuts); SAR vs. SHR still UNDISTINGUISHED
// by any test case (Milestone 79's own open verification gap, untouched
// here); this kernel still gives every process exactly ONE 4 KiB stack
// page, completely untouched by this milestone (a pure userspace
// toolchain change).
//
// MILESTONE 84 closes the compound-assignment gap Milestone 83's own
// closing disclosure named as "the oldest-standing named grammar gap"
// -- the nine operators `+=` `-=` `*=` `/=` `&=` `|=` `^=` `<<=` `>>=`.
// Picked over the other two standing candidates (MAX_FUNCS/MAX_PARAMS,
// still unblocked by any concrete program; arrays/pointers, a
// substantially bigger memory-addressing step). Grammar change, a
// single production:
//   assign_stmt := IDENT ("=" | "+=" | "-=" | "*=" | "/=" | "&=" | "|="
//                          | "^=" | "<<=" | ">>=") logic_or ";"
// `x OP= e` is DESUGARED at parse time to `x = (x OP e)` -- the parser
// synthesizes an ordinary EXPR_BINARY node (matching op) over a fresh
// EXPR_IDENT reference to `x` and the parsed RHS, and hands that to the
// existing STMT_ASSIGN path. So this milestone adds NINE lexer tokens
// and ONE parser branch and ZERO new codegen: `gen_expr()`/
// `STMT_ASSIGN` already generate correct code for every AST shape the
// desugaring produces. Evaluating `x` twice is exact here because `x`
// is always a plain IDENT in this subset (no index/deref/call as an
// lvalue), so real C's "the lvalue is evaluated exactly once" guarantee
// is satisfied for free rather than needing a temp. RHS is `logic_or`,
// this subset's lowest-precedence expression, so `x += a * b + c` is
// `x = x + ((a*b)+c)` and `x <<= 1 + 1` is `x = x << (1+1)` -- checked
// against real C's precedence table, and CASE 42 below is a real
// precedence-regression test built so a wrong binding gives a
// different, distinguishable number.
//
// Lexing follows the same longest-match-first order every other
// multi-char operator already uses: `<<=`/`>>=` (3 chars, a new third
// byte of lookahead) are matched before `<<`/`>>` (M79), which are
// matched before `<=`/`>=` (M70) and `<`/`>`; the seven 2-char
// operators are matched before their single-char forms; `&=`/`|=` fall
// through M76's `&&`/`||` checks unharmed (those test the second byte
// for `&`/`|`, not `=`). `/=` is safe -- this subset has no `//`
// comment syntax.
//
// Two new self-test cases: CASE 41 (in-process Callable path -- all
// nine operators applied in sequence to one variable, each result
// hand-computed) and CASE 42 (real on-disk-ELF + kernel exec()+wait()
// path, this milestone's strongest tier -- the precedence-regression
// check).
//
// Still genuinely open after this milestone: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); no arrays/pointers, no additional C types; no `++`/`--`
// (this subset has never had them and nothing here adds them -- `x +=
// 1` now covers the same ground); SAR vs. SHR still undistinguished by
// any test case (Milestone 79's open verification gap, untouched); one
// 4 KiB stack page per process, untouched (a pure userspace toolchain
// change).
//
// MILESTONE 85 adds NO grammar and NO codegen -- it closes the
// SAR-vs-SHR verification hole Milestone 79's own closing note first
// disclosed and every milestone since (80, 81, 83, 84) re-confirmed
// open: "SAR vs. SHR is genuinely UNDISTINGUISHED by any test case
// (both only ever shift a non-negative value, where the two encodings
// produce identical results)". Weighed against growing the grammar a
// third time running (`for`, `break`/`continue`) -- Milestone 75's own
// reasoning applies: with a specifically-named, on-the-books
// verification hole in the compiler's OWN output, closing it is the
// more valuable move than another operator. `gen_expr()`'s `OP_SHR`
// arm has always emitted `emit_sar_rax_cl()` (real SAR, the
// arithmetic sign-preserving shift), so a latent bug that emitted SHR
// instead would have passed every prior test. Two new cases shift a
// genuinely NEGATIVE value and then read its sign, so SAR (stays
// negative) and SHR (becomes a large positive) give different,
// distinguishable answers: CASE 43 (`-8 >> 1` == -4, in-process
// Callable path, returns `0 - a` == 4) and CASE 44 (`-16 >> 2` == -4,
// real on-disk-ELF + kernel exec()+wait() path, routed through an
// `if (x < 0)` so the sign-bit difference reaches the exit code --
// exits 7 under SAR, would exit 9 under SHR). `<<` needs no arithmetic
// variant (SHL is bit-identical signed/unsigned) and is not separately
// re-verified.
//
// Still genuinely open after Milestone 85: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); `for` loops and `break`/`continue` (the leading grammar
// candidates now, `break`/`continue` open since Milestone 71); no
// arrays/pointers, no additional C types; one 4 KiB stack page per
// process.
//
// MILESTONE 86 adds `for` loops, the cleaner of Milestone 85's two
// named grammar candidates:
//   for_stmt := "for" "(" for_clause? ";" logic_or? ";" for_clause? ")"
//               "{" stmt* "}"
//   for_clause := IDENT ("=" | "+=" | ... | ">>=") logic_or
// `for (init; cond; step) { body }` is DESUGARED at parse time to
// `init; while (cond) { body step; }` -- Milestone 71's `STMT_WHILE`
// (and therefore all of its codegen) is reused verbatim, so this is a
// pure parser milestone: ZERO new codegen, one new keyword token, one
// new `parse_stmt` arm. `init` and `step` are ordinary IDENT-assignments
// (plain `=` or any Milestone-84 compound form), parsed by the SAME
// `finish_ident_assign()` the plain assignment statement uses (extracted
// into its own method this milestone for exactly that reuse), with
// `consume_semi = false`. `init`, `cond`, and `step` are each optional;
// an absent `cond` becomes a synthesized `EXPR_INTLIT(1)`. The loop
// variable must be declared before the loop -- this subset has no
// combined `int i = 0` decl-init anywhere, an unchanged scope cut.
//
// The one real structural subtlety: `parse_stmt()` can now return a
// two-node CHAIN (`init -> while`), not always a single node, so
// `parse_stmt_list_until_rbrace()` now advances its `tail` cursor to the
// real end of whatever `parse_stmt()` returned before linking the next
// statement -- a small, general fix, not `for`-specific.
//
// Two new self-test cases: CASE 45 (in-process Callable path -- a
// counting loop, `for (i=1; i<5; i+=1) s += i` -> 10) and CASE 46 (real
// on-disk-ELF + kernel exec()+wait() path -- factorial via `for`, built
// so a wrong desugar order gives 720 or 24 instead of 120).
//
// Still genuinely open after Milestone 86: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); `break`/`continue` (open since Milestone 71, now the leading
// grammar candidate -- needs a loop-context stack so a `break` forward-
// patches to the loop exit and a `continue` to the step); no
// combined decl-init; no arrays/pointers, no additional C types; one
// 4 KiB stack page per process.
//
// MILESTONE 87 adds `break` and `continue`, open since Milestone 71's
// own disclosure ("the only way out of a while-loop is its own
// condition going false, or an early return"). Grammar: two new leaf
// statements, `break ;` and `continue ;` -- all the real work is
// codegen-side. `gen_stmt_list()` gains two value parameters: `loop_top`
// (the innermost enclosing loop's condition-re-check offset, or
// `NOT_IN_LOOP` outside any loop) and `loop_is_for` (1 if that loop is a
// `for`). Both are threaded UNCHANGED through `STMT_IF`'s branch
// recursion (a `break` inside an `if` inside a `while` binds to that
// `while`) and replaced by `STMT_WHILE` for its own body (so nesting is
// a natural stack -- break/continue always bind innermost). The
// break/continue jump-placeholder lists themselves are module `static
// mut` arenas (`LOOP_BRK`/`LOOP_CONT`) with a save/restore-of-count
// discipline per loop -- NOT per-loop stack arrays behind an escaped
// pointer, which the optimizer read stale through the recursion,
// producing a wild `jmp` that null-faulted cc.elf (a real bug caught by
// a boot during this milestone).
//   - `break` is an unconditional forward `jmp` recorded as a
//     placeholder in the arena; `STMT_WHILE` resolves every entry from
//     this loop's base index to the loop exit once that address is
//     known, then truncates the arena back -- the
//     exact same emit-placeholder-then-`patch_rel32()` machinery
//     if/else/`&&`/`||` already use.
//   - `continue` for a plain `while` is a backward `jmp` straight to
//     the condition re-check (`emit_jmp_back(loop_top)`, target already
//     known). For a `for` loop the next iteration must run the `step`
//     clause first (real C semantics), so `continue` is instead a
//     forward `jmp` into a second per-loop patch list, resolved to the
//     point right before the step's own codegen. This is why Milestone
//     86's `for` desugar now carries the step in the `STMT_WHILE`'s
//     `else_body` slot (unused for a plain `while`) rather than
//     appending it to the body -- behaviourally identical without
//     break/continue, but keeping the step distinct is what lets
//     `continue` target it.
//   - `break`/`continue` outside any loop are real semantic errors
//     (`CodeGenError::BreakOutsideLoop`/`ContinueOutsideLoop`),
//     exercised by CASE 49.
// Three new self-test cases: CASE 47 (in-process -- both, in a `while`,
// each from inside a nested `if` -> 52), CASE 48 (real on-disk-ELF +
// kernel exec()+wait() -- both in a `for`, built so `continue` skipping
// the step would HANG the program instead of exiting 8), CASE 49 (the
// BreakOutsideLoop error path).
//
// Still genuinely open after Milestone 87: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); no combined `int i = 0` decl-init; no arrays/pointers, no
// additional C types; no `switch`/`goto`; one 4 KiB stack page per
// process.
//
// Every byte access below goes through raw core::ptr::read/write on
// u64 addresses rather than `[]` slice/array indexing wherever the
// index is runtime-variable -- proactively following the SAME
// discipline libc.rs's own fwrite()/fread()/fprintf() doc comments
// already establish and explain: `[]` indexing on a runtime-variable
// index the compiler can't statically bound inserts a real bounds-
// check panic path, which pulls `core::fmt` (for the panic message's
// own `Display` formatting) into this freestanding, `panic=abort`
// binary -- a real, already-hit link failure at this kernel's unusually
// high USER_CODE_ADDR link address (`R_X86_64_GOTPCREL out of range`,
// documented in tools/stdiotest_src/libc.rs's own fwrite() doc
// comment). Written this way from the start here rather than re-
// discovering the same failure independently.
#![no_std]
#![no_main]

mod libc;
use libc::*;

// =======================================================================
// Small helpers -- write to stdout via the real write() syscall, and
// print an unsigned decimal number without ever touching core::fmt
// (same raw-pointer-digit-buffer technique as libc.rs's own write_udec()
// helper for its fprintf-equivalent, copied here rather than exposed
// from libc.rs since it's a test-harness-only concern, not a real libc
// surface).
// =======================================================================

unsafe fn w(bytes: &[u8]) {
    unsafe { sys_write(bytes.as_ptr() as u64, bytes.len() as u64) };
}

unsafe fn write_check(label: &[u8], ok: bool) {
    unsafe { w(label) };
    unsafe { w(if ok { b"PASS" } else { b"FAIL" }) };
    unsafe { w(b" ") };
}

unsafe fn write_u64_dec(mut val: u64) {
    if val == 0 {
        unsafe { w(b"0") };
        return;
    }
    let mut digits = [0u8; 20];
    let digits_ptr = digits.as_mut_ptr() as u64;
    let mut n: u64 = 0;
    while val > 0 {
        let digit = b'0' + (val % 10) as u8;
        unsafe { core::ptr::write((digits_ptr + n) as *mut u8, digit) };
        val /= 10;
        n += 1;
    }
    let mut i = n;
    while i > 0 {
        i -= 1;
        let digit = unsafe { core::ptr::read((digits_ptr + i) as *const u8) };
        let one = [digit];
        unsafe { w(&one) };
    }
}

/// Real, raw-pointer byte-for-byte comparison of the `len` bytes at
/// `src_ptr + off` against the literal `s` -- used both by the lexer
/// (keyword recognition) and by the self-test below (checking a parsed
/// identifier's source bytes against an expected name), matching
/// strcmp()'s own "compare via pointer reads, never slice" convention
/// already established in libc.rs.
unsafe fn word_is(src_ptr: u64, off: u64, len: u64, s: &[u8]) -> bool {
    if len != s.len() as u64 {
        return false;
    }
    let sp = s.as_ptr() as u64;
    let mut i: u64 = 0;
    while i < len {
        let a = unsafe { core::ptr::read((src_ptr + off + i) as *const u8) };
        let b = unsafe { core::ptr::read((sp + i) as *const u8) };
        if a != b {
            return false;
        }
        i += 1;
    }
    true
}

// =======================================================================
// Lexer -- tokens live in a fixed-size, caller-provided buffer accessed
// entirely via raw pointer arithmetic (tok_read/tok_write below), never
// `[]` indexing, for the reason given in this file's own top doc
// comment.
// =======================================================================

const TOK_EOF: u8 = 0;
const TOK_INT: u8 = 1; // keyword "int"
const TOK_RETURN: u8 = 2; // keyword "return"
const TOK_IDENT: u8 = 3;
const TOK_INTLIT: u8 = 4;
const TOK_LPAREN: u8 = 5;
const TOK_RPAREN: u8 = 6;
const TOK_LBRACE: u8 = 7;
const TOK_RBRACE: u8 = 8;
const TOK_SEMI: u8 = 9;
const TOK_ASSIGN: u8 = 10;
const TOK_PLUS: u8 = 11;
const TOK_MINUS: u8 = 12;
const TOK_STAR: u8 = 13;
const TOK_SLASH: u8 = 14;
// MILESTONE 70: comparison operators (two-char lookahead for ==, !=,
// <=, >=) and the "if"/"else" keywords.
const TOK_EQ: u8 = 15; // "=="
const TOK_NE: u8 = 16; // "!="
const TOK_LT: u8 = 17; // "<"
const TOK_LE: u8 = 18; // "<="
const TOK_GT: u8 = 19; // ">"
const TOK_GE: u8 = 20; // ">="
const TOK_IF: u8 = 21; // keyword "if"
const TOK_ELSE: u8 = 22; // keyword "else"
const TOK_WHILE: u8 = 23; // MILESTONE 71: keyword "while"
const TOK_COMMA: u8 = 24; // MILESTONE 72: argument/parameter-list separator
const TOK_ANDAND: u8 = 25; // MILESTONE 76: "&&"
const TOK_OROR: u8 = 26; // MILESTONE 76: "||"
// MILESTONE 79: real bitwise operators -- five binary (&, |, ^, <<, >>)
// plus one unary (~). "&"/"|" were lexable as UnknownChar-only tokens
// through Milestone 76 (see that milestone's own lex() comment: "a
// LONE '&' or '|' has no meaning in this subset ... a real, deliberate
// UnknownChar"); that's the gap this milestone closes, for real, not
// just for these two symbols but the complete real C bitwise set.
const TOK_AMP: u8 = 27; // "&"
const TOK_PIPE: u8 = 28; // "|"
const TOK_CARET: u8 = 29; // "^"
const TOK_TILDE: u8 = 30; // "~" (unary only in this subset -- same "no bare use beyond its one real grammar slot" scope as every other operator token here)
const TOK_SHL: u8 = 31; // "<<"
const TOK_SHR: u8 = 32; // ">>"
// MILESTONE 83: real logical NOT. "!" was lexable ONLY as the first half
// of "!=" through Milestone 82 (see lex()'s own comment: "'!' has no
// standalone token ... only '!=' is recognized"); a lone '!' was a
// deliberate UnknownChar. This is the last unary-operator gap every
// "still genuinely open" disclosure since Milestone 70 has named, now
// closed: unary "-" (M76), "~" (M79), and "!" (this milestone) are the
// complete real unary set for this subset.
const TOK_BANG: u8 = 33; // "!"
// MILESTONE 84: the nine compound-assignment operators. `x OP= e` is
// parsed as `x = x OP e` (a real desugaring in parse_stmt, not new
// codegen -- `x` is always a plain IDENT in this subset, so evaluating
// it twice is identical to once). Nine new lexer tokens, one parser
// branch, ZERO new CodeBuf encodings. Lexed with the same
// longest-match-first discipline every other multi-char operator uses:
// "<<="/">>=" (3 chars) are checked before "<<"/">>" (M79) which are
// checked before "<="/">=" (M70) and "<"/">"; "+="/"-="/... (2 chars)
// before the single-char "+"/"-"/... ; "&="/"|=" are distinct from
// M76's "&&"/"||" by their second byte.
const TOK_PLUSEQ: u8 = 34;  // "+="
const TOK_MINUSEQ: u8 = 35; // "-="
const TOK_STAREQ: u8 = 36;  // "*="
const TOK_SLASHEQ: u8 = 37; // "/="
const TOK_AMPEQ: u8 = 38;   // "&="
const TOK_PIPEEQ: u8 = 39;  // "|="
const TOK_CARETEQ: u8 = 40; // "^="
const TOK_SHLEQ: u8 = 41;   // "<<="
const TOK_SHREQ: u8 = 42;   // ">>="
// MILESTONE 86: keyword "for". `for (init; cond; step) { body }` is
// desugared at parse time to `init; while (cond) { body step; }` --
// reuses Milestone 71's while codegen wholesale, ZERO new codegen. `init`
// and `step` are ordinary assignment statements (plain `=` or any M84
// compound form), each optional; `cond` is optional (absent == an
// always-true `1`). The loop variable must be declared before the loop
// (`int i = 0` combined decl-init is not in this subset's grammar
// anywhere -- a real, disclosed scope cut, not new here).
const TOK_FOR: u8 = 43; // keyword "for"
// MILESTONE 87: `break` and `continue`, open since Milestone 71's own
// disclosure ("the only way out of a while-loop is its own condition
// going false, or an early return"). `break` jumps forward past the
// loop; `continue` jumps to the next iteration -- to the loop-top
// condition for a plain `while`, and to the `step` clause for a `for`
// (real C semantics, which is why Milestone 86's `for` desugar now
// carries the step in `else_body` instead of appending it to the body).
const TOK_BREAK: u8 = 44;    // keyword "break"
const TOK_CONTINUE: u8 = 45; // keyword "continue"

#[derive(Clone, Copy)]
#[repr(C)]
struct Token {
    kind: u8,
    int_val: u64,
    ident_off: u32,
    ident_len: u8,
}

/// MILESTONE 70 FIX: raised 64 -> 128 -- a real bug this milestone's own
/// verification found (not the ELF-segment-page cap issue, a separate,
/// genuine token-buffer-capacity bug): CASE 11 below (all six comparison
/// operators combined into one hand-verifiable decimal result) lexes to
/// 67 real tokens + 1 EOF = 68, already past the old 64-token cap --
/// confirmed the hard way via a real boot where CASE 11 printed
/// `(compile failed)` (lex_and_parse() -> lex() -> Err(LexError::
/// TooManyTokens) -> None), not a hypothetical count. `toks_ptr` is a
/// transient malloc()ed scratch buffer, freed implicitly at process exit
/// -- unlike MAX_PAGES_PER_ELF_SEGMENT, raising this has ZERO effect on
/// cc.elf's own compiled/linked SIZE (no token ever gets emitted into
/// the ELF image itself), so it doesn't reopen that separate concern.
/// 128 is real, deliberate headroom (roughly 2x CASE 11's own 68) for
/// this and future milestones' own combined-source test cases, not
/// picked to exactly fit today's one known failure.
const MAX_TOKENS: usize = 128;

unsafe fn tok_write(toks_ptr: u64, idx: u64, t: Token) {
    unsafe { core::ptr::write((toks_ptr + idx * core::mem::size_of::<Token>() as u64) as *mut Token, t) };
}

unsafe fn tok_read(toks_ptr: u64, idx: u64) -> Token {
    unsafe { core::ptr::read((toks_ptr + idx * core::mem::size_of::<Token>() as u64) as *const Token) }
}

enum LexError {
    UnknownChar(u64),
    TooManyTokens,
}

/// Real lexer: scans `src_len` bytes at `src_ptr`, writing tokens
/// (including a final TOK_EOF) into the caller's `toks_ptr` buffer
/// (capacity `max_toks`). Returns the real token count (EOF included)
/// on success, or a real LexError -- TooManyTokens if the fixed buffer
/// would overflow, UnknownChar(offset) at the first byte that isn't
/// whitespace, an identifier/keyword start, a digit, or one of this
/// subset's ten recognized single-character symbols.
unsafe fn lex(src_ptr: u64, src_len: u64, toks_ptr: u64, max_toks: u64) -> Result<u64, LexError> {
    let mut i: u64 = 0;
    let mut n: u64 = 0;
    while i < src_len {
        let b = unsafe { core::ptr::read((src_ptr + i) as *const u8) };
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            i += 1;
            continue;
        }
        if n >= max_toks {
            return Err(LexError::TooManyTokens);
        }
        if b.is_ascii_alphabetic() || b == b'_' {
            let start = i;
            i += 1;
            loop {
                if i >= src_len {
                    break;
                }
                let c = unsafe { core::ptr::read((src_ptr + i) as *const u8) };
                if c.is_ascii_alphanumeric() || c == b'_' {
                    i += 1;
                } else {
                    break;
                }
            }
            let wlen = i - start;
            let kind = if unsafe { word_is(src_ptr, start, wlen, b"int") } {
                TOK_INT
            } else if unsafe { word_is(src_ptr, start, wlen, b"return") } {
                TOK_RETURN
            } else if unsafe { word_is(src_ptr, start, wlen, b"if") } {
                TOK_IF
            } else if unsafe { word_is(src_ptr, start, wlen, b"else") } {
                TOK_ELSE
            } else if unsafe { word_is(src_ptr, start, wlen, b"while") } {
                // MILESTONE 71
                TOK_WHILE
            } else if unsafe { word_is(src_ptr, start, wlen, b"for") } {
                // MILESTONE 86
                TOK_FOR
            } else if unsafe { word_is(src_ptr, start, wlen, b"break") } {
                // MILESTONE 87
                TOK_BREAK
            } else if unsafe { word_is(src_ptr, start, wlen, b"continue") } {
                // MILESTONE 87
                TOK_CONTINUE
            } else {
                TOK_IDENT
            };
            unsafe {
                tok_write(
                    toks_ptr,
                    n,
                    Token { kind, int_val: 0, ident_off: start as u32, ident_len: wlen as u8 },
                )
            };
            n += 1;
            continue;
        }
        if b.is_ascii_digit() {
            let mut val: u64 = 0;
            loop {
                if i >= src_len {
                    break;
                }
                let c = unsafe { core::ptr::read((src_ptr + i) as *const u8) };
                if c.is_ascii_digit() {
                    val = val * 10 + (c - b'0') as u64;
                    i += 1;
                } else {
                    break;
                }
            }
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_INTLIT, int_val: val, ident_off: 0, ident_len: 0 }) };
            n += 1;
            continue;
        }
        // MILESTONE 70: two-char lookahead for the four comparison
        // operators that share a first byte with a shorter token
        // ('=' alone is TOK_ASSIGN, "==" is TOK_EQ; '<'/'>' alone are
        // TOK_LT/TOK_GT, "<="/">=" are TOK_LE/TOK_GE; '!' alone is
        // TOK_BANG as of MILESTONE 83, "!=" is TOK_NE). Checked BEFORE
        // the single-char match below so the longer token always wins
        // (a lone '!' still falls through to the single-char match).
        let next = if i + 1 < src_len { Some(unsafe { core::ptr::read((src_ptr + i + 1) as *const u8) }) } else { None };
        // MILESTONE 84: a third byte of lookahead, only for the two
        // 3-char operators "<<=" and ">>=" -- checked further below,
        // before "<<"/">>", so a genuine "<<=" is never split.
        let next2 = if i + 2 < src_len { Some(unsafe { core::ptr::read((src_ptr + i + 2) as *const u8) }) } else { None };
        if b == b'=' && next == Some(b'=') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_EQ, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        if b == b'!' && next == Some(b'=') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_NE, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        if b == b'<' && next == Some(b'=') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_LE, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        if b == b'>' && next == Some(b'=') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_GE, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        // MILESTONE 76: "&&"/"||" -- same two-char-lookahead shape as the
        // four comparison operators above. A LONE '&' or '|' has no
        // meaning in this subset (no bitwise operators at all) and is a
        // real, deliberate UnknownChar, the same "only the longer token
        // is legal" treatment '!' alone already got from Milestone 70 --
        // checked BEFORE the single-char match below so it's never
        // reached for these two bytes at all.
        if b == b'&' && next == Some(b'&') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_ANDAND, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        if b == b'|' && next == Some(b'|') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_OROR, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        // MILESTONE 84: "<<="/">>=" -- the two 3-char operators, checked
        // BEFORE "<<"/">>" just below so a genuine "x <<= 1" lexes as
        // TOK_SHLEQ, not TOK_SHL followed by a stray TOK_ASSIGN. "<="/">="
        // above already fail for these (their `next` is '<'/'>' not '=').
        if b == b'<' && next == Some(b'<') && next2 == Some(b'=') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_SHLEQ, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 3;
            continue;
        }
        if b == b'>' && next == Some(b'>') && next2 == Some(b'=') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_SHREQ, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 3;
            continue;
        }
        // MILESTONE 79: "<<"/">>" -- same two-char-lookahead-before-
        // single-char shape as every other multi-char operator above.
        // Checked here (before the single-char match below, where '<'/
        // '>' alone already resolve to TOK_LT/TOK_GT) so "<<"/">>" are
        // never split into two single-char tokens.
        if b == b'<' && next == Some(b'<') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_SHL, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        if b == b'>' && next == Some(b'>') {
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_SHR, int_val: 0, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += 2;
            continue;
        }
        // MILESTONE 84: the seven 2-char compound-assignment operators
        // "+=" "-=" "*=" "/=" "&=" "|=" "^=" -- each checked before the
        // single-char match below so a lone '+'/'-'/... followed by '='
        // is one token, not two. "&="/"|=" fall through M76's "&&"/"||"
        // checks above (whose `next` is '&'/'|', not '='). "/=" is safe:
        // this subset has no "//" comment syntax, '/' is only ever
        // TOK_SLASH.
        {
            let ce = if b == b'+' { Some(TOK_PLUSEQ) }
                else if b == b'-' { Some(TOK_MINUSEQ) }
                else if b == b'*' { Some(TOK_STAREQ) }
                else if b == b'/' { Some(TOK_SLASHEQ) }
                else if b == b'&' { Some(TOK_AMPEQ) }
                else if b == b'|' { Some(TOK_PIPEEQ) }
                else if b == b'^' { Some(TOK_CARETEQ) }
                else { None };
            if let (Some(k), Some(b'=')) = (ce, next) {
                unsafe { tok_write(toks_ptr, n, Token { kind: k, int_val: 0, ident_off: 0, ident_len: 0 }) };
                n += 1;
                i += 2;
                continue;
            }
        }
        let kind = match b {
            b'(' => TOK_LPAREN,
            b')' => TOK_RPAREN,
            b'{' => TOK_LBRACE,
            b'}' => TOK_RBRACE,
            b';' => TOK_SEMI,
            b'=' => TOK_ASSIGN,
            b'+' => TOK_PLUS,
            b'-' => TOK_MINUS,
            b'*' => TOK_STAR,
            b'/' => TOK_SLASH,
            b'<' => TOK_LT,
            b'>' => TOK_GT,
            b',' => TOK_COMMA, // MILESTONE 72
            // MILESTONE 79: real single-char bitwise tokens -- unreached
            // for a genuine "&&"/"||"/"<<"/">>" (those `continue` above,
            // before this match ever runs), so a lone '&'/'|'/'<'/'>'
            // followed by anything else still correctly lands here.
            b'&' => TOK_AMP,
            b'|' => TOK_PIPE,
            b'^' => TOK_CARET,
            b'~' => TOK_TILDE,
            // MILESTONE 83: lone '!' -- unreached for a genuine "!="
            // (that `continue`s above, before this match runs), so a
            // '!' followed by anything other than '=' correctly lands
            // here as the real logical-NOT operator.
            b'!' => TOK_BANG,
            _ => return Err(LexError::UnknownChar(i)),
        };
        unsafe { tok_write(toks_ptr, n, Token { kind, int_val: 0, ident_off: 0, ident_len: 0 }) };
        n += 1;
        i += 1;
    }
    if n >= max_toks {
        return Err(LexError::TooManyTokens);
    }
    unsafe { tok_write(toks_ptr, n, Token { kind: TOK_EOF, int_val: 0, ident_off: 0, ident_len: 0 }) };
    n += 1;
    Ok(n)
}

// =======================================================================
// AST -- real, malloc()-allocated nodes linked by u64 pointers (0 =
// null, the same "page 0 never mapped" sentinel convention malloc()
// itself already established). Expressions form a real binary tree
// (ExprNode.left/right); a function body is a real singly-linked list
// of statements (StmtNode.next).
// =======================================================================

const EXPR_INTLIT: u8 = 1;
const EXPR_IDENT: u8 = 2;
const EXPR_BINARY: u8 = 3;
const EXPR_CALL: u8 = 4; // MILESTONE 72: a function-call expression, IDENT "(" args? ")"
const EXPR_UNARY: u8 = 5; // MILESTONE 76: a unary-minus expression, "-" unary -- only `left` is meaningful (the operand); `right` stays 0, the same "unused field stays 0" convention this AST already uses throughout

// MILESTONE 70: comparison-operator sentinels for ExprNode.op, deliberately
// NOT reusing raw ASCII bytes the way the four arithmetic ops do (b'+' etc)
// -- '<' (0x3C) and '>' (0x3E) are real ASCII printable bytes too and
// reusing them directly would work for LT/GT alone, but "==", "!=", "<=",
// ">=" have no natural single ASCII byte, so all six comparisons use the
// same small integer-sentinel scheme for uniformity, values chosen to be
// disjoint from the arithmetic ops' own ASCII range (0x2A/0x2B/0x2D/0x2F)
// on inspection, not by construction (they don't need to be, since gen_expr
// switches on a Rust enum-shaped match, not by ASCII means, but keeping
// them visibly disjoint avoids any confusion reading the match arms).
const OP_EQ: u8 = 1;
const OP_NE: u8 = 2;
const OP_LT: u8 = 3;
const OP_GT: u8 = 4;
const OP_LE: u8 = 5;
const OP_GE: u8 = 6;
// MILESTONE 76: one more unary sentinel (EXPR_UNARY's own `op`, only ever
// OP_NEG today -- kept as a named value rather than an implicit "any
// EXPR_UNARY is negation" assumption, so a second unary operator could be
// added later without renumbering anything) and two more binary sentinels
// for the real short-circuit logical operators, same disjoint-integer
// scheme as OP_EQ..OP_GE above, not raw ASCII bytes (`&&`/`||` have no
// natural single ASCII byte either).
const OP_NEG: u8 = 7;
const OP_LOGAND: u8 = 8;
const OP_LOGOR: u8 = 9;
// MILESTONE 79: real bitwise operators -- five new EXPR_BINARY op
// sentinels plus one new EXPR_UNARY op sentinel (OP_BITNOT, sibling to
// OP_NEG above -- same "op field distinguishes which unary operator"
// shape that comment already flagged as the real extension point when
// it was written).
const OP_BITOR: u8 = 10;
const OP_BITXOR: u8 = 11;
const OP_BITAND: u8 = 12;
const OP_SHL: u8 = 13;
const OP_SHR: u8 = 14;
const OP_BITNOT: u8 = 15;
// MILESTONE 83: one more EXPR_UNARY op sentinel -- logical NOT ("!"),
// third sibling to OP_NEG/OP_BITNOT, exactly the extension point those
// comments already named. `!x` is 1 when x == 0 and 0 otherwise; unlike
// OP_BITNOT it is a real boolean-producing operator (0/1 result), so
// its codegen reuses the same test/setcc/movzx normalize idiom OP_NE
// and the comparison operators already use, not the F7 in-place family.
const OP_LOGNOT: u8 = 16;

#[repr(C)]
struct ExprNode {
    kind: u8,
    int_val: u64,
    ident_off: u32, // variable name (EXPR_IDENT) OR callee name (EXPR_CALL, MILESTONE 72) -- same field-reuse discipline STMT_IF/STMT_WHILE's `expr`/`then_body` already established
    ident_len: u8,
    op: u8, // '+' | '-' | '*' | '/', only meaningful when kind == EXPR_BINARY
    left: u64,
    right: u64,
    // MILESTONE 72: only meaningful when kind == EXPR_CALL. `call_args_ptr`
    // is a malloc()ed array of `call_argc` u64 ExprNode pointers (one per
    // argument, in real source order) -- an EXTERNAL buffer accessed via
    // the same tok_write/tok_read-style raw-pointer read/write convention
    // this whole file already uses (see call_arg_write/call_arg_read
    // below), NOT an embedded `[u64; N]` array field, specifically so no
    // runtime-variable-index `[]` access into a fixed-size struct field is
    // ever needed -- this file's own top doc comment already explains why
    // that's a real, previously-hit link failure (a bounds-check panic
    // path pulls core::fmt into this freestanding binary), not a style
    // preference. 0 (null) when call_argc == 0, the same "0 = absent"
    // convention this whole AST already uses throughout.
    call_args_ptr: u64,
    call_argc: u8,
}

const STMT_DECL: u8 = 1;
const STMT_ASSIGN: u8 = 2;
const STMT_RETURN: u8 = 3;
const STMT_IF: u8 = 4; // MILESTONE 70
const STMT_WHILE: u8 = 5; // MILESTONE 71
const STMT_BREAK: u8 = 6; // MILESTONE 87
const STMT_CONTINUE: u8 = 7; // MILESTONE 87

#[repr(C)]
struct StmtNode {
    kind: u8,
    ident_off: u32, // target variable name, for DECL/ASSIGN
    ident_len: u8,
    expr: u64, // 0 for DECL (no initializer in this subset); the
    // condition ExprNode for STMT_IF/STMT_WHILE (reusing this same field
    // rather than adding a separate `cond` field -- both are just "the
    // one ExprNode this statement evaluates", the same real shape DECL
    // already left unused for exactly this kind of reuse)
    next: u64,
    // MILESTONE 70: the head of the then-branch's own StmtNode list, and
    // the head of the else-branch's (0 if no `else` was written -- the
    // same "0 = absent" convention this whole AST already uses
    // throughout). Both fields are unused (left 0) for DECL/ASSIGN/RETURN.
    // MILESTONE 71: STMT_WHILE reuses `then_body` for its own loop-body
    // StmtNode list (the same "the one body this statement runs" role
    // STMT_IF's then_body already plays) -- `else_body` stays 0/unused
    // for STMT_WHILE, there being no else-branch concept for a loop in
    // this subset, the same deliberate field-reuse-over-new-field
    // discipline `expr` above already established rather than growing
    // StmtNode with a fifth, while-only field for one bit of shape reuse.
    then_body: u64,
    else_body: u64,
}

// MILESTONE 72: one function parameter's name -- a real, malloc()ed
// external buffer element (param_write/param_read below), the exact same
// "external buffer, not an embedded fixed array" reasoning ExprNode's own
// call_args_ptr doc comment above already gives.
#[derive(Clone, Copy)]
#[repr(C)]
struct ParamInfo {
    name_off: u32,
    name_len: u8,
}

/// MILESTONE 72: the maximum number of parameters a single function may
/// declare, and (the same cap, reused) the maximum number of arguments a
/// single call expression may pass -- a real, deliberate, disclosed scope
/// cut, not an oversight: 4 covers this kernel's own real syscall ABI
/// convention (checked directly against libc.rs: every sys_* wrapper here
/// passes its arguments via rdi/rsi/rdx, i.e. up to 3, and this milestone's
/// own calling convention below extends that same real register-argument
/// idea one register further, to rdi/rsi/rdx/rcx) without needing any
/// stack-passed-argument codegen (the 5th-and-later-argument case real
/// SysV x86_64 handles via the stack) -- deferred future work, not
/// attempted here.
const MAX_PARAMS: u64 = 4;

/// MILESTONE 72: the maximum number of function definitions a single
/// program may contain -- real, deliberate, small headroom (2x this
/// milestone's own largest real test program, which uses 2 functions),
/// not picked to exactly fit today's cases; a real, disclosed,
/// deliberately-unraised cap, the same "small real headroom, not
/// maximal generality" discipline MAX_TOKENS/MAX_VARS above already
/// established for their own milestones.
const MAX_FUNCS: u64 = 4;

/// MILESTONE 74: the maximum number of real forward-call PATCH-LIST
/// entries a single program's own codegen pass may record (see
/// `PendingCall` below) -- real, deliberate, small headroom (the
/// largest real test program added this milestone needs exactly 2), the
/// same "small real headroom, not maximal generality" discipline
/// MAX_FUNCS/MAX_PARAMS above already established. Exceeding it is a
/// real, disclosed `CodeGenError::TooManyForwardCalls` (see that
/// variant's own doc comment) -- note this caps the number of CALL
/// SITES that happen to reference a not-yet-compiled callee, not the
/// number of functions or calls in general; an ordinary call to an
/// already-compiled function (backward reference, or self) never
/// touches this list at all.
const MAX_PENDING_CALLS: u64 = 8;

/// MILESTONE 74: a real sentinel value for `FuncSym::code_off`, meaning
/// "this function's own name/param_count are already registered in the
/// table (see gen_program()'s own new real first pass below) but its
/// body has not been compiled yet, so its real code offset is not yet
/// known." `u64::MAX` is never a real in-buffer offset for this
/// subset's own CodeBuf (`gen_program()`'s own `CodeBuf::new(2048)` caps
/// every real offset at 2048), so it cannot collide with a genuine,
/// already-resolved `code_off`.
const UNRESOLVED_CODE_OFF: u64 = u64::MAX;

/// MILESTONE 74: one entry in the real forward-call patch list --
/// `field_off` is the byte offset (into the shared CodeBuf, same
/// convention `emit_jz_placeholder()`/`patch_rel32()` already use) of a
/// `call rel32` instruction's own 4-byte rel32 FIELD that was emitted
/// via `CodeBuf::emit_call_placeholder()` before its own callee had been
/// compiled; `func_idx` is that callee's own index into `funcs_ptr`
/// (NOT its code_off, which is exactly the value not yet known at the
/// moment this entry is recorded) -- resolved to a real code_off, and
/// the placeholder patched via `CodeBuf::patch_call_rel32()`, only after
/// gen_program()'s own per-function compile loop finishes and every
/// function's own real code_off is guaranteed final.
#[derive(Clone, Copy)]
#[repr(C)]
struct PendingCall {
    field_off: u64,
    func_idx: u64,
}

unsafe fn pending_write(p: u64, idx: u64, v: PendingCall) {
    unsafe { core::ptr::write((p + idx * core::mem::size_of::<PendingCall>() as u64) as *mut PendingCall, v) };
}
unsafe fn pending_read(p: u64, idx: u64) -> PendingCall {
    unsafe { core::ptr::read((p + idx * core::mem::size_of::<PendingCall>() as u64) as *const PendingCall) }
}

unsafe fn param_write(p: u64, idx: u64, v: ParamInfo) {
    unsafe { core::ptr::write((p + idx * core::mem::size_of::<ParamInfo>() as u64) as *mut ParamInfo, v) };
}
unsafe fn param_read(p: u64, idx: u64) -> ParamInfo {
    unsafe { core::ptr::read((p + idx * core::mem::size_of::<ParamInfo>() as u64) as *const ParamInfo) }
}

/// MILESTONE 72: a single u64 (ExprNode pointer) write/read into an
/// EXPR_CALL's own `call_args_ptr` buffer -- same convention as
/// param_write/param_read above, just element type u64 instead of
/// ParamInfo.
unsafe fn call_arg_write(p: u64, idx: u64, v: u64) {
    unsafe { core::ptr::write((p + idx * 8) as *mut u64, v) };
}
unsafe fn call_arg_read(p: u64, idx: u64) -> u64 {
    unsafe { core::ptr::read((p + idx * 8) as *const u64) }
}

#[repr(C)]
struct FuncDef {
    name_off: u32,
    name_len: u8,
    body: u64, // head of the StmtNode list, 0 if empty
    stmt_count: u32,
    // MILESTONE 72: this function's own parameters -- `params_ptr` is a
    // malloc()ed array of `param_count` ParamInfo entries, in real source
    // declaration order (0 when param_count == 0, the same "0 = absent"
    // convention this whole AST already uses). `next` is the next
    // FuncDef in the program's own function list (0 if this is the last
    // function) -- Milestone 67-71's grammar only ever had ONE function
    // per program, so FuncDef never needed this link before; a genuinely
    // new field for a genuinely new capability, not an oversight.
    params_ptr: u64,
    param_count: u8,
    next: u64,
}

unsafe fn alloc_expr() -> u64 {
    unsafe { malloc(core::mem::size_of::<ExprNode>() as u64) }
}
unsafe fn alloc_stmt() -> u64 {
    unsafe { malloc(core::mem::size_of::<StmtNode>() as u64) }
}
unsafe fn alloc_func() -> u64 {
    unsafe { malloc(core::mem::size_of::<FuncDef>() as u64) }
}

enum ParseError {
    /// Real parse failure -- the token index (into the lexer's own
    /// output) where parsing could not continue.
    UnexpectedToken(u64),
    OutOfMemory,
    /// MILESTONE 72: a function's own parameter list, or a single call
    /// expression's own argument list, declared/passed more than
    /// MAX_PARAMS entries -- a real, disclosed, deliberately unraised cap
    /// (see MAX_PARAMS's own doc comment), not exercised by this
    /// milestone's own self-test cases (all of which stay at or under the
    /// cap), so this Err path is real but genuinely unverified -- disclosed
    /// honestly rather than silently left untested.
    TooManyParams,
    TooManyArgs,
    /// MILESTONE 72: the source declared more than MAX_FUNCS function
    /// definitions -- same "real but unexercised, honestly disclosed"
    /// status as TooManyParams/TooManyArgs above.
    TooManyFunctions,
}

struct Parser {
    toks_ptr: u64,
    ntoks: u64,
    pos: u64,
}

impl Parser {
    unsafe fn peek(&self) -> Token {
        unsafe { tok_read(self.toks_ptr, self.pos) }
    }

    unsafe fn advance(&mut self) -> Token {
        let t = unsafe { self.peek() };
        if self.pos + 1 < self.ntoks {
            self.pos += 1;
        }
        t
    }

    unsafe fn expect(&mut self, kind: u8) -> Result<Token, ParseError> {
        let t = unsafe { self.peek() };
        if t.kind == kind {
            Ok(unsafe { self.advance() })
        } else {
            Err(ParseError::UnexpectedToken(self.pos))
        }
    }

    unsafe fn parse_factor(&mut self) -> Result<u64, ParseError> {
        let t = unsafe { self.peek() };
        match t.kind {
            TOK_INTLIT => {
                unsafe { self.advance() };
                let node = unsafe { alloc_expr() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut ExprNode,
                        ExprNode {
                            kind: EXPR_INTLIT,
                            int_val: t.int_val,
                            ident_off: 0,
                            ident_len: 0,
                            op: 0,
                            left: 0,
                            right: 0,
                            call_args_ptr: 0,
                            call_argc: 0,
                        },
                    )
                };
                Ok(node)
            }
            // MILESTONE 72: IDENT is now ambiguous between an ordinary
            // variable reference (Milestone 67's original shape) and a
            // call expression (IDENT "(" args? ")", new this milestone) --
            // resolved with exactly one token of lookahead AFTER consuming
            // the IDENT itself, the same "advance, then peek the next
            // token to decide which alternative" shape this file's own
            // parse_stmt() already uses in several places (e.g.
            // TOK_IDENT's own assign_stmt arm below).
            TOK_IDENT => {
                let name = unsafe { self.advance() };
                if unsafe { self.peek() }.kind == TOK_LPAREN {
                    unsafe { self.advance() }; // consume '('
                    let mut argc: u8 = 0;
                    let mut args_ptr: u64 = 0;
                    if unsafe { self.peek() }.kind != TOK_RPAREN {
                        args_ptr = unsafe { malloc(MAX_PARAMS * 8) };
                        if args_ptr == 0 {
                            return Err(ParseError::OutOfMemory);
                        }
                        loop {
                            if argc as u64 >= MAX_PARAMS {
                                return Err(ParseError::TooManyArgs);
                            }
                            // Each argument is a full cond_expr (not just a
                            // factor) -- an argument can be any expression
                            // this subset can build, e.g. `add(x, y + 1)`,
                            // the same widening parse_cond_expr's own
                            // parenthesized-factor case already relies on.
                            let e = unsafe { self.parse_logic_or() }?;
                            unsafe { call_arg_write(args_ptr, argc as u64, e) };
                            argc += 1;
                            if unsafe { self.peek() }.kind == TOK_COMMA {
                                unsafe { self.advance() };
                                continue;
                            }
                            break;
                        }
                    }
                    unsafe { self.expect(TOK_RPAREN) }?;
                    let node = unsafe { alloc_expr() };
                    if node == 0 {
                        return Err(ParseError::OutOfMemory);
                    }
                    unsafe {
                        core::ptr::write(
                            node as *mut ExprNode,
                            ExprNode {
                                kind: EXPR_CALL,
                                int_val: 0,
                                ident_off: name.ident_off,
                                ident_len: name.ident_len,
                                op: 0,
                                left: 0,
                                right: 0,
                                call_args_ptr: args_ptr,
                                call_argc: argc,
                            },
                        )
                    };
                    Ok(node)
                } else {
                    let node = unsafe { alloc_expr() };
                    if node == 0 {
                        return Err(ParseError::OutOfMemory);
                    }
                    unsafe {
                        core::ptr::write(
                            node as *mut ExprNode,
                            ExprNode {
                                kind: EXPR_IDENT,
                                int_val: 0,
                                ident_off: name.ident_off,
                                ident_len: name.ident_len,
                                op: 0,
                                left: 0,
                                right: 0,
                                call_args_ptr: 0,
                                call_argc: 0,
                            },
                        )
                    };
                    Ok(node)
                }
            }
            TOK_LPAREN => {
                unsafe { self.advance() };
                // MILESTONE 70: recurses through parse_cond_expr (not the
                // narrower parse_expr) so a parenthesized comparison like
                // `(a < b)` is a legal sub-expression -- see this file's
                // own top grammar comment for why that widening is
                // deliberate.
                let e = unsafe { self.parse_logic_or() }?;
                unsafe { self.expect(TOK_RPAREN) }?;
                Ok(e)
            }
            _ => Err(ParseError::UnexpectedToken(self.pos)),
        }
    }

    /// MILESTONE 76: real unary-minus parse -- `unary := "-" unary |
    /// factor`, one token of lookahead before falling through to the
    /// pre-existing parse_factor() chain (INTLIT/IDENT/call/parenthesized
    /// sub-expression, all UNCHANGED). Right-recursive on itself (not
    /// parse_factor()), so a doubled sign like `--x` parses as the real
    /// C shape UNARY('-', UNARY('-', IDENT(x))) -- double negation, not a
    /// special-cased decrement operator (this subset has no `--`/`++` at
    /// all, and never will from this alone: nothing here consumes two
    /// adjacent MINUS tokens as one lexeme, the lexer still emits them as
    /// two separate TOK_MINUS tokens, exactly like `- -x` with a space).
    /// Called from parse_term() below in place of parse_factor() at every
    /// operand position, so unary minus binds TIGHTER than `*`/`/` (`-a *
    /// b` parses as `(-a) * b`) and, transitively, tighter than binary
    /// `+`/`-` too (parse_expr() still calls parse_term(), unchanged) --
    /// checked against real C's own operator-precedence table before
    /// picking this exact insertion point, not guessed.
    /// MILESTONE 79: extended for real unary `~` (bitwise NOT) -- SAME
    /// slot unary minus already occupies (`unary := ("-"|"~") unary |
    /// factor`), so `~` binds exactly as tightly as `-` (both bind
    /// tighter than `*`/`/`, real C's own ordering) and both compose
    /// with each other via the same recursive-unary call (`-~x`, `~-x`,
    /// `~~x` are all real, legal parses here, same as real C).
    /// MILESTONE 83: extended once more for real logical `!` -- the same
    /// slot again (`unary := ("-"|"~"|"!") unary | factor`), same
    /// precedence as `-`/`~` (tighter than `*`/`/`, real C's own
    /// ordering -- `!a * b` is `(!a) * b`), and it composes with the
    /// other two and itself through the same recursive-unary call
    /// (`!!x`, `!-x`, `-!x`, `~!x` are all real, legal parses here).
    unsafe fn parse_unary(&mut self) -> Result<u64, ParseError> {
        let k = unsafe { self.peek() }.kind;
        let op = if k == TOK_MINUS {
            OP_NEG
        } else if k == TOK_TILDE {
            OP_BITNOT
        } else if k == TOK_BANG {
            OP_LOGNOT
        } else {
            return unsafe { self.parse_factor() };
        };
        unsafe { self.advance() };
        let operand = unsafe { self.parse_unary() }?;
        let node = unsafe { alloc_expr() };
        if node == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                node as *mut ExprNode,
                ExprNode { kind: EXPR_UNARY, int_val: 0, ident_off: 0, ident_len: 0, op, left: operand, right: 0, call_args_ptr: 0, call_argc: 0 },
            )
        };
        Ok(node)
    }

    unsafe fn parse_term(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_unary() }?;
        loop {
            let k = unsafe { self.peek() }.kind;
            if k != TOK_STAR && k != TOK_SLASH {
                break;
            }
            let op = if k == TOK_STAR { b'*' } else { b'/' };
            unsafe { self.advance() };
            let right = unsafe { self.parse_unary() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    unsafe fn parse_expr(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_term() }?;
        loop {
            let k = unsafe { self.peek() }.kind;
            if k != TOK_PLUS && k != TOK_MINUS {
                break;
            }
            let op = if k == TOK_PLUS { b'+' } else { b'-' };
            unsafe { self.advance() };
            let right = unsafe { self.parse_term() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    /// MILESTONE 70: real top-level condition-expression parse --
    /// `expr (relop expr)?`. Wraps the pre-existing arithmetic `expr`
    /// (sum-of-terms, UNCHANGED) with an optional single comparison,
    /// deliberately non-chaining (`a < b < c` is not legal -- a real,
    /// disclosed scope cut, see this file's own top grammar comment).
    /// This is the function assign_stmt/return_stmt/if_stmt's condition
    /// and factor's parenthesized case now call, in place of the bare
    /// `parse_expr()` Milestone 67 originally wired them to.
    /// MILESTONE 76 UPDATE: this is no longer the real top-level
    /// expression production -- `parse_logic_and()`/`parse_logic_or()`
    /// immediately below now wrap it for `&&`/`||`, and every call site
    /// named in this comment (assign_stmt/return_stmt/if_stmt/while_stmt's
    /// condition, a call argument, factor's parenthesized case) now calls
    /// `parse_logic_or()` instead of this function directly -- see
    /// `parse_logic_or()`'s own doc comment for the full updated
    /// production list. This function's OWN shape (`expr (relop expr)?`,
    /// still deliberately non-chaining on relops) is completely
    /// UNCHANGED; it is simply no longer the outermost layer.
    /// MILESTONE 79 UPDATE: both operands now come from `parse_shift_
    /// expr()` (below) instead of `parse_expr()` directly -- real C
    /// puts `<<`/`>>` BETWEEN additive and relational (tighter than
    /// comparisons, looser than `+`/`-`), checked against the C
    /// standard's own precedence table before picking this exact
    /// insertion point, the same discipline Milestone 76's own
    /// unary/`&&`/`||` insertion used. This function's OWN shape
    /// (single, non-chaining relop) is otherwise completely UNCHANGED.
    unsafe fn parse_cond_expr(&mut self) -> Result<u64, ParseError> {
        let left = unsafe { self.parse_shift_expr() }?;
        let k = unsafe { self.peek() }.kind;
        let op = match k {
            TOK_EQ => OP_EQ,
            TOK_NE => OP_NE,
            TOK_LT => OP_LT,
            TOK_GT => OP_GT,
            TOK_LE => OP_LE,
            TOK_GE => OP_GE,
            _ => return Ok(left),
        };
        unsafe { self.advance() };
        let right = unsafe { self.parse_shift_expr() }?;
        let node = unsafe { alloc_expr() };
        if node == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                node as *mut ExprNode,
                ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op, left, right, call_args_ptr: 0, call_argc: 0 },
            )
        };
        Ok(node)
    }

    /// MILESTONE 79: real `<<`/`>>` parse -- `shift_expr := expr (("<<"
    /// | ">>") expr)*`, left-associative CHAINING (real C's own rule:
    /// `1 << 2 << 3` is `(1 << 2) << 3`), calling the pre-existing
    /// `parse_expr()` (the additive sum-of-terms level, UNCHANGED) for
    /// each operand -- so `1 << 2 + 1` parses as `1 << (2 + 1)`, real
    /// C's own precedence (`+` binds tighter than `<<`), not the other
    /// reading. Both operators share one AST-building loop the same
    /// way `*`/`/` already do in `parse_term()` above; `gen_expr()`'s
    /// own SHL/SAR codegen (below) is what distinguishes them.
    unsafe fn parse_shift_expr(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_expr() }?;
        loop {
            let k = unsafe { self.peek() }.kind;
            if k != TOK_SHL && k != TOK_SHR {
                break;
            }
            let op = if k == TOK_SHL { OP_SHL } else { OP_SHR };
            unsafe { self.advance() };
            let right = unsafe { self.parse_expr() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    /// MILESTONE 79: real `&` (bitwise AND) parse -- `bit_and :=
    /// cond_expr ("&" cond_expr)*`, calling the pre-existing
    /// `parse_cond_expr()` (relational/equality, UNCHANGED) for each
    /// operand -- real C's own rule that `&`/`^`/`|` all bind LOOSER
    /// than every comparison, so `a & b == c` parses as `a & (b == c)`,
    /// not `(a & b) == c` (a real, classic C precedence trap this
    /// insertion point deliberately gets right, verified directly by
    /// this milestone's own CASE 38 self-test below, not just asserted
    /// in a comment).
    unsafe fn parse_bit_and(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_cond_expr() }?;
        loop {
            if unsafe { self.peek() }.kind != TOK_AMP {
                break;
            }
            unsafe { self.advance() };
            let right = unsafe { self.parse_cond_expr() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op: OP_BITAND, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    /// MILESTONE 79: real `^` (bitwise XOR) parse -- `bit_xor :=
    /// bit_and ("^" bit_and)*`, one precedence level above `&` (real
    /// C's own ordering: `&` binds tighter than `^`).
    unsafe fn parse_bit_xor(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_bit_and() }?;
        loop {
            if unsafe { self.peek() }.kind != TOK_CARET {
                break;
            }
            unsafe { self.advance() };
            let right = unsafe { self.parse_bit_and() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op: OP_BITXOR, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    /// MILESTONE 79: real `|` (bitwise OR) parse -- `bit_or := bit_xor
    /// ("|" bit_xor)*`, one precedence level above `^` (real C's own
    /// ordering: `^` binds tighter than `|`), and this is now what
    /// `parse_logic_and()` (below) calls in place of `parse_cond_expr()`
    /// directly -- the same "wrap the existing top layer with one more"
    /// widening Milestone 76's own `parse_logic_and()`/`parse_logic_or()`
    /// already established as this codegen's precedent.
    unsafe fn parse_bit_or(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_bit_xor() }?;
        loop {
            if unsafe { self.peek() }.kind != TOK_PIPE {
                break;
            }
            unsafe { self.advance() };
            let right = unsafe { self.parse_bit_xor() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op: OP_BITOR, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    /// MILESTONE 76: real `&&` parse -- `logic_and := cond_expr ("&&"
    /// cond_expr)*`, left-associative CHAINING (unlike parse_cond_expr()'s
    /// own single, deliberately non-chaining relop just above -- a real,
    /// intentional difference, not an inconsistency: real C's `&&`/`||`
    /// chain, its relational operators don't, and this milestone's own
    /// short-circuit codegen below composes across any number of chained
    /// operands the same left-to-right way EXPR_BINARY's arithmetic
    /// chaining already does). Each `&&` node is built the same
    /// `ExprNode { kind: EXPR_BINARY, op: OP_LOGAND, .. }` shape every
    /// other binary operator already uses -- gen_expr() (below) is what
    /// gives `OP_LOGAND` its real, distinguishing short-circuit codegen,
    /// not a new AST node kind.
    /// MILESTONE 79 UPDATE: both operands now come from `parse_bit_or()`
    /// instead of `parse_cond_expr()` directly -- real C's own ordering
    /// puts `|`/`^`/`&` all between comparisons and `&&`, so
    /// `parse_bit_or()` (which itself calls down through `parse_bit_xor()`
    /// -> `parse_bit_and()` -> the original `parse_cond_expr()`, all
    /// UNCHANGED) is now the real production `logic_and` grammar. This
    /// function's OWN shape (chaining `&&`) is otherwise identical.
    unsafe fn parse_logic_and(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_bit_or() }?;
        loop {
            if unsafe { self.peek() }.kind != TOK_ANDAND {
                break;
            }
            unsafe { self.advance() };
            let right = unsafe { self.parse_bit_or() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op: OP_LOGAND, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    /// MILESTONE 76: real `||` parse, one precedence level above `&&`
    /// (real C's own ordering -- `&&` binds tighter than `||`, checked
    /// against the C standard's own operator-precedence table before
    /// picking this layering, not guessed) -- `logic_or := logic_and
    /// ("||" logic_and)*`. THIS is now the real top-level expression
    /// production this subset's grammar has: every assign_stmt/
    /// return_stmt/if_stmt/while_stmt condition, call argument, and
    /// parenthesized sub-expression now parses through `parse_logic_or()`,
    /// not the narrower `parse_cond_expr()` Milestone 70 originally wired
    /// them to (see this file's own top Milestone 76 grammar comment for
    /// the complete updated production list) -- a real, deliberate
    /// widening of what a "condition" or "value" can be, the exact same
    /// kind of widening Milestone 70's own factor-recurses-through-
    /// cond_expr change already established as this codegen's precedent
    /// for growing an expression grammar's top layer without touching any
    /// lower one.
    unsafe fn parse_logic_or(&mut self) -> Result<u64, ParseError> {
        let mut left = unsafe { self.parse_logic_and() }?;
        loop {
            if unsafe { self.peek() }.kind != TOK_OROR {
                break;
            }
            unsafe { self.advance() };
            let right = unsafe { self.parse_logic_and() }?;
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op: OP_LOGOR, left, right, call_args_ptr: 0, call_argc: 0 },
                )
            };
            left = node;
        }
        Ok(left)
    }

    /// MILESTONE 84 logic, extracted at MILESTONE 86 so `for`'s init and
    /// step clauses can reuse it. `name` is the already-consumed IDENT;
    /// the next token is `=` or one of the nine compound-assignment
    /// operators. `x OP= e` is desugared to `x = (x OP e)` -- a
    /// synthesized EXPR_BINARY over a fresh EXPR_IDENT reference to
    /// `name` and the parsed RHS. Returns a STMT_ASSIGN node. `consume_
    /// semi` is true for a plain assignment statement (which ends in
    /// `;`), false for a `for` init/step clause (which ends in `;`/`)`
    /// consumed by the `for` parser itself).
    unsafe fn finish_ident_assign(&mut self, name: Token, consume_semi: bool) -> Result<u64, ParseError> {
        let opk = unsafe { self.peek() }.kind;
        let binop: Option<u8> = if opk == TOK_PLUSEQ { Some(b'+') }
            else if opk == TOK_MINUSEQ { Some(b'-') }
            else if opk == TOK_STAREQ { Some(b'*') }
            else if opk == TOK_SLASHEQ { Some(b'/') }
            else if opk == TOK_AMPEQ { Some(OP_BITAND) }
            else if opk == TOK_PIPEEQ { Some(OP_BITOR) }
            else if opk == TOK_CARETEQ { Some(OP_BITXOR) }
            else if opk == TOK_SHLEQ { Some(OP_SHL) }
            else if opk == TOK_SHREQ { Some(OP_SHR) }
            else { None };
        if binop.is_none() {
            unsafe { self.expect(TOK_ASSIGN) }?;
        } else {
            unsafe { self.advance() };
        }
        let rhs = unsafe { self.parse_logic_or() }?;
        if consume_semi {
            unsafe { self.expect(TOK_SEMI) }?;
        }
        let expr = if let Some(op) = binop {
            let lhs_ref = unsafe { alloc_expr() };
            if lhs_ref == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    lhs_ref as *mut ExprNode,
                    ExprNode { kind: EXPR_IDENT, int_val: 0, ident_off: name.ident_off, ident_len: name.ident_len, op: 0, left: 0, right: 0, call_args_ptr: 0, call_argc: 0 },
                )
            };
            let bin = unsafe { alloc_expr() };
            if bin == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    bin as *mut ExprNode,
                    ExprNode { kind: EXPR_BINARY, int_val: 0, ident_off: 0, ident_len: 0, op, left: lhs_ref, right: rhs, call_args_ptr: 0, call_argc: 0 },
                )
            };
            bin
        } else {
            rhs
        };
        let node = unsafe { alloc_stmt() };
        if node == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                node as *mut StmtNode,
                StmtNode { kind: STMT_ASSIGN, ident_off: name.ident_off, ident_len: name.ident_len, expr, next: 0, then_body: 0, else_body: 0 },
            )
        };
        Ok(node)
    }

    unsafe fn parse_stmt(&mut self) -> Result<u64, ParseError> {
        let t = unsafe { self.peek() };
        match t.kind {
            TOK_INT => {
                unsafe { self.advance() };
                let name = unsafe { self.expect(TOK_IDENT) }?;
                unsafe { self.expect(TOK_SEMI) }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode {
                            kind: STMT_DECL,
                            ident_off: name.ident_off,
                            ident_len: name.ident_len,
                            expr: 0,
                            next: 0,
                            then_body: 0,
                            else_body: 0,
                        },
                    )
                };
                Ok(node)
            }
            TOK_RETURN => {
                unsafe { self.advance() };
                let e = unsafe { self.parse_logic_or() }?;
                unsafe { self.expect(TOK_SEMI) }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode { kind: STMT_RETURN, ident_off: 0, ident_len: 0, expr: e, next: 0, then_body: 0, else_body: 0 },
                    )
                };
                Ok(node)
            }
            TOK_IDENT => {
                let name = unsafe { self.advance() };
                unsafe { self.finish_ident_assign(name, true) }
            }
            // MILESTONE 86: `for (init; cond; step) { body }` -- desugared
            // to `init; while (cond) { body step; }`. Reuses Milestone
            // 71's STMT_WHILE (and therefore its codegen) verbatim; the
            // only new AST work is chaining `init` in front of the while
            // and appending `step` to the tail of the while body. `init`
            // and `step` are ordinary IDENT-assignments (plain `=` or any
            // Milestone-84 compound form), parsed by the SAME
            // `finish_ident_assign()` the plain assign-statement uses,
            // with `consume_semi = false`; each is optional. An absent
            // `cond` becomes a synthesized `EXPR_INTLIT(1)` (always
            // true). The loop variable must be declared before the loop
            // -- this subset has no combined `int i = 0` decl-init
            // anywhere, a real disclosed scope cut, not introduced here.
            TOK_FOR => {
                unsafe { self.advance() };
                unsafe { self.expect(TOK_LPAREN) }?;
                // init (optional)
                let init: u64 = if unsafe { self.peek() }.kind == TOK_SEMI {
                    0
                } else {
                    let n = unsafe { self.expect(TOK_IDENT) }?;
                    unsafe { self.finish_ident_assign(n, false) }?
                };
                unsafe { self.expect(TOK_SEMI) }?;
                // cond (optional -- absent == always-true 1)
                let cond: u64 = if unsafe { self.peek() }.kind == TOK_SEMI {
                    let one = unsafe { alloc_expr() };
                    if one == 0 {
                        return Err(ParseError::OutOfMemory);
                    }
                    unsafe {
                        core::ptr::write(
                            one as *mut ExprNode,
                            ExprNode { kind: EXPR_INTLIT, int_val: 1, ident_off: 0, ident_len: 0, op: 0, left: 0, right: 0, call_args_ptr: 0, call_argc: 0 },
                        )
                    };
                    one
                } else {
                    unsafe { self.parse_logic_or() }?
                };
                unsafe { self.expect(TOK_SEMI) }?;
                // step (optional)
                let step: u64 = if unsafe { self.peek() }.kind == TOK_RPAREN {
                    0
                } else {
                    let n = unsafe { self.expect(TOK_IDENT) }?;
                    unsafe { self.finish_ident_assign(n, false) }?
                };
                unsafe { self.expect(TOK_RPAREN) }?;
                unsafe { self.expect(TOK_LBRACE) }?;
                let body = unsafe { self.parse_stmt_list_until_rbrace() }?;
                // MILESTONE 87: the `step` clause now rides in the
                // STMT_WHILE's `else_body` slot (unused for a plain
                // `while`), NOT appended to `then_body`. Behaviourally
                // identical for a `for` with no `break`/`continue` -- the
                // codegen still runs `body` then `step` then re-checks
                // `cond` -- but keeping `step` distinct is what lets
                // `continue` jump to it (real C `for`-continue semantics)
                // instead of skipping it. `step == 0` (a stepless `for`)
                // leaves `else_body` at 0, i.e. a plain `while`.
                let while_node = unsafe { alloc_stmt() };
                if while_node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        while_node as *mut StmtNode,
                        StmtNode { kind: STMT_WHILE, ident_off: 0, ident_len: 0, expr: cond, next: 0, then_body: body, else_body: step },
                    )
                };
                if init == 0 {
                    Ok(while_node)
                } else {
                    unsafe { (*(init as *mut StmtNode)).next = while_node };
                    Ok(init)
                }
            }
            TOK_IF => {
                unsafe { self.advance() };
                unsafe { self.expect(TOK_LPAREN) }?;
                let cond = unsafe { self.parse_logic_or() }?;
                unsafe { self.expect(TOK_RPAREN) }?;
                unsafe { self.expect(TOK_LBRACE) }?;
                let then_body = unsafe { self.parse_stmt_list_until_rbrace() }?;
                let mut else_body: u64 = 0;
                if unsafe { self.peek() }.kind == TOK_ELSE {
                    unsafe { self.advance() };
                    unsafe { self.expect(TOK_LBRACE) }?;
                    else_body = unsafe { self.parse_stmt_list_until_rbrace() }?;
                }
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode { kind: STMT_IF, ident_off: 0, ident_len: 0, expr: cond, next: 0, then_body, else_body },
                    )
                };
                Ok(node)
            }
            // MILESTONE 71: `while_stmt := "while" "(" cond_expr ")" "{"
            // stmt* "}"` -- deliberately the SAME shape as if_stmt's own
            // condition-plus-braced-body parse above, minus the optional
            // `else`, reusing parse_stmt_list_until_rbrace() the exact
            // same way if/else's then_body already does.
            TOK_WHILE => {
                unsafe { self.advance() };
                unsafe { self.expect(TOK_LPAREN) }?;
                let cond = unsafe { self.parse_logic_or() }?;
                unsafe { self.expect(TOK_RPAREN) }?;
                unsafe { self.expect(TOK_LBRACE) }?;
                let body = unsafe { self.parse_stmt_list_until_rbrace() }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode {
                            kind: STMT_WHILE,
                            ident_off: 0,
                            ident_len: 0,
                            expr: cond,
                            next: 0,
                            then_body: body,
                            else_body: 0,
                        },
                    )
                };
                Ok(node)
            }
            // MILESTONE 87: `break ;` and `continue ;` -- two trivial
            // leaf statements. All the real work is codegen-side (the
            // loop-context stack in gen_stmt_list); the parser just
            // records which one it is. A `break`/`continue` outside any
            // loop parses fine here and is caught at codegen as
            // CodeGenError::BreakOutsideLoop/ContinueOutsideLoop.
            TOK_BREAK | TOK_CONTINUE => {
                let kind = if unsafe { self.peek() }.kind == TOK_BREAK { STMT_BREAK } else { STMT_CONTINUE };
                unsafe { self.advance() };
                unsafe { self.expect(TOK_SEMI) }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode { kind, ident_off: 0, ident_len: 0, expr: 0, next: 0, then_body: 0, else_body: 0 },
                    )
                };
                Ok(node)
            }
            _ => Err(ParseError::UnexpectedToken(self.pos)),
        }
    }

    /// MILESTONE 70: real, shared "parse statements until the closing
    /// `}`" loop -- refactored out of Milestone 67's own
    /// `parse_function()` (which used to inline this exact loop) so
    /// if/else bodies can reuse it too, rather than duplicating it.
    /// Assumes the opening `{` was already consumed by the caller;
    /// consumes the closing `}` itself before returning. Returns the
    /// real head of the resulting singly-linked StmtNode list (0 if the
    /// block is empty), in real source order -- identical shape to
    /// `parse_function()`'s own original `head`/`tail` loop.
    unsafe fn parse_stmt_list_until_rbrace(&mut self) -> Result<u64, ParseError> {
        let mut head: u64 = 0;
        let mut tail: u64 = 0;
        loop {
            let k = unsafe { self.peek() }.kind;
            if k == TOK_RBRACE {
                break;
            }
            if k == TOK_EOF {
                return Err(ParseError::UnexpectedToken(self.pos));
            }
            let stmt = unsafe { self.parse_stmt() }?;
            if head == 0 {
                head = stmt;
            } else {
                unsafe { (*(tail as *mut StmtNode)).next = stmt };
            }
            // MILESTONE 86: parse_stmt() may now return a SHORT CHAIN, not
            // a single node -- `for` returns `init -> while`. Advance
            // `tail` to the real end of whatever came back so the next
            // statement links after the whole thing, not after its head.
            tail = stmt;
            while unsafe { (*(tail as *const StmtNode)).next } != 0 {
                tail = unsafe { (*(tail as *const StmtNode)).next };
            }
        }
        unsafe { self.expect(TOK_RBRACE) }?;
        Ok(head)
    }

    /// MILESTONE 72: `"int" IDENT ("," "int" IDENT)*` parameter list,
    /// called with `self.pos` already just past the function's own `(` --
    /// consumes up to (but not including) the closing `)`. Returns
    /// `(0, 0)` for an empty list (the "()" case Milestone 67 already
    /// supported) without even malloc()ing a params buffer -- the same
    /// "0 = absent, no allocation for the empty case" convention this
    /// whole AST already uses (e.g. StmtNode.else_body). A parameter
    /// beyond MAX_PARAMS is a real, disclosed TooManyParams error (see
    /// that variant's own doc comment).
    unsafe fn parse_params(&mut self) -> Result<(u64, u8), ParseError> {
        if unsafe { self.peek() }.kind == TOK_RPAREN {
            return Ok((0, 0));
        }
        let params_ptr = unsafe { malloc(MAX_PARAMS * core::mem::size_of::<ParamInfo>() as u64) };
        if params_ptr == 0 {
            return Err(ParseError::OutOfMemory);
        }
        let mut count: u8 = 0;
        loop {
            unsafe { self.expect(TOK_INT) }?;
            let name = unsafe { self.expect(TOK_IDENT) }?;
            if count as u64 >= MAX_PARAMS {
                return Err(ParseError::TooManyParams);
            }
            unsafe { param_write(params_ptr, count as u64, ParamInfo { name_off: name.ident_off, name_len: name.ident_len }) };
            count += 1;
            if unsafe { self.peek() }.kind == TOK_COMMA {
                unsafe { self.advance() };
                continue;
            }
            break;
        }
        Ok((params_ptr, count))
    }

    /// Real top-level single-function parse: `"int" IDENT "(" params? ")"
    /// "{" stmt* "}"`. Builds and returns a real, malloc()-allocated
    /// FuncDef whose `body` is the head of a real singly-linked StmtNode
    /// list, in real source order. `stmt_count` counts only TOP-LEVEL
    /// statements (unchanged Milestone 67 semantics -- a nested if/else's
    /// own then/else statements are not counted here, matching what
    /// CASE 1's pre-existing self-test check already assumes).
    /// MILESTONE 72: grammar widened from Milestone 67's fixed `"(" ")"`
    /// to `"(" params? ")"` -- see parse_params() above -- and the
    /// resulting FuncDef's own new `params_ptr`/`param_count` fields are
    /// populated from it; `next` is always left 0 here (this function
    /// only ever builds ONE FuncDef with no linkage of its own -- linking
    /// several into a real program-level list is parse_program()'s job,
    /// below, not this function's).
    unsafe fn parse_function(&mut self) -> Result<u64, ParseError> {
        unsafe { self.expect(TOK_INT) }?;
        let name = unsafe { self.expect(TOK_IDENT) }?;
        unsafe { self.expect(TOK_LPAREN) }?;
        let (params_ptr, param_count) = unsafe { self.parse_params() }?;
        unsafe { self.expect(TOK_RPAREN) }?;
        unsafe { self.expect(TOK_LBRACE) }?;

        let head = unsafe { self.parse_stmt_list_until_rbrace() }?;

        let mut count: u32 = 0;
        let mut cur = head;
        while cur != 0 {
            count += 1;
            cur = unsafe { core::ptr::read(cur as *const StmtNode) }.next;
        }

        let func = unsafe { alloc_func() };
        if func == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                func as *mut FuncDef,
                FuncDef {
                    name_off: name.ident_off,
                    name_len: name.ident_len,
                    body: head,
                    stmt_count: count,
                    params_ptr,
                    param_count,
                    next: 0,
                },
            )
        };
        Ok(func)
    }

    /// MILESTONE 72: real top-level MULTI-function parse --
    /// `program := function+` -- new this milestone; Milestone 67-71's
    /// grammar had no `program` production at all, every prior self-test
    /// case called `parse_function()` directly against a single-function
    /// source. Links each parsed FuncDef into a real singly-linked list
    /// via its own `next` field (parse_function() itself always leaves
    /// `next` 0; THIS function is what actually threads the list
    /// together), in real source order, and returns the list's head.
    /// More than MAX_FUNCS function definitions is a real, disclosed
    /// TooManyFunctions error (see that variant's own doc comment); an
    /// empty program (EOF before even one function) is a real
    /// UnexpectedToken error at the EOF token's own index -- there is no
    /// legal empty program in this grammar, the same "no legal empty
    /// input" stance Milestone 67's own parse_function() already took for
    /// a missing `int`.
    unsafe fn parse_program(&mut self) -> Result<u64, ParseError> {
        let mut head: u64 = 0;
        let mut tail: u64 = 0;
        let mut n: u64 = 0;
        loop {
            if unsafe { self.peek() }.kind == TOK_EOF {
                break;
            }
            if n >= MAX_FUNCS {
                return Err(ParseError::TooManyFunctions);
            }
            let f = unsafe { self.parse_function() }?;
            if head == 0 {
                head = f;
            } else {
                unsafe { (*(tail as *mut FuncDef)).next = f };
            }
            tail = f;
            n += 1;
        }
        if head == 0 {
            return Err(ParseError::UnexpectedToken(self.pos));
        }
        Ok(head)
    }
}

// =======================================================================
// MILESTONE 68: real x86_64 machine-code generation from the AST built
// above -- Tier 3's second slice. Direct machine-code BYTES are emitted
// here, not textual assembly, because no assembler exists yet to consume
// textual asm (see README.md's own Milestone 68 entry for the full
// reasoning behind that choice). Every generated function follows the
// ordinary x86_64 SysV leaf-function shape (push rbp; mov rbp,rsp;
// sub rsp,N; ...; leave; ret), so it can be invoked directly through a
// plain Rust function pointer -- no assembler, no linker, no ELF-
// wrapping needed for THIS milestone's own verification (the self-test
// below actually CALLS the generated bytes and checks the real returned
// integer, not just their shape).
//
// This is only possible because this kernel does not set
// PageTableFlags::NO_EXECUTE anywhere yet -- checked directly against
// kernel/src/process.rs's own heap-mapping code, BOTH the one-shot
// malloc()-backing path (heap_flags, ~line 4579) and the demand-paged
// try_demand_page_heap() path (~line 1917): both map heap pages with
// only PRESENT | WRITABLE | USER_ACCESSIBLE. A real, disclosed,
// currently-open Tier 11 ("W^X") gap this milestone deliberately RELIES
// ON rather than closes. A future milestone that adds W^X will need a
// dedicated executable-heap allocation (or a real on-disk ELF + a real
// exec()) for this same in-process-call technique to keep working; not
// attempted here.
// =======================================================================

enum CodeGenError {
    BufferFull,
    TooManyVars,
    /// Real semantic-error path: an IDENT (as an expression operand, or
    /// as an ASSIGN target) whose name matches no declared variable.
    /// Carries the byte offset (into the ORIGINAL source, same
    /// convention as LexError::UnknownChar/ParseError::UnexpectedToken
    /// above) of the undeclared identifier's first byte, for
    /// diagnostics -- exercised for real by CASE 7 in the self-test
    /// below, the same "prove the Err path is real, not just an
    /// unexercised variant" discipline Milestone 67's own CASE 2/CASE 3
    /// already established for LexError/ParseError.
    UndeclaredVariable(u64),
    // MILESTONE 72:
    /// A call expression referencing a name that matches no function in
    /// the program's own function table -- the exact same "real
    /// semantic-error path, carries the source byte offset" shape
    /// UndeclaredVariable above already established, just for callee
    /// names instead of variable names. Exercised for real by CASE 29
    /// below.
    UndeclaredFunction(u64),
    /// A call expression passed a different number of arguments than the
    /// callee itself declares parameters -- carries the callee name's own
    /// source byte offset (same convention as UndeclaredFunction). Real,
    /// but NOT exercised by this milestone's own self-test cases (every
    /// call site below is deliberately arity-correct) -- disclosed
    /// honestly rather than silently left untested, the same status
    /// ParseError::TooManyParams/TooManyArgs above already carry.
    ArgCountMismatch(u64),
    /// More function definitions in one program than gen_program()'s own
    /// fixed-size function-symbol-table buffer (MAX_FUNCS entries) can
    /// hold -- parse_program() already independently enforces this same
    /// cap at parse time (ParseError::TooManyFunctions), so this codegen-
    /// side check is a real, deliberate defense-in-depth backstop against
    /// a program built by some OTHER, hypothetical future caller of
    /// gen_program() that skipped parse_program()'s own check, not a
    /// reachable path from this file's own lex-then-parse-then-codegen
    /// pipeline today.
    TooManyFuncs,
    /// gen_program() reached the end of the function list without finding
    /// any function named "main" -- every real program this subset can
    /// execute (Callable or Standalone) needs a real, unambiguous entry
    /// point, exactly the same real requirement ordinary C's own `main`
    /// convention already imposes.
    NoMainFunction,
    /// MILESTONE 74: more genuine forward-reference call sites (a call
    /// whose own callee has not been compiled yet at the point the call
    /// site itself is compiled) in one program than the real, fixed-size
    /// pending-call patch list (MAX_PENDING_CALLS entries) can hold -- the
    /// same real, disclosed, deliberately small-cap-overflow shape
    /// TooManyFuncs above already established, just for this milestone's
    /// own new patch list instead of the function table itself. Real but
    /// NOT exercised by this milestone's own self-test cases (both new
    /// cases below stay well under the cap) -- disclosed honestly rather
    /// than silently left untested, the same status ArgCountMismatch
    /// above already carries.
    TooManyForwardCalls,
    /// MILESTONE 87: a `break` statement outside any loop body. Real
    /// semantic-error path, the same "prove the Err path is real" shape
    /// UndeclaredVariable/UndeclaredFunction already carry -- exercised
    /// for real by CASE 49 in the self-test below.
    BreakOutsideLoop,
    /// MILESTONE 87: a `continue` statement outside any loop body. Same
    /// shape as BreakOutsideLoop; real but NOT separately exercised
    /// (CASE 49 covers the `break` variant -- the enclosing-loop check
    /// is the identical code path), disclosed honestly rather than left
    /// silently untested, the same status ArgCountMismatch already
    /// carries.
    ContinueOutsideLoop,
    /// MILESTONE 87: more `break`/`continue` jumps in one loop body than
    /// the fixed-size per-loop patch list (MAX_LOOP_JUMPS) can hold --
    /// the same real, disclosed, deliberately-small-cap shape
    /// TooManyForwardCalls already established for its own patch list.
    /// Real but NOT exercised (every test loop below stays well under
    /// the cap).
    TooManyLoopJumps,
}

/// MILESTONE 87: fixed cap on live `break`/`continue` jump sites across
/// all currently-nested loops, same "small real headroom" discipline
/// MAX_FUNCS/MAX_PARAMS/MAX_PENDING_CALLS already established.
const MAX_LOOP_JUMPS: usize = 32;

/// MILESTONE 87: sentinel for `gen_stmt_list`'s `loop_top` argument
/// meaning "not inside any loop" (0 is a valid CodeBuf offset, so it
/// can't be the sentinel).
const NOT_IN_LOOP: u64 = u64::MAX;

/// MILESTONE 87: `break`/`continue` patch lists as module `static mut`
/// arenas rather than per-loop malloc (cc.elf's per-process heap is
/// small and the compiler self-test compiles dozens of loop-bearing
/// programs) or per-loop stack arrays (escaped-pointer aliasing through
/// the recursive `gen_stmt_list` made the optimizer read stale values,
/// producing a wild `jmp` that null-faulted cc.elf -- caught by a real
/// boot). Codegen is single-pass and non-reentrant per compile, and
/// nested loops use a save/restore of the counts (each `STMT_WHILE`
/// records the count on entry and truncates back to it on exit), so one
/// shared arena is correct. `gen_function`/`gen_program` reset the
/// counts to 0 before each function.
static mut LOOP_BRK: [u64; MAX_LOOP_JUMPS] = [0; MAX_LOOP_JUMPS];
static mut LOOP_CONT: [u64; MAX_LOOP_JUMPS] = [0; MAX_LOOP_JUMPS];
static mut LOOP_BRK_N: u64 = 0;
static mut LOOP_CONT_N: u64 = 0;

unsafe fn loop_reset() {
    unsafe {
        core::ptr::write(core::ptr::addr_of_mut!(LOOP_BRK_N), 0);
        core::ptr::write(core::ptr::addr_of_mut!(LOOP_CONT_N), 0);
    }
}
unsafe fn loop_brk_n() -> u64 { unsafe { core::ptr::read(core::ptr::addr_of!(LOOP_BRK_N)) } }
unsafe fn loop_cont_n() -> u64 { unsafe { core::ptr::read(core::ptr::addr_of!(LOOP_CONT_N)) } }
unsafe fn loop_set_brk_n(v: u64) { unsafe { core::ptr::write(core::ptr::addr_of_mut!(LOOP_BRK_N), v) } }
unsafe fn loop_set_cont_n(v: u64) { unsafe { core::ptr::write(core::ptr::addr_of_mut!(LOOP_CONT_N), v) } }
unsafe fn loop_brk_at(i: u64) -> u64 {
    unsafe { core::ptr::read((core::ptr::addr_of!(LOOP_BRK) as u64 + i * 8) as *const u64) }
}
unsafe fn loop_cont_at(i: u64) -> u64 {
    unsafe { core::ptr::read((core::ptr::addr_of!(LOOP_CONT) as u64 + i * 8) as *const u64) }
}
unsafe fn loop_push_brk(field: u64) -> Result<(), CodeGenError> {
    let n = unsafe { loop_brk_n() };
    if n >= MAX_LOOP_JUMPS as u64 {
        return Err(CodeGenError::TooManyLoopJumps);
    }
    unsafe { core::ptr::write((core::ptr::addr_of_mut!(LOOP_BRK) as u64 + n * 8) as *mut u64, field) };
    unsafe { loop_set_brk_n(n + 1) };
    Ok(())
}
unsafe fn loop_push_cont(field: u64) -> Result<(), CodeGenError> {
    let n = unsafe { loop_cont_n() };
    if n >= MAX_LOOP_JUMPS as u64 {
        return Err(CodeGenError::TooManyLoopJumps);
    }
    unsafe { core::ptr::write((core::ptr::addr_of_mut!(LOOP_CONT) as u64 + n * 8) as *mut u64, field) };
    unsafe { loop_set_cont_n(n + 1) };
    Ok(())
}

const MAX_VARS: u64 = 8;

#[derive(Clone, Copy)]
#[repr(C)]
struct VarSlot {
    name_off: u32,
    name_len: u8,
    stack_off: i32, // bytes from rbp, always negative
}

unsafe fn var_write(vars_ptr: u64, idx: u64, v: VarSlot) {
    unsafe { core::ptr::write((vars_ptr + idx * core::mem::size_of::<VarSlot>() as u64) as *mut VarSlot, v) };
}
unsafe fn var_read(vars_ptr: u64, idx: u64) -> VarSlot {
    unsafe { core::ptr::read((vars_ptr + idx * core::mem::size_of::<VarSlot>() as u64) as *const VarSlot) }
}

/// Real span-vs-span byte comparison (both spans read from the SAME
/// source buffer) -- the same "compare via pointer reads, never slice"
/// convention word_is() above already establishes, generalized past
/// word_is()'s "span vs. Rust literal" shape to "span vs. span" (needed
/// to compare a declared variable's name against a later IDENT
/// reference's name -- both real source spans, not compile-time
/// literals).
unsafe fn same_span(src_ptr: u64, a_off: u64, a_len: u64, b_off: u64, b_len: u64) -> bool {
    if a_len != b_len {
        return false;
    }
    let mut i: u64 = 0;
    while i < a_len {
        let a = unsafe { core::ptr::read((src_ptr + a_off + i) as *const u8) };
        let b = unsafe { core::ptr::read((src_ptr + b_off + i) as *const u8) };
        if a != b {
            return false;
        }
        i += 1;
    }
    true
}

/// Real walk of the STMT_DECL nodes in a function body's statement list,
/// assigning each declared variable its own 8-byte rbp-relative stack
/// slot in real source declaration order (first DECL gets rbp-8, second
/// rbp-16, ...). MILESTONE 70: now recurses into STMT_IF's own
/// then_body/else_body lists too, so a `int` declared inside an if/else
/// branch still gets a real stack slot -- a deliberate, disclosed
/// simplification, not full C block scoping: every variable in a
/// function shares ONE flat slot namespace for the function's whole
/// body regardless of which branch declared it (no shadowing, no
/// lifetime/visibility restriction to its own branch), the same
/// "flat, single-function, no nested scopes" model this subset already
/// used before if/else existed, now just reached via recursion instead
/// of a single linear walk. Returns the real total variable count on
/// success.
unsafe fn collect_vars(body_head: u64, vars_ptr: u64) -> Result<u64, CodeGenError> {
    let mut n: u64 = 0;
    unsafe { collect_vars_rec(body_head, vars_ptr, &mut n) }?;
    Ok(n)
}

unsafe fn collect_vars_rec(body_head: u64, vars_ptr: u64, n: &mut u64) -> Result<(), CodeGenError> {
    let mut cur = body_head;
    while cur != 0 {
        let s = unsafe { core::ptr::read(cur as *const StmtNode) };
        if s.kind == STMT_DECL {
            if *n >= MAX_VARS {
                return Err(CodeGenError::TooManyVars);
            }
            let off = -8 * (*n as i32 + 1);
            unsafe { var_write(vars_ptr, *n, VarSlot { name_off: s.ident_off, name_len: s.ident_len, stack_off: off }) };
            *n += 1;
        } else if s.kind == STMT_IF {
            // Recurse into both branches (else_body is 0, a real no-op
            // walk, when the source had no `else`).
            unsafe { collect_vars_rec(s.then_body, vars_ptr, n) }?;
            unsafe { collect_vars_rec(s.else_body, vars_ptr, n) }?;
        } else if s.kind == STMT_WHILE {
            // MILESTONE 71: same recursive-DECL-collection reasoning as
            // STMT_IF above, into the loop body (`then_body`).
            // MILESTONE 87: also walk `else_body` -- for a `for` loop it
            // now holds the `step` clause. That clause is an assignment
            // (no `int` decl), so this walk finds nothing today, but
            // keeping it symmetric with STMT_IF is correct, not lucky.
            unsafe { collect_vars_rec(s.then_body, vars_ptr, n) }?;
            unsafe { collect_vars_rec(s.else_body, vars_ptr, n) }?;
        }
        cur = s.next;
    }
    Ok(())
}

/// MILESTONE 72: the multi-function analogue of collect_vars() above --
/// used only by gen_program()'s own per-function codegen loop, NOT by the
/// original single-function gen_function() (which keeps calling
/// collect_vars() completely UNCHANGED, so CASE 1-22's own already-
/// verified codegen paths are untouched by this milestone). Seeds the
/// function's OWN parameters into `vars_ptr` FIRST, in real declaration
/// order (param 0 gets rbp-8, param 1 gets rbp-16, ...) -- exactly the
/// real x86_64 -O0 "spill each incoming argument register to its own
/// stack slot on entry" shape, independently checked against real
/// rustc/gcc output before writing this, the same discipline every prior
/// codegen milestone in this file used -- then continues into the SAME
/// collect_vars_rec() walk collect_vars() itself already uses for the
/// function's own local `int` declarations, which naturally continue the
/// slot numbering right after the last parameter (first local DECL lands
/// at rbp-8*(param_count+1)). A function's parameters and its locals
/// share the one flat slot namespace this whole codegen already uses for
/// locals alone -- consistent with, not a departure from, the "no nested
/// scopes" simplification collect_vars_rec()'s own doc comment above
/// already discloses.
unsafe fn collect_vars_for_function(f: &FuncDef, vars_ptr: u64) -> Result<u64, CodeGenError> {
    let mut n: u64 = 0;
    let mut pi: u64 = 0;
    while pi < f.param_count as u64 {
        if n >= MAX_VARS {
            return Err(CodeGenError::TooManyVars);
        }
        let p = unsafe { param_read(f.params_ptr, pi) };
        let off = -8 * (n as i32 + 1);
        unsafe { var_write(vars_ptr, n, VarSlot { name_off: p.name_off, name_len: p.name_len, stack_off: off }) };
        n += 1;
        pi += 1;
    }
    unsafe { collect_vars_rec(f.body, vars_ptr, &mut n) }?;
    Ok(n)
}

unsafe fn find_var(src_ptr: u64, vars_ptr: u64, nvars: u64, ident_off: u64, ident_len: u64) -> Option<i32> {
    let mut i: u64 = 0;
    while i < nvars {
        let v = unsafe { var_read(vars_ptr, i) };
        if unsafe { same_span(src_ptr, v.name_off as u64, v.name_len as u64, ident_off, ident_len) } {
            return Some(v.stack_off);
        }
        i += 1;
    }
    None
}

// MILESTONE 72: a function-symbol table -- one FuncSym per function in
// the program. MILESTONE 74: no longer built purely incrementally as
// each function is compiled -- gen_program() below now runs a real
// FIRST pass that registers every function's own name/param_count (with
// `code_off` starting at the real UNRESOLVED_CODE_OFF sentinel) before
// any body is compiled, so a call site anywhere in the program can
// resolve any OTHER function's name/param_count immediately regardless
// of source order; only `code_off` itself may still be unresolved at the
// moment a given call site is compiled (a genuine forward reference),
// handled by the real placeholder/patch-list mechanism documented at
// this file's own top Milestone 74 doc comment and at gen_program()
// below. Same "malloc()ed external buffer, raw-pointer read/write, no
// `[]` indexing on a runtime-variable index" convention VarSlot/
// var_write/var_read above already establish.
#[derive(Clone, Copy)]
#[repr(C)]
struct FuncSym {
    name_off: u32,
    name_len: u8,
    /// This function's own entry byte offset within the shared CodeBuf
    /// (i.e. `buf.len` at the moment its own prologue was emitted) --
    /// deliberately a BUFFER-relative offset, not an absolute address, so
    /// it is valid unchanged whether the buffer ends up called in-process
    /// (Callable mode, `buf.ptr + code_off`) or wrapped in a real ELF
    /// image whose code region begins right after a fixed-size header
    /// (Standalone mode, `ELF_LOAD_ADDR + header_len + code_off`) --
    /// exactly the same "relative offset, base cancels out in the rel32
    /// math" reasoning CodeBuf::emit_jmp_back()'s own doc comment above
    /// already relies on for a backward jump, generalized here to a call
    /// between two different functions instead of within one.
    /// MILESTONE 74: may also hold the real UNRESOLVED_CODE_OFF sentinel
    /// (this function's own name/param_count are registered, but its
    /// body has not been compiled yet) -- see that constant's own doc
    /// comment.
    code_off: u64,
    /// This function's own declared parameter count -- carried here so a
    /// CALL SITE can check real argument-count-vs-parameter-count
    /// agreement (CodeGenError::ArgCountMismatch) without a second,
    /// separate walk of the whole function list.
    param_count: u8,
}

unsafe fn funcsym_write(p: u64, idx: u64, v: FuncSym) {
    unsafe { core::ptr::write((p + idx * core::mem::size_of::<FuncSym>() as u64) as *mut FuncSym, v) };
}
unsafe fn funcsym_read(p: u64, idx: u64) -> FuncSym {
    unsafe { core::ptr::read((p + idx * core::mem::size_of::<FuncSym>() as u64) as *const FuncSym) }
}

/// Real span-vs-span lookup into the function-symbol table above -- the
/// exact same shape find_var() above already establishes for variables,
/// generalized to also return the callee's own declared parameter count
/// (needed by EXPR_CALL codegen's own real arity check, see gen_expr()
/// below) alongside its code offset. MILESTONE 74: also returns the
/// callee's own table INDEX (`i`) -- a genuinely new third element,
/// needed so a forward-reference call site (its `code_off` still the
/// UNRESOLVED_CODE_OFF sentinel) can record which table entry a later
/// backpatch pass should resolve against, since the real code_off itself
/// is exactly what is not yet known at the point this returns.
unsafe fn find_func(src_ptr: u64, funcs_ptr: u64, nfuncs: u64, ident_off: u64, ident_len: u64) -> Option<(u64, u8, u64)> {
    let mut i: u64 = 0;
    while i < nfuncs {
        let f = unsafe { funcsym_read(funcs_ptr, i) };
        if unsafe { same_span(src_ptr, f.name_off as u64, f.name_len as u64, ident_off, ident_len) } {
            return Some((f.code_off, f.param_count, i));
        }
        i += 1;
    }
    None
}

/// Real, growable-by-fixed-cap machine-code output buffer -- heap-
/// allocated via malloc() (NOT a stack array; see this file's own
/// Milestone 67 "real bug this milestone found and fixed in itself"
/// section above for exactly why stack buffers in THIS process are
/// dangerous -- this process still has exactly one 4KiB stack page).
/// Overflow is tracked via a real `overflowed` flag rather than letting
/// a `[]`-indexing panic happen, the same "no bounds-check panic path ->
/// no core::fmt pulled into this freestanding, panic=abort binary"
/// reason given at the top of this file.
struct CodeBuf {
    ptr: u64,
    len: u64,
    cap: u64,
    overflowed: bool,
}

impl CodeBuf {
    unsafe fn new(cap: u64) -> Self {
        CodeBuf { ptr: unsafe { malloc(cap) }, len: 0, cap, overflowed: false }
    }

    unsafe fn push(&mut self, b: u8) {
        if self.len >= self.cap {
            self.overflowed = true;
            return;
        }
        unsafe { core::ptr::write((self.ptr + self.len) as *mut u8, b) };
        self.len += 1;
    }

    unsafe fn push_bytes(&mut self, bytes: &[u8]) {
        for &b in bytes {
            unsafe { self.push(b) };
        }
    }

    // Real, individually hand-verified x86_64 encodings (checked byte by
    // byte against the Intel SDM's own encoding tables before being
    // written here -- see README.md's own Milestone 68 entry for the
    // worked-out derivation of each one):
    unsafe fn emit_prologue(&mut self, frame_bytes: u8) {
        unsafe { self.push(0x55) }; // push rbp
        unsafe { self.push_bytes(&[0x48, 0x89, 0xE5]) }; // mov rbp, rsp
        if frame_bytes > 0 {
            unsafe { self.push_bytes(&[0x48, 0x83, 0xEC, frame_bytes]) }; // sub rsp, imm8
        }
    }
    unsafe fn emit_mov_eax_imm32(&mut self, v: u32) {
        unsafe { self.push(0xB8) }; // mov eax, imm32 (zero-extends into rax)
        unsafe { self.push_bytes(&v.to_le_bytes()) };
    }
    unsafe fn emit_mov_rax_rbp_off(&mut self, off: i32) {
        unsafe { self.push_bytes(&[0x48, 0x8B, 0x45, off as i8 as u8]) }; // mov rax, [rbp+disp8]
    }
    unsafe fn emit_mov_rbp_off_rax(&mut self, off: i32) {
        unsafe { self.push_bytes(&[0x48, 0x89, 0x45, off as i8 as u8]) }; // mov [rbp+disp8], rax
    }
    unsafe fn emit_push_rax(&mut self) {
        unsafe { self.push(0x50) };
    }
    unsafe fn emit_pop_rax(&mut self) {
        unsafe { self.push(0x58) };
    }
    unsafe fn emit_mov_rcx_rax(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x89, 0xC1]) }; // mov rcx, rax
    }
    unsafe fn emit_add_rax_rcx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x01, 0xC8]) }; // add rax, rcx
    }
    unsafe fn emit_sub_rax_rcx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x29, 0xC8]) }; // sub rax, rcx
    }
    // MILESTONE 79: real bitwise-operator encodings -- AND/OR/XOR r/m64,
    // r64 are the SAME opcode family as ADD(0x01)/SUB(0x29)/CMP(0x39)
    // above (the classic x86 "ALU group": ADD=00/01, OR=08/09, ADC=10/11,
    // SBB=18/19, AND=20/21, SUB=28/29, XOR=30/31, CMP=38/39 -- the /r
    // "r/m,r" odd-numbered form each already uses), so these three are
    // the exact same REX.W + opcode + ModRM 0xC8 shape as
    // emit_add_rax_rcx()/emit_sub_rax_rcx()/emit_cmp_rax_rcx() above,
    // only the opcode byte changes -- individually hand-verified against
    // the Intel SDM's own encoding table before being written here, the
    // same discipline every other CodeBuf method in this file uses.
    unsafe fn emit_and_rax_rcx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x21, 0xC8]) }; // and rax, rcx
    }
    unsafe fn emit_or_rax_rcx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x09, 0xC8]) }; // or rax, rcx
    }
    unsafe fn emit_xor_rax_rcx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x31, 0xC8]) }; // xor rax, rcx
    }
    /// `NOT r/m64` -- the SAME F7 opcode-extension-digit group
    /// emit_neg_rax() below already uses (TEST=0, NOT=2, NEG=3, MUL=4,
    /// IMUL=5, DIV=6, IDIV=7 -- see that method's own doc comment for
    /// the full table), digit 2 instead of NEG's digit 3: ModRM
    /// 11 010 000 = 0xD0 (mod=11 register-direct, reg=010=NOT's own
    /// opcode-extension digit, rm=000=RAX).
    unsafe fn emit_not_rax(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0xF7, 0xD0]) }; // not rax
    }
    /// `SHL r/m64, CL` and `SAR r/m64, CL` -- x86's real "Shift Group 2"
    /// opcode D3 (shift-by-CL, as opposed to D1's shift-by-1 or C1's
    /// shift-by-immediate -- CL is the one this milestone needs, since
    /// the shift COUNT is itself a real runtime expression, not always a
    /// literal), opcode-extension digits ROL=0, ROR=1, RCL=2, RCR=3,
    /// SHL/SAL=4, SHR=5, SAR=7 (6 is unused). SAR (arithmetic, sign-
    /// preserving), not SHR (logical), is the real, deliberate choice
    /// for `>>` here -- this subset's one type is a signed 64-bit
    /// machine word (see gen_expr()'s own "value in RAX" postcondition
    /// doc), and real C's own `>>` on a signed operand is (as of C99)
    /// implementation-defined but universally arithmetic on every real
    /// x86_64 C compiler this toolchain's own output needs to agree
    /// with. ModRM for SHL: 11 100 000 = 0xE0 (reg=100). ModRM for SAR:
    /// 11 111 000 = 0xF8 (reg=111). Both operate on RAX in place, with
    /// the shift count implicitly read from CL (the low 8 bits of RCX)
    /// -- which is EXACTLY where gen_expr()'s own existing "right
    /// operand into RCX, left operand popped back into RAX" stack-
    /// machine sequence (already used by every other binary operator
    /// above) already leaves the shift count, so no extra register
    /// shuffling is needed for either operator.
    unsafe fn emit_shl_rax_cl(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0xD3, 0xE0]) }; // shl rax, cl
    }
    unsafe fn emit_sar_rax_cl(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0xD3, 0xF8]) }; // sar rax, cl
    }
    unsafe fn emit_imul_rax_rcx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x0F, 0xAF, 0xC1]) }; // imul rax, rcx
    }
    unsafe fn emit_cqo(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x99]) }; // cqo (sign-extend rax into rdx:rax)
    }
    unsafe fn emit_idiv_rcx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0xF7, 0xF9]) }; // idiv rcx (quotient -> rax)
    }
    unsafe fn emit_leave(&mut self) {
        unsafe { self.push(0xC9) };
    }
    unsafe fn emit_ret(&mut self) {
        unsafe { self.push(0xC3) };
    }
    // MILESTONE 69: the two additional real encodings STANDALONE mode
    // needs (see CodegenMode's own doc comment below) -- moving RAX's
    // value into RDI (the real x86_64 SysV first-argument register, and
    // this kernel's own real syscall-ABI first-argument register --
    // checked directly against libc.rs's sys_exit() wrapper, `in("rdi")
    // code`) then issuing this kernel's real `int 0x80` sys_exit (syscall
    // number 1, checked directly against libc.rs's own doc comment)
    // instead of `leave; ret` -- a standalone process entry point has no
    // caller to return to, so emitting `ret` there would pop whatever
    // garbage happens to sit at the top of this fresh process's own
    // stack into RIP, a real, avoided crash, not a hypothetical one.
    unsafe fn emit_mov_rdi_rax(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x89, 0xC7]) }; // mov rdi, rax
    }
    unsafe fn emit_int80(&mut self) {
        unsafe { self.push_bytes(&[0xCD, 0x80]) }; // int 0x80
    }

    // MILESTONE 70: real encodings for comparison-operator codegen and
    // real conditional/unconditional near jumps for if/else -- each
    // individually hand-verified against the Intel SDM's own encoding
    // tables before being written here, the same discipline Milestone
    // 68's own comment above this impl block already established.
    unsafe fn emit_cmp_rax_rcx(&mut self) {
        // CMP r/m64, r64 (opcode 0x39 /r); ModRM 0xC8 = 11 001 000 (reg
        // field = RCX(001), rm field = RAX(000)) -- same ModRM-byte
        // derivation as emit_sub_rax_rcx's own `sub rax, rcx` above
        // (opcode differs, operand encoding is identical).
        unsafe { self.push_bytes(&[0x48, 0x39, 0xC8]) };
    }
    /// `cc_opcode` is the real second byte of a two-byte SETcc encoding
    /// (0F 9x /r, ModRM 0xC0 selects the AL register for both the
    /// unused reg field and the rm field) -- callers pass one of the six
    /// real SETcc opcodes named at each gen_expr() call site below
    /// (0x94 sete, 0x95 setne, 0x9C setl, 0x9D setge, 0x9E setle, 0x9F
    /// setg), so this one real encoder covers all six comparison
    /// operators.
    unsafe fn emit_setcc(&mut self, cc_opcode: u8) {
        unsafe { self.push_bytes(&[0x0F, cc_opcode, 0xC0]) };
    }
    unsafe fn emit_movzx_eax_al(&mut self) {
        // MOVZX r32, r/m8 (0F B6 /r), ModRM 0xC0 = 11 000 000 (reg=EAX,
        // rm=AL). No REX.W prefix, deliberately -- a 32-bit destination
        // write on x86_64 implicitly zero-extends into the full 64-bit
        // register, the exact same "implicit zero-extend" property
        // emit_mov_eax_imm32()'s own doc comment above already relies on
        // for TOK_INTLIT, reused here rather than re-derived.
        unsafe { self.push_bytes(&[0x0F, 0xB6, 0xC0]) };
    }
    unsafe fn emit_test_rax_rax(&mut self) {
        // TEST r/m64, r64 (0x85 /r), ModRM 0xC0 = 11 000 000 (reg=RAX,
        // rm=RAX) -- `test rax, rax` sets ZF iff rax == 0, the real
        // x86_64 idiom for a truthiness test with no extra register and
        // no risk of overflow flags misleading a signed comparison the
        // way `cmp rax, 0` plus a signed Jcc could for edge values (moot
        // for this subset's own int range, used anyway as the honestly
        // correct primitive rather than the merely-adequate one).
        unsafe { self.push_bytes(&[0x48, 0x85, 0xC0]) };
    }
    /// Emits `jz rel32` (0F 84 <rel32>, 6 bytes total) with a real
    /// placeholder rel32 of 0, and returns the real byte offset (into
    /// this buffer) of that 4-byte rel32 FIELD -- not the instruction's
    /// own start -- for `patch_rel32()` below to fill in once the real
    /// jump target is known. The classic real two-pass-free "emit now,
    /// patch later" technique for a forward reference whose target
    /// hasn't been emitted yet: this subset's if/else bodies are always
    /// emitted AFTER the jz/jmp that needs to skip over or around them,
    /// so the target offset genuinely isn't known until gen_stmt_list()
    /// finishes emitting that body -- there is no way to know it up
    /// front in a single forward pass.
    unsafe fn emit_jz_placeholder(&mut self) -> u64 {
        unsafe { self.push_bytes(&[0x0F, 0x84, 0, 0, 0, 0]) };
        self.len - 4
    }
    /// Same real "emit now, patch later" shape as emit_jz_placeholder()
    /// above, for `jmp rel32` (0xE9 <rel32>, 5 bytes total) -- the
    /// unconditional jump an if WITH an else uses to skip over the else
    /// branch once the then branch finishes.
    unsafe fn emit_jmp_placeholder(&mut self) -> u64 {
        unsafe { self.push_bytes(&[0xE9, 0, 0, 0, 0]) };
        self.len - 4
    }
    /// Real forward-reference patch: writes the real rel32 displacement
    /// for a jz/jmp emitted earlier via emit_jz_placeholder()/
    /// emit_jmp_placeholder() (`field_off`, the offset THOSE returned),
    /// computed against `self.len` -- the buffer's CURRENT end, i.e. the
    /// real address the next instruction will be emitted at, which by
    /// construction (this function is always called at exactly the point
    /// where the jump's real target is about to be emitted) is the real
    /// jump target. x86_64 Jcc/JMP rel32 displacements are relative to
    /// the address of the NEXT instruction (i.e. the byte right after
    /// the 4-byte rel32 field itself, `field_off + 4`), not the jump
    /// opcode's own start -- checked directly against the Intel SDM's
    /// own "target := RIP + rel32, RIP already advanced past this
    /// instruction" definition before using `field_off + 4` (not
    /// `field_off`) as the base here. Uses `write_unaligned` rather than
    /// `core::ptr::write` since `field_off` is an arbitrary byte offset
    /// into a malloc()ed buffer with no `i32`-alignment guarantee.
    unsafe fn patch_rel32(&mut self, field_off: u64) {
        let field_end = field_off + 4;
        let target = self.len;
        let rel = (target as i64 - field_end as i64) as i32;
        unsafe { core::ptr::write_unaligned((self.ptr + field_off) as *mut i32, rel) };
    }

    /// MILESTONE 71: the one genuinely new real encoding this milestone
    /// needs -- an unconditional `jmp rel32` (same 0xE9 opcode
    /// emit_jmp_placeholder() above already uses for if/else's
    /// skip-the-else jump) whose target is emitted BACKWARD, i.e.
    /// already known at the point this is called, unlike every other
    /// jump in this file so far (all of which are FORWARD references,
    /// hence the separate emit-placeholder-then-patch_rel32() two-step
    /// those need). A while-loop's own "jump back to re-check the
    /// condition" is the first backward jump this codegen has ever
    /// emitted: `target` is `self.len` captured by the caller BEFORE the
    /// condition/test/jz/body were emitted, so by the time this runs it
    /// already points at real, already-written bytes -- no placeholder,
    /// no later patch call needed, the rel32 can be computed and written
    /// in one pass. Same real "displacement is relative to the address
    /// of the NEXT instruction, i.e. field_off + 4" rule patch_rel32()'s
    /// own doc comment above derives from the Intel SDM, just computed
    /// against a caller-supplied `target` instead of `self.len`.
    unsafe fn emit_jmp_back(&mut self, target: u64) {
        unsafe { self.push(0xE9) }; // jmp rel32
        let field_off = self.len;
        unsafe { self.push_bytes(&[0, 0, 0, 0]) };
        let field_end = field_off + 4;
        let rel = (target as i64 - field_end as i64) as i32;
        unsafe { core::ptr::write_unaligned((self.ptr + field_off) as *mut i32, rel) };
    }

    // MILESTONE 72: the three real encodings function parameters/calls
    // need, on top of everything Milestone 68-71 already built -- each
    // individually hand-verified against the Intel SDM's own encoding
    // tables before being written here, the same discipline every prior
    // CodeBuf method above already used.

    /// `mov [rbp+disp8], reg64` (REX.W 89 /r) -- the real prologue-time
    /// "spill an incoming argument register to its own stack slot" write.
    /// Same ModRM shape as emit_mov_rbp_off_rax() above (mod=01,
    /// rm=101=rbp, disp8 follows), generalized past RAX-only (reg field
    /// fixed at 000) to any of the four argument registers this
    /// milestone's own calling convention uses -- `reg` is the caller-
    /// supplied 3-bit register-field encoding (REG_RDI/REG_RSI/REG_RDX/
    /// REG_RCX below), placed into ModRM bits 3-5.
    unsafe fn emit_mov_rbp_off_reg(&mut self, off: i32, reg: u8) {
        unsafe { self.push_bytes(&[0x48, 0x89, 0x40 | (reg << 3) | 0x05, off as i8 as u8]) };
    }
    /// `pop reg64` (single-byte opcode 0x58+reg, no REX needed -- POP
    /// defaults to a full 64-bit operand in long mode, and every register
    /// this milestone ever pops into (RAX/RCX/RDX/RSI/RDI, register
    /// numbers 0-7) fits the single-byte 0x58-0x5F encoding with no
    /// REX.B extension required, unlike R8-R15). Used at a call site to
    /// move each already-pushed argument off the real hardware stack and
    /// into its own real SysV argument register, right before the `call`
    /// itself.
    unsafe fn emit_pop_reg(&mut self, reg: u8) {
        unsafe { self.push(0x58 + reg) };
    }
    /// `call rel32` (0xE8 <rel32>, 5 bytes) -- same real "displacement is
    /// relative to the address of the NEXT instruction" rule and the same
    /// single-pass "target is already known, no placeholder/patch needed"
    /// shape emit_jmp_back() above already established for a backward
    /// jump. MILESTONE 74: no longer the ONLY way this codegen ever calls
    /// a function -- still the real, unmodified fast path for a call to
    /// an already-compiled function (backward reference, or self; see
    /// gen_program()'s own updated doc comment below for the real,
    /// current source-order rule), reused unchanged here -- but a genuine
    /// forward reference now takes emit_call_placeholder()/
    /// patch_call_rel32() below instead. CALL additionally pushes its own
    /// return address before transferring control, real x86_64 hardware
    /// behavior this encoding relies on rather than emulates, and RET
    /// (already used by every non-entry function's own ordinary `leave;
    /// ret` epilogue, unchanged since Milestone 68) pops it back off on
    /// return.
    unsafe fn emit_call(&mut self, target: u64) {
        unsafe { self.push(0xE8) }; // call rel32
        let field_off = self.len;
        unsafe { self.push_bytes(&[0, 0, 0, 0]) };
        let field_end = field_off + 4;
        let rel = (target as i64 - field_end as i64) as i32;
        unsafe { core::ptr::write_unaligned((self.ptr + field_off) as *mut i32, rel) };
    }

    /// MILESTONE 74: `call rel32` (same 0xE8 opcode emit_call() above
    /// uses) with a real placeholder rel32 of 0, returning the real byte
    /// offset of that 4-byte rel32 FIELD -- the exact same "emit now,
    /// patch later" shape emit_jz_placeholder()/emit_jmp_placeholder()
    /// above already established for if/else's own forward jumps,
    /// generalized to CALL: used at a call site whose callee is a genuine
    /// forward reference (found in the function table by find_func(), but
    /// that entry's own code_off is still the real UNRESOLVED_CODE_OFF
    /// sentinel -- the callee hasn't been compiled yet, so its real
    /// target offset genuinely isn't known at this point in gen_program()'s
    /// own single left-to-right pass over the buffer).
    unsafe fn emit_call_placeholder(&mut self) -> u64 {
        unsafe { self.push_bytes(&[0xE8, 0, 0, 0, 0]) };
        self.len - 4
    }

    /// MILESTONE 74: the real backpatch step for a `call rel32` field
    /// emitted via emit_call_placeholder() above. Deliberately NOT
    /// patch_rel32() reused unchanged: patch_rel32() always computes its
    /// own target against `self.len` -- the buffer's CURRENT end -- which
    /// is correct for if/else's own jz/jmp (always patched at exactly the
    /// point their real target is about to be emitted, immediately
    /// after), but wrong here: a forward CALL's own real target (its
    /// callee's own code_off) was already emitted earlier in the buffer,
    /// at gen_program()'s own final backpatch pass, run once EVERY
    /// function's own code_off is final -- so `target` here is a real,
    /// CALLER-supplied absolute buffer offset (the callee's own resolved
    /// FuncSym::code_off), not `self.len`. Same real rel32 math as every
    /// other patch/emit function above: displacement is relative to the
    /// address of the byte right after the 4-byte field itself.
    unsafe fn patch_call_rel32(&mut self, field_off: u64, target: u64) {
        let field_end = field_off + 4;
        let rel = (target as i64 - field_end as i64) as i32;
        unsafe { core::ptr::write_unaligned((self.ptr + field_off) as *mut i32, rel) };
    }

    /// MILESTONE 76: `neg r/m64` (REX.W F7 /3) -- the one new real
    /// encoding unary minus needs. ModRM 0xD8 = 11 011 000: mod=11
    /// (register-direct operand), the middle 3 bits (011 = 3) are NOT a
    /// register selector here -- they're the real opcode-EXTENSION digit
    /// the Intel SDM's own F7 opcode table assigns to NEG (TEST=0, ---=1,
    /// NOT=2, NEG=3, MUL=4, IMUL=5, DIV=6, IDIV=7), the same "middle
    /// ModRM bits pick which F7 sub-opcode, not which register" shape
    /// emit_idiv_rcx() above already relies on for opcode digit 7; rm
    /// field = 000 = RAX, the one register this whole codegen ever
    /// operates on in place. Individually hand-verified against the
    /// Intel SDM's own encoding table before being written here, the same
    /// discipline every other CodeBuf method above already used.
    unsafe fn emit_neg_rax(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0xF7, 0xD8]) };
    }
}

// MILESTONE 72: the real x86_64 register-number encodings
// emit_mov_rbp_off_reg()/emit_pop_reg() above need -- the ordinary
// ModRM/opcode "reg" field values every x86_64 reference table uses
// (RAX=0, RCX=1, RDX=2, RBX=3, RSP=4, RBP=5, RSI=6, RDI=7), named here
// for exactly the four registers this milestone's own calling convention
// actually uses. This convention -- integer arguments 1-4 in RDI, RSI,
// RDX, RCX, in that order -- is not an arbitrary/new choice: checked
// directly against this kernel's own real syscall ABI (libc.rs's own
// sys_write/sys_open/sys_fdwrite wrappers, which already pass their own
// first three arguments via `in("rdi")`, `in("rsi")`, `in("rdx")`) before
// picking it, so a subset-C function call now uses the SAME real
// register ordering this kernel's syscall boundary already established,
// just extended one register further (RCX for a real 4th argument,
// genuine standard SysV x86_64 order) rather than inventing a new,
// unrelated one.
const REG_RCX: u8 = 1;
const REG_RDX: u8 = 2;
const REG_RSI: u8 = 6;
const REG_RDI: u8 = 7;

/// The real argument-register-by-position lookup this milestone's own
/// calling convention uses, both at a call site (after all arguments are
/// evaluated and pushed, popped back off in reverse into these) and in a
/// callee's own prologue (spilling each incoming register to its own
/// stack slot) -- ONE real mapping, used both directions, rather than two
/// independently-written tables that could silently drift apart.
unsafe fn param_reg(idx: u64) -> u8 {
    match idx {
        0 => REG_RDI,
        1 => REG_RSI,
        2 => REG_RDX,
        _ => REG_RCX,
    }
}

/// MILESTONE 69: which real epilogue shape gen_function() below emits.
/// `Callable` is Milestone 68's own original, UNCHANGED shape (`leave;
/// ret`) -- for code meant to be invoked through a plain Rust function
/// pointer from another function already running (the in-process
/// technique CASE 4/5/6/7 above still use). `Standalone` is Milestone
/// 69's new shape -- the generated code IS the whole process (its real
/// ELF e_entry, wrapped by build_elf64_standalone() below), so instead of
/// `leave; ret` (a real crash waiting to happen -- see emit_int80()'s own
/// doc comment above) it moves the function's result into RDI and issues
/// a real sys_exit(result) syscall, exactly the same syscall a normal
/// ring-3 program's own explicit `sys_exit()` call would issue.
#[derive(Clone, Copy, PartialEq)]
enum CodegenMode {
    Callable,
    Standalone,
}

unsafe fn emit_epilogue(buf: &mut CodeBuf, mode: CodegenMode) {
    match mode {
        CodegenMode::Callable => {
            unsafe { buf.emit_leave() };
            unsafe { buf.emit_ret() };
        }
        CodegenMode::Standalone => {
            unsafe { buf.emit_mov_rdi_rax() };
            unsafe { buf.emit_mov_eax_imm32(1) }; // sys_exit's real syscall number (libc.rs: "1 = exit(code)")
            unsafe { buf.emit_int80() };
        }
    }
}

/// Real recursive codegen over the SAME ExprNode tree Milestone 67's
/// parser built -- for every node, emits real x86_64 bytes that leave
/// the node's value in RAX once control reaches the next instruction.
/// BINARY nodes evaluate the left operand into RAX, PUSH it onto the
/// real stack, evaluate the right operand into RAX, MOV that into RCX,
/// POP the saved left operand back into RAX, then apply the real
/// two-register x86_64 op -- the standard "stack-machine" codegen shape
/// for a binary-operator AST, real register pressure handled by the
/// real hardware stack (PUSH/POP), not assumed away.
/// MILESTONE 72: two new parameters -- `funcs_ptr`/`nfuncs`, the real
/// function-symbol table gen_program() below builds incrementally as it
/// compiles each function -- threaded through so a call expression
/// anywhere in an expression tree can resolve its own callee. The
/// original single-function gen_function() (CASE 1-22's own codegen path,
/// UNCHANGED) always passes `(0, 0)` here -- an empty table -- since none
/// of those programs' source ever contains call syntax; `find_func()`
/// against an empty table simply never matches, so this is a real,
/// inert no-op for every pre-Milestone-72 program, not a behavior change.
/// MILESTONE 74: two MORE new parameters -- `pending_ptr`/
/// `pending_count_ptr`, the real forward-call patch list gen_program()
/// below allocates once per program and threads through unchanged, the
/// same "external malloc()ed buffer + explicit count, no hidden global
/// state" convention every other cross-cutting piece of state in this
/// file (funcs_ptr/nfuncs, vars_ptr/nvars) already uses. `pending_count_ptr`
/// is a pointer to a single real u64 CELL (not a plain count-by-value
/// parameter) specifically because EXPR_CALL below must be able to
/// durably INCREMENT it across however many sibling/nested gen_expr()
/// calls happen to run after it in the same function body -- the same
/// real reason funcs_ptr/vars_ptr are themselves pointers into
/// externally-owned storage rather than by-value snapshots. Both are `0`
/// for the original single-function gen_function() path (CASE 1-22,
/// UNCHANGED) -- exactly as inert as funcs_ptr=0/nfuncs=0 already are
/// there, since a program with an empty function table can never reach
/// the EXPR_CALL arm that would dereference them.
unsafe fn gen_expr(buf: &mut CodeBuf, src_ptr: u64, vars_ptr: u64, nvars: u64, funcs_ptr: u64, nfuncs: u64, pending_ptr: u64, pending_count_ptr: u64, node: u64) -> Result<(), CodeGenError> {
    let e = unsafe { core::ptr::read(node as *const ExprNode) };
    match e.kind {
        EXPR_INTLIT => unsafe { buf.emit_mov_eax_imm32(e.int_val as u32) },
        EXPR_IDENT => {
            let off = unsafe { find_var(src_ptr, vars_ptr, nvars, e.ident_off as u64, e.ident_len as u64) };
            match off {
                Some(o) => unsafe { buf.emit_mov_rax_rbp_off(o) },
                None => return Err(CodeGenError::UndeclaredVariable(e.ident_off as u64)),
            }
        }
        // MILESTONE 72: a call expression's own real codegen -- evaluate
        // every argument left-to-right into RAX, PUSHing each one
        // immediately after (the exact same "stack machine" technique
        // EXPR_BINARY already uses for its own two operands just below,
        // generalized past two operands to however many this call has),
        // THEN pop them back off, in REVERSE order, into the real SysV
        // integer argument registers (RDI, RSI, RDX, RCX, in that order --
        // see param_reg()'s own doc comment above for why this exact
        // ordering was chosen). Popping in reverse is not incidental: the
        // stack is LIFO, so the LAST-pushed (highest-index) argument comes
        // off FIRST -- this loop counts DOWN from the last argument index
        // to the first, assigning each just-popped value to the register
        // matching ITS OWN original argument index (not the pop order),
        // which exactly undoes the push order above and lands every
        // argument in the register a real callee's own prologue (see
        // gen_program() below) expects it in. Finally, a real `call
        // rel32` to the callee -- MILESTONE 74: no longer necessarily to
        // an "already-known" code offset. gen_program() below now runs a
        // real first pass that registers every function's own name/
        // param_count before any body is compiled, so find_func() finds a
        // real match here regardless of source order; what it returns as
        // `code_off` may still be the real UNRESOLVED_CODE_OFF sentinel
        // (a genuine forward reference -- the callee hasn't been
        // compiled yet). If so, this emits a real placeholder instead
        // (emit_call_placeholder()) and records a pending-patch entry
        // (this field's own offset, the callee's own table index) for
        // gen_program()'s own final backpatch pass to resolve once every
        // function's own code_off is final -- otherwise (the callee is
        // already compiled, backward reference or self), the exact
        // original single-instruction emit_call() path runs, unchanged.
        // On return, RAX already holds the callee's own result (its own
        // RETURN statement's gen_expr() call already left it there before
        // that function's epilogue ran) -- the same "gen_expr()'s own
        // postcondition is always value-in-RAX" invariant this whole
        // function already guarantees for every OTHER node kind, so a
        // call expression composes with the rest of this codegen (as an
        // operand of arithmetic, as another call's own argument, etc.)
        // with zero special-casing anywhere else.
        EXPR_CALL => {
            let target = unsafe { find_func(src_ptr, funcs_ptr, nfuncs, e.ident_off as u64, e.ident_len as u64) };
            let (code_off, callee_param_count, callee_idx) = match target {
                Some(t) => t,
                None => return Err(CodeGenError::UndeclaredFunction(e.ident_off as u64)),
            };
            if e.call_argc != callee_param_count {
                return Err(CodeGenError::ArgCountMismatch(e.ident_off as u64));
            }
            let mut ai: u64 = 0;
            while ai < e.call_argc as u64 {
                let arg_node = unsafe { call_arg_read(e.call_args_ptr, ai) };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, arg_node) }?;
                unsafe { buf.emit_push_rax() };
                ai += 1;
            }
            let mut ai_rev: i64 = e.call_argc as i64 - 1;
            while ai_rev >= 0 {
                let reg = unsafe { param_reg(ai_rev as u64) };
                unsafe { buf.emit_pop_reg(reg) };
                ai_rev -= 1;
            }
            if code_off == UNRESOLVED_CODE_OFF {
                // MILESTONE 74: genuine forward reference -- emit a real
                // placeholder now, resolve it later.
                let field_off = unsafe { buf.emit_call_placeholder() };
                let cnt = unsafe { core::ptr::read(pending_count_ptr as *const u64) };
                if cnt >= MAX_PENDING_CALLS {
                    return Err(CodeGenError::TooManyForwardCalls);
                }
                unsafe { pending_write(pending_ptr, cnt, PendingCall { field_off, func_idx: callee_idx }) };
                unsafe { core::ptr::write(pending_count_ptr as *mut u64, cnt + 1) };
            } else {
                unsafe { buf.emit_call(code_off) };
            }
        }
        // MILESTONE 76: unary minus -- evaluate the one operand into RAX
        // (completely ordinary gen_expr() recursion, same postcondition
        // as every other node kind), then negate it in place. One real
        // new encoding (emit_neg_rax(), CodeBuf's own doc comment above
        // has the full derivation), no jump/branch machinery needed --
        // unlike `&&`/`||` just below, unary minus has no
        // short-circuiting concern at all, its one operand is always
        // evaluated.
        // MILESTONE 79: extended for real unary `~` (bitwise NOT) --
        // same "evaluate the one operand into RAX, then transform it in
        // place, no jump machinery needed" shape unary minus already
        // established; only which real one-operand encoding runs
        // differs (emit_not_rax() vs. emit_neg_rax()).
        // MILESTONE 83: extended for real logical `!` -- still the same
        // "evaluate the one operand into RAX, transform in place, no
        // jump machinery" shape, but `!` is a BOOLEAN-producing operator
        // (result is exactly 0 or 1), so instead of an F7-family in-place
        // bit transform it reuses the exact `test rax,rax` + `setcc` +
        // `movzx eax,al` normalize idiom OP_NE and every comparison arm
        // already use -- here with `sete` (0x94: AL := 1 iff ZF, i.e.
        // iff the operand was 0), zero new CodeBuf encodings.
        EXPR_UNARY => {
            unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.left) }?;
            if e.op == OP_BITNOT {
                unsafe { buf.emit_not_rax() };
            } else if e.op == OP_LOGNOT {
                unsafe { buf.emit_test_rax_rax() };
                unsafe { buf.emit_setcc(0x94) }; // sete al -- AL := (operand == 0)
                unsafe { buf.emit_movzx_eax_al() };
            } else {
                unsafe { buf.emit_neg_rax() };
            }
        }
        EXPR_BINARY => {
            // MILESTONE 76: `&&`/`||` need real SHORT-CIRCUIT codegen --
            // the right operand must NOT be evaluated at all once the
            // left operand already determines the result (left-false for
            // `&&`, left-true for `||`). Real C semantics, not a
            // cosmetic nicety: this subset's function calls are the one
            // place evaluating an expression has any externally
            // observable cost (a real call, real stack use, in the
            // extreme even the real recursion this file's own Milestone
            // 75 stack-overflow-safety work exists because of) -- a
            // right operand containing a call must genuinely not run
            // when short-circuited, the same way real gcc/rustc-compiled
            // C already behaves, checked against the C standard's own
            // "&&/|| are sequence points; the second operand is
            // evaluated only if needed" rule before writing this, not
            // guessed. Faking this with an unconditional-evaluate
            // bitwise AND/OR (both operands always evaluated, then
            // combined) would be a real, silent, wrong-for-real-programs
            // semantic bug, not merely a missed optimization -- the same
            // "implement it for real, not faked" discipline this file's
            // own if/else and while codegen already established for
            // every other real branch. Both operators below reuse the
            // EXACT SAME emit_jz_placeholder()/emit_jmp_placeholder()/
            // patch_rel32() forward-patch machinery Milestone 70/71
            // already built (STMT_IF/STMT_WHILE's own codegen in
            // gen_stmt_list() below) -- no new CodeBuf jump encoding
            // needed for either operator, only this file's existing
            // `setne al` + `movzx eax, al` boolean-normalize idiom
            // (already used by OP_NE above) to make sure the FINAL result
            // is always a clean 0/1, exactly real C's own `&&`/`||`
            // result type, not either operand's raw value (`1 && 5` is
            // `1`, not `5`).
            if e.op == OP_LOGAND {
                // a && b:
                //   <a>                  (gen_expr, into RAX)
                //   test rax, rax
                //   jz false_label        (forward-patched -- a is
                //                          false: skip b ENTIRELY, the
                //                          real point of this shape)
                //   <b>                  (gen_expr, into RAX -- only
                //                          reached when a was true)
                //   test rax, rax
                //   setne al              (normalize b's raw value to a
                //                          clean boolean 0/1)
                //   movzx eax, al
                //   jmp end_label         (forward-patched)
                // false_label:
                //   mov eax, 0
                // end_label:
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.left) }?;
                unsafe { buf.emit_test_rax_rax() };
                let jz_field = unsafe { buf.emit_jz_placeholder() };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.right) }?;
                unsafe { buf.emit_test_rax_rax() };
                unsafe { buf.emit_setcc(0x95) }; // setne
                unsafe { buf.emit_movzx_eax_al() };
                let jmp_field = unsafe { buf.emit_jmp_placeholder() };
                unsafe { buf.patch_rel32(jz_field) };
                unsafe { buf.emit_mov_eax_imm32(0) };
                unsafe { buf.patch_rel32(jmp_field) };
            } else if e.op == OP_LOGOR {
                // a || b: the mirror-image shape -- a TRUE left operand
                // short-circuits straight to result 1, b never runs.
                //   <a>
                //   test rax, rax
                //   jz check_b            (forward-patched -- a is
                //                          false: b must be checked)
                //   mov eax, 1             (a was true -- result 1, skip
                //                          b entirely)
                //   jmp end_label         (forward-patched)
                // check_b:
                //   <b>                  (only reached when a was false)
                //   test rax, rax
                //   setne al
                //   movzx eax, al
                // end_label:
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.left) }?;
                unsafe { buf.emit_test_rax_rax() };
                let jz_field = unsafe { buf.emit_jz_placeholder() };
                unsafe { buf.emit_mov_eax_imm32(1) };
                let jmp_field = unsafe { buf.emit_jmp_placeholder() };
                unsafe { buf.patch_rel32(jz_field) };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.right) }?;
                unsafe { buf.emit_test_rax_rax() };
                unsafe { buf.emit_setcc(0x95) }; // setne
                unsafe { buf.emit_movzx_eax_al() };
                unsafe { buf.patch_rel32(jmp_field) };
            } else {
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.left) }?;
                unsafe { buf.emit_push_rax() };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.right) }?;
                unsafe { buf.emit_mov_rcx_rax() };
                unsafe { buf.emit_pop_rax() };
                match e.op {
                    b'+' => unsafe { buf.emit_add_rax_rcx() },
                    b'-' => unsafe { buf.emit_sub_rax_rcx() },
                    b'*' => unsafe { buf.emit_imul_rax_rcx() },
                    b'/' => {
                        unsafe { buf.emit_cqo() };
                        unsafe { buf.emit_idiv_rcx() };
                    }
                    // MILESTONE 79: the five new binary bitwise operators.
                    // AND/OR/XOR reuse this exact same left-in-rax/
                    // right-in-rcx convention every arithmetic op above
                    // already established -- one real instruction each,
                    // no extra setup. SHL/SAR need the shift COUNT in CL
                    // specifically (x86's own real ABI requirement for
                    // shift-by-register) -- which right operand's value
                    // already IS, since it's sitting in RCX from this
                    // same preamble (emit_mov_rcx_rax() a few lines
                    // above): CL is simply RCX's own low 8 bits, so no
                    // additional register move is needed for either
                    // shift operator, a genuinely free fit, not
                    // engineered to look that way.
                    OP_BITAND => unsafe { buf.emit_and_rax_rcx() },
                    OP_BITOR => unsafe { buf.emit_or_rax_rcx() },
                    OP_BITXOR => unsafe { buf.emit_xor_rax_rcx() },
                    OP_SHL => unsafe { buf.emit_shl_rax_cl() },
                    OP_SHR => unsafe { buf.emit_sar_rax_cl() },
                    // MILESTONE 70: the six comparison operators, each real
                    // `cmp rax, rcx` (left vs. right, exactly the same
                    // left-in-rax/right-in-rcx order every arithmetic op
                    // above already relies on) followed by the ONE real
                    // SETcc opcode that means that operator, then a real
                    // zero-extend so the whole 64-bit RAX holds a clean 0/1
                    // rather than a stale upper 56 bits from whatever was in
                    // RAX before -- SETcc only ever writes the low 8 bits.
                    OP_EQ => {
                        unsafe { buf.emit_cmp_rax_rcx() };
                        unsafe { buf.emit_setcc(0x94) }; // sete
                        unsafe { buf.emit_movzx_eax_al() };
                    }
                    OP_NE => {
                        unsafe { buf.emit_cmp_rax_rcx() };
                        unsafe { buf.emit_setcc(0x95) }; // setne
                        unsafe { buf.emit_movzx_eax_al() };
                    }
                    OP_LT => {
                        unsafe { buf.emit_cmp_rax_rcx() };
                        unsafe { buf.emit_setcc(0x9C) }; // setl
                        unsafe { buf.emit_movzx_eax_al() };
                    }
                    OP_GT => {
                        unsafe { buf.emit_cmp_rax_rcx() };
                        unsafe { buf.emit_setcc(0x9F) }; // setg
                        unsafe { buf.emit_movzx_eax_al() };
                    }
                    OP_LE => {
                        unsafe { buf.emit_cmp_rax_rcx() };
                        unsafe { buf.emit_setcc(0x9E) }; // setle
                        unsafe { buf.emit_movzx_eax_al() };
                    }
                    _ => {
                        // Every op byte parse_term()/parse_expr()/
                        // parse_cond_expr() can ever set is now covered
                        // by an explicit arm above except OP_GE -- kept
                        // as the fallback (rather than adding an
                        // eleventh, purely mechanical arm) the same way
                        // Milestone 68's own original match used a
                        // fallback for '/'. OP_LOGAND/OP_LOGOR can never
                        // reach this inner match at all -- both are
                        // handled by the two `if`/`else if` arms above,
                        // before this `else` block's own preamble
                        // (unconditional evaluate-both-operands) ever
                        // runs.
                        unsafe { buf.emit_cmp_rax_rcx() };
                        unsafe { buf.emit_setcc(0x9D) }; // setge
                        unsafe { buf.emit_movzx_eax_al() };
                    }
                }
            }
        }
        _ => {}
    }
    Ok(())
}

/// Real top-level codegen: walks a FuncDef's statement list in real
/// source order, emitting a real x86_64 function -- ordinary SysV leaf-
/// function prologue, then body, then a real epilogue whose SHAPE depends
/// on `mode` (see CodegenMode's own doc comment): `Callable` emits `leave;
/// ret` so the caller (CASE 4/5/6/7 below) can invoke the resulting bytes
/// through a plain Rust function pointer with zero special-casing, exactly
/// Milestone 68's own original behavior, UNCHANGED; `Standalone`
/// (MILESTONE 69, new) emits a real sys_exit(result) sequence instead, so
/// the resulting bytes are directly usable as a whole process's own
/// e_entry once wrapped by build_elf64_standalone() below. DECL reserves a
/// stack slot (already assigned by collect_vars() before this runs) and
/// emits no code of its own; ASSIGN evaluates its expression and stores
/// the result; RETURN evaluates its expression into RAX and emits the real
/// mode-appropriate epilogue. A trailing epilogue (same mode-appropriate
/// shape) is ALWAYS appended after the last statement as a real safety
/// backstop against falling off the end of the buffer for a program whose
/// body never reaches an explicit `return` (this subset's grammar does not
/// require or enforce exactly-one, last-position `return` -- a real,
/// disclosed gap, not hidden: see README.md's own Milestone 68 "still
/// genuinely open" section, still true here).
/// MILESTONE 70: real recursive codegen over one StmtNode list -- the
/// walk gen_function() below used to inline directly (Milestone 68's
/// own original shape), pulled out into its own function so an
/// if/else's then_body/else_body lists can be emitted through the exact
/// same real DECL/ASSIGN/RETURN handling as a function's own top-level
/// body, and so STMT_IF's own codegen can recurse into its branches.
///
/// STMT_IF's real codegen shape: evaluate the condition into RAX (via
/// gen_expr(), completely unchanged -- ANY expression is a legal
/// condition, not just a comparison, matching real C's own "if tests
/// truthiness" semantics rather than requiring a relational operator),
/// `test rax, rax` to set ZF from it, then a real forward-patched `jz`
/// (see emit_jz_placeholder()/patch_rel32() above for the real
/// mechanism) to the branch to take when the condition is FALSE. With
/// no else: that jz's own target is simply wherever code resumes after
/// the then-block, i.e. patched once the then-block itself has been
/// emitted. With an else: the then-block ends with its own forward-
/// patched unconditional `jmp` (skipping the else-block once the
/// then-block has already run), the jz is patched to the else-block's
/// real start, the else-block is emitted, and finally the jmp is
/// patched to the real address right after it -- the standard real
/// "if/else via one conditional branch plus one unconditional branch"
/// shape, not guessed at: independently checked against how rustc/gcc's
/// own -O0 output shapes an equivalent if/else before writing this.
/// MILESTONE 72: two new parameters -- `funcs_ptr`/`nfuncs`, threaded
/// through to every gen_expr() call and every recursive gen_stmt_list()
/// call below for exactly the same reason gen_expr()'s own doc comment
/// above gives (the original single-function gen_function() path passes
/// `(0, 0)`, a real, inert no-op for every pre-Milestone-72 program).
/// MILESTONE 74: two MORE new parameters -- `pending_ptr`/
/// `pending_count_ptr`, threaded through the exact same way, to the exact
/// same functions, for the exact same reason -- see gen_expr()'s own
/// updated doc comment above for the full real reasoning.
unsafe fn gen_stmt_list(
    buf: &mut CodeBuf,
    src_ptr: u64,
    vars_ptr: u64,
    nvars: u64,
    funcs_ptr: u64,
    nfuncs: u64,
    pending_ptr: u64,
    pending_count_ptr: u64,
    mode: CodegenMode,
    head: u64,
    // MILESTONE 87: the INNERMOST enclosing loop's condition-re-check
    // offset (== NOT_IN_LOOP when this list is not inside any loop body),
    // and 1 if that loop is a `for` (whose `continue` must run the step
    // clause). Threaded UNCHANGED through STMT_IF's branch recursion (a
    // `break` inside an `if` inside a `while` still binds to that
    // `while`); replaced by STMT_WHILE for its own body; passed as
    // (NOT_IN_LOOP, 0) for the `for` step clause. The break/continue
    // patch lists themselves live in module `static mut` arenas
    // (LOOP_BRK/LOOP_CONT) with a save/restore-of-count discipline in
    // STMT_WHILE -- see their own doc comment for why not per-loop
    // stack/heap.
    loop_top: u64,
    loop_is_for: u64,
) -> Result<(), CodeGenError> {
    let mut cur = head;
    while cur != 0 {
        let s = unsafe { core::ptr::read(cur as *const StmtNode) };
        match s.kind {
            STMT_DECL => {}
            STMT_ASSIGN => {
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                let off = unsafe { find_var(src_ptr, vars_ptr, nvars, s.ident_off as u64, s.ident_len as u64) };
                match off {
                    Some(o) => unsafe { buf.emit_mov_rbp_off_rax(o) },
                    None => return Err(CodeGenError::UndeclaredVariable(s.ident_off as u64)),
                }
            }
            STMT_RETURN => {
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                unsafe { emit_epilogue(buf, mode) };
            }
            // MILESTONE 87: `break` -- an unconditional forward jump to
            // the innermost loop's exit, recorded as a placeholder in
            // that loop's own patch list (resolved by STMT_WHILE once the
            // exit address is known). Outside any loop -> a real semantic
            // error, the same shape as UndeclaredVariable.
            STMT_BREAK => {
                if loop_top == NOT_IN_LOOP {
                    return Err(CodeGenError::BreakOutsideLoop);
                }
                let field = unsafe { buf.emit_jmp_placeholder() };
                unsafe { loop_push_brk(field) }?;
            }
            // MILESTONE 87: `continue` -- for a plain `while` this is a
            // BACKWARD jump straight to the condition re-check (target
            // already known). For a `for` loop the next iteration must
            // run the `step` clause first, so it is a FORWARD jump into
            // that loop's `cont` patch list, resolved by STMT_WHILE to
            // the point right before the step's own codegen.
            STMT_CONTINUE => {
                if loop_top == NOT_IN_LOOP {
                    return Err(CodeGenError::ContinueOutsideLoop);
                }
                if loop_is_for == 0 {
                    // plain `while` -- straight back to the condition.
                    unsafe { buf.emit_jmp_back(loop_top) };
                } else {
                    // `for` -- forward jump to just before the step clause,
                    // resolved by STMT_WHILE below.
                    let field = unsafe { buf.emit_jmp_placeholder() };
                    unsafe { loop_push_cont(field) }?;
                }
            }
            STMT_IF => {
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                unsafe { buf.emit_test_rax_rax() };
                let jz_field = unsafe { buf.emit_jz_placeholder() };
                unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.then_body, loop_top, loop_is_for) }?;
                if s.else_body != 0 {
                    let jmp_field = unsafe { buf.emit_jmp_placeholder() };
                    unsafe { buf.patch_rel32(jz_field) };
                    unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.else_body, loop_top, loop_is_for) }?;
                    unsafe { buf.patch_rel32(jmp_field) };
                } else {
                    unsafe { buf.patch_rel32(jz_field) };
                }
            }
            // MILESTONE 71: STMT_WHILE's real codegen shape --
            // condition-at-top, the standard real "while" lowering
            // (checked against rustc/gcc -O0 output the same way
            // STMT_IF's own doc comment above already did for if/else):
            //   loop_top:  <condition>            (gen_expr, into RAX)
            //              test rax, rax
            //              jz exit                (forward-patched, same
            //                                       machinery as if/else)
            //              <body>                 (gen_stmt_list,
            //                                       recursing into
            //                                       then_body exactly
            //                                       like STMT_IF's own
            //                                       branches do)
            //              jmp loop_top            (BACKWARD, target
            //                                       already known --
            //                                       emit_jmp_back())
            //   exit:
            // `loop_top` is captured as `buf.len` BEFORE the condition is
            // emitted (not after) so each iteration genuinely re-runs the
            // condition, not a stale cached truthiness -- real C `while`
            // semantics, condition checked every time including the
            // first, zero iterations when it starts false.
            STMT_WHILE => {
                let my_top = buf.len;
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                unsafe { buf.emit_test_rax_rax() };
                let jz_field = unsafe { buf.emit_jz_placeholder() };

                // MILESTONE 87: `s.else_body` carries a `for` loop's step
                // clause (0 for a plain `while`); its presence is what
                // makes this loop's `continue` a forward jump.
                let my_is_for: u64 = if s.else_body != 0 { 1 } else { 0 };
                // Save the shared arena counts on entry; every break /
                // continue this loop's body records goes at index
                // >= these bases; on exit we patch [base .. now] and
                // truncate back so an enclosing loop is unaffected.
                let brk_base = unsafe { loop_brk_n() };
                let cont_base = unsafe { loop_cont_n() };

                unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.then_body, my_top, my_is_for) }?;

                if my_is_for != 0 {
                    // `continue`'s forward-jump target is HERE, right
                    // before the step clause's own codegen.
                    let cn = unsafe { loop_cont_n() };
                    let mut i = cont_base;
                    while i < cn {
                        unsafe { buf.patch_rel32(loop_cont_at(i)) };
                        i += 1;
                    }
                    unsafe { loop_set_cont_n(cont_base) };
                    unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.else_body, NOT_IN_LOOP, 0) }?;
                }

                unsafe { buf.emit_jmp_back(my_top) };

                // exit: the `jz` (condition false) and every `break` land here.
                unsafe { buf.patch_rel32(jz_field) };
                let bn = unsafe { loop_brk_n() };
                let mut j = brk_base;
                while j < bn {
                    unsafe { buf.patch_rel32(loop_brk_at(j)) };
                    j += 1;
                }
                unsafe { loop_set_brk_n(brk_base) };
            }
            _ => {}
        }
        cur = s.next;
    }
    Ok(())
}

unsafe fn gen_function(src_ptr: u64, func: u64, mode: CodegenMode) -> Result<CodeBuf, CodeGenError> {
    let f = unsafe { core::ptr::read(func as *const FuncDef) };

    let vars_ptr = unsafe { malloc(MAX_VARS * core::mem::size_of::<VarSlot>() as u64) };
    let nvars = unsafe { collect_vars(f.body, vars_ptr) }?;
    let raw_frame = nvars * 8;
    let frame_bytes = if raw_frame == 0 { 0u8 } else { (((raw_frame + 15) / 16) * 16) as u8 };

    // MILESTONE 70: bumped from Milestone 68's original 512 to 1024 --
    // real headroom for if/else's own extra bytes (test+jz is 9 bytes,
    // jmp is 5, on top of whatever the branch bodies themselves emit);
    // still comfortably small (one CodeBuf here is at most 1 KiB, well
    // under fs::MAX_FILE_BYTES once wrapped in an ELF image by
    // build_elf64_standalone() below), and buf.overflowed still catches
    // it for real if any future test ever needs more.
    let mut buf = unsafe { CodeBuf::new(1024) };
    unsafe { buf.emit_prologue(frame_bytes) };

    // MILESTONE 72: `(0, 0)` -- an empty, real function-symbol table --
    // see gen_expr()/gen_stmt_list()'s own doc comments above for exactly
    // why that's a real, inert no-op for this single-function path,
    // completely UNCHANGED from Milestone 68-71's own behavior. MILESTONE
    // 74: `(0, 0)` again for the two new pending-call-list parameters,
    // the exact same real inert-no-op reasoning -- a program with an
    // empty function table can never reach the EXPR_CALL arm that would
    // dereference them.
    unsafe { loop_reset() }; // MILESTONE 87: fresh break/continue arena per function
    unsafe { gen_stmt_list(&mut buf, src_ptr, vars_ptr, nvars, 0, 0, 0, 0, mode, f.body, NOT_IN_LOOP, 0) }?;

    // Safety backstop -- see this function's own doc comment above.
    unsafe { emit_epilogue(&mut buf, mode) };

    if buf.overflowed {
        return Err(CodeGenError::BufferFull);
    }
    Ok(buf)
}

/// MILESTONE 72: the real multi-function codegen entry point -- compiles
/// an entire `program := function+` FuncDef list (parse_program()'s own
/// output) into ONE shared CodeBuf, and returns that buffer together with
/// the real byte offset (WITHIN that buffer) where the function named
/// "main" begins -- callers need that offset because, unlike
/// gen_function()'s own single-function output, `buf.ptr` itself is no
/// longer guaranteed to BE the entry point once more than one function
/// exists in the source (the entry function might not be the first one
/// compiled).
///
/// **MILESTONE 74: the real dependency-order scope cut Milestone 72/73
/// both disclosed is now LIFTED.** Through Milestone 73, a function could
/// only be called by a function compiled LATER in this same loop, or by
/// itself -- every callee had to be defined at or before its own caller
/// in source order, because `emit_call()`'s own `call rel32` encoding
/// computed and wrote its displacement in a single pass against an
/// already-known target offset, with no forward-reference placeholder/
/// patch step. This function now runs a real FIRST pass over the whole
/// function list that registers every function's own name/param_count
/// into `funcs_ptr` (with `code_off` starting at the real
/// UNRESOLVED_CODE_OFF sentinel) before any body is compiled at all --
/// so a call site anywhere in the program resolves any OTHER function's
/// name/param_count immediately, regardless of source order. The
/// original per-function loop (SECOND pass, below) still overwrites each
/// function's OWN table entry with its real `code_off` immediately
/// before that function's OWN body is compiled -- the exact same
/// ordering Milestone 73 verified makes direct self-recursion resolvable
/// (own entry real before own body compiles), now also true, from the
/// very start of pass two, for every OTHER function in the program: a
/// call to an already-compiled function (backward reference, or self)
/// still takes gen_expr()'s own original single-instruction `emit_call()`
/// path, completely unchanged. A call to a function *not yet compiled*
/// (a genuine forward reference -- its table entry's own `code_off` is
/// still the unresolved sentinel) instead emits a real placeholder
/// (`CodeBuf::emit_call_placeholder()`) and records a pending-patch
/// entry in a new, small, fixed-cap list (`MAX_PENDING_CALLS`) -- the
/// exact same "emit now, patch later" technique
/// `emit_jz_placeholder()`/`emit_jmp_placeholder()` already established
/// for if/else's own forward branches (Milestone 70), generalized here
/// to CALL. Once this loop finishes (every function's own real code_off
/// is now final, full stop), a new, final backpatch pass below walks the
/// pending list and writes each placeholder's real rel32 displacement
/// against its own callee's now-resolved code_off
/// (`CodeBuf::patch_call_rel32()`).
///
/// This is real, general MUTUAL recursion, not just forward calls in
/// isolation: function A calling a not-yet-compiled function B (a
/// forward reference, patched after the loop) and B in turn calling A
/// (a backward reference, resolved immediately since A's own code_off
/// was already written before A's own body -- which contains the call to
/// B -- was compiled) both resolve correctly, because the final backpatch
/// pass runs only once EVERY function in the program has been compiled
/// and every code_off is simultaneously final -- there is no ordering
/// constraint left between which of the two calls is "forward" and which
/// is "backward". Verified for real below: CASE 32 (a genuine forward
/// call, in-process) and CASE 33 (genuine mutual recursion -- is_even()
/// calls is_odd() forward, is_odd() calls is_even() backward -- through
/// the real on-disk-ELF + kernel exec()/wait() path).
///
/// Still real, disclosed, UNCHANGED by this milestone (see this
/// milestone's own top-of-file doc comment and closing self-test
/// disclosure for the complete list): this kernel's single 4KiB
/// per-process stack page itself, MAX_FUNCS/MAX_PARAMS (both still 4,
/// unraised). Deep-recursion stack-depth SAFETY (as opposed to the
/// page count itself) was this milestone's own open item -- MILESTONE
/// 75 closed it, not by raising the page count, but by adding a real
/// kernel-side diagnostic (`interrupts.rs`'s `STACK_GUARD_REGION_SIZE`)
/// and proving end-to-end (CASE 34, unconditional unbounded
/// self-recursion) that a self-recursive/forward/mutual call chain
/// that runs off this single page is caught cleanly
/// (`WaitOutcome::Signaled`), not left as an unmeasured risk of silent
/// corruption.
///
/// Every function's own epilogue is the ordinary `leave; ret` shape
/// (CodegenMode::Callable) EXCEPT the entry function itself ("main"),
/// which uses the caller-supplied `mode` -- Standalone's real
/// `sys_exit(result)` sequence, or Callable's own `leave; ret` (itself
/// indistinguishable from an ordinary callee's epilogue, since a
/// Callable-mode entry point IS invoked as an ordinary function-pointer
/// call from Rust, see GenFn's own doc comment below) -- a real,
/// deliberate per-function decision, not a global one: a non-entry
/// function's own `return` must always genuinely RETURN TO ITS CALLER
/// (the real `call` instruction's own return address, already on the
/// stack), never `sys_exit()` the whole process out from under that
/// caller.
unsafe fn gen_program(src_ptr: u64, prog_head: u64, mode: CodegenMode) -> Result<(CodeBuf, u64), CodeGenError> {
    let mut nfuncs_total: u64 = 0;
    let mut probe = prog_head;
    while probe != 0 {
        nfuncs_total += 1;
        probe = unsafe { core::ptr::read(probe as *const FuncDef) }.next;
    }
    if nfuncs_total == 0 || nfuncs_total > MAX_FUNCS {
        return Err(CodeGenError::TooManyFuncs);
    }

    let funcs_ptr = unsafe { malloc(MAX_FUNCS * core::mem::size_of::<FuncSym>() as u64) };

    // MILESTONE 74: real PASS ONE -- register every function's own name
    // and param_count into funcs_ptr, in real source order, BEFORE any
    // body is compiled. `code_off` starts at the real UNRESOLVED_CODE_OFF
    // sentinel for every entry (this function's body-compiling loop,
    // pass two below, overwrites each entry with its own real code_off
    // immediately before that SAME function's own body is compiled --
    // exactly the ordering Milestone 73 already verified for
    // self-recursion, now covering every function in the table from the
    // very start of pass two). This is what makes a call site anywhere
    // in the program able to resolve any OTHER function's real name/
    // param_count regardless of source order, closing Milestone 72/73's
    // own disclosed "callee must be defined before caller" restriction.
    let mut reg_idx: u64 = 0;
    let mut reg_cur = prog_head;
    while reg_cur != 0 {
        let rf = unsafe { core::ptr::read(reg_cur as *const FuncDef) };
        unsafe {
            funcsym_write(funcs_ptr, reg_idx, FuncSym { name_off: rf.name_off, name_len: rf.name_len, code_off: UNRESOLVED_CODE_OFF, param_count: rf.param_count })
        };
        reg_idx += 1;
        reg_cur = rf.next;
    }

    let mut main_offset: Option<u64> = None;

    // MILESTONE 74: the real forward-call patch list -- see PendingCall's
    // own doc comment above for the full shape. Allocated once per
    // program, threaded unchanged through every gen_expr()/gen_stmt_list()
    // call in pass two below, resolved by the final backpatch pass at the
    // very end of this function.
    let pending_ptr = unsafe { malloc(MAX_PENDING_CALLS * core::mem::size_of::<PendingCall>() as u64) };
    let pending_count_ptr = unsafe { malloc(8) };
    unsafe { core::ptr::write(pending_count_ptr as *mut u64, 0u64) };

    // Same real headroom reasoning as gen_function()'s own CodeBuf::new()
    // call above, doubled (2048, not 1024) for real, disclosed reasons:
    // this buffer now holds potentially MULTIPLE functions' worth of
    // code, not just one, and buf.overflowed still catches it for real if
    // any future test ever needs more.
    let mut buf = unsafe { CodeBuf::new(2048) };

    // MILESTONE 74: real PASS TWO -- the original per-function compile
    // loop, unchanged in shape, now indexing into the already-fully-
    // populated table from pass one above instead of appending fresh
    // entries to a growing one. `nfuncs_total` (not an incrementing
    // per-iteration count) is what's threaded into gen_stmt_list() below,
    // since every function's own name/param_count is already real and
    // visible from the very first iteration.
    let mut idx: u64 = 0;
    let mut cur = prog_head;
    while cur != 0 {
        let f = unsafe { core::ptr::read(cur as *const FuncDef) };
        let code_off = buf.len;

        // Real, deliberate defense-in-depth backstop -- see
        // CodeGenError::TooManyFuncs's own doc comment; pass one above
        // already independently enforces nfuncs_total <= MAX_FUNCS.
        if idx >= MAX_FUNCS {
            return Err(CodeGenError::TooManyFuncs);
        }
        unsafe {
            funcsym_write(funcs_ptr, idx, FuncSym { name_off: f.name_off, name_len: f.name_len, code_off, param_count: f.param_count })
        };
        idx += 1;

        let is_entry = unsafe { word_is(src_ptr, f.name_off as u64, f.name_len as u64, b"main") };
        if is_entry {
            main_offset = Some(code_off);
        }
        let this_fn_mode = if is_entry { mode } else { CodegenMode::Callable };

        let vars_ptr = unsafe { malloc(MAX_VARS * core::mem::size_of::<VarSlot>() as u64) };
        let nvars = unsafe { collect_vars_for_function(&f, vars_ptr) }?;
        let raw_frame = nvars * 8;
        let frame_bytes = if raw_frame == 0 { 0u8 } else { (((raw_frame + 15) / 16) * 16) as u8 };

        unsafe { buf.emit_prologue(frame_bytes) };

        // Real prologue-time argument spill -- see emit_mov_rbp_off_reg()
        // and param_reg()'s own doc comments above for the full real
        // encoding/convention reasoning. Each parameter's own stack slot
        // was already assigned by collect_vars_for_function() above, in
        // the SAME declaration order param_reg() itself indexes by.
        let mut pi: u64 = 0;
        while pi < f.param_count as u64 {
            let off = -8 * (pi as i32 + 1);
            let reg = unsafe { param_reg(pi) };
            unsafe { buf.emit_mov_rbp_off_reg(off, reg) };
            pi += 1;
        }

        unsafe { loop_reset() }; // MILESTONE 87: fresh break/continue arena per function
        unsafe { gen_stmt_list(&mut buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs_total, pending_ptr, pending_count_ptr, this_fn_mode, f.body, NOT_IN_LOOP, 0) }?;

        // Safety backstop -- same real reasoning as gen_function()'s own
        // trailing epilogue above, per-function here.
        unsafe { emit_epilogue(&mut buf, this_fn_mode) };

        cur = f.next;
    }

    // MILESTONE 74: the real final backpatch pass -- every function in
    // the program has now been compiled, so every FuncSym's own code_off
    // is final; walk the pending-call list recorded during pass two above
    // and write each forward call's own real rel32 displacement now.
    let pending_count = unsafe { core::ptr::read(pending_count_ptr as *const u64) };
    let mut pc_i: u64 = 0;
    while pc_i < pending_count {
        let pc = unsafe { pending_read(pending_ptr, pc_i) };
        let callee = unsafe { funcsym_read(funcs_ptr, pc.func_idx) };
        unsafe { buf.patch_call_rel32(pc.field_off, callee.code_off) };
        pc_i += 1;
    }

    if buf.overflowed {
        return Err(CodeGenError::BufferFull);
    }
    match main_offset {
        Some(o) => Ok((buf, o)),
        None => Err(CodeGenError::NoMainFunction),
    }
}

// =======================================================================
// MILESTONE 69: a real, from-scratch ELF64 *writer* -- Tier 3's third
// slice. Wraps Milestone 68's STANDALONE-mode machine code (see
// CodegenMode above) in a real, minimal, well-formed ELF64 executable
// image: a real Elf64_Ehdr, a real single Elf64_Phdr (PT_LOAD), and the
// raw code bytes as that segment's content -- written entirely by hand
// here, in Rust, against the SAME field layout kernel/src/elf.rs's own
// real ELF64 *parser* already documents byte-for-byte (checked directly
// against that file before writing a single offset below, not
// independently re-derived): e_type@16, e_machine@18, e_version@20,
// e_entry@24, e_phoff@32, e_shoff@40, e_flags@48, e_ehsize@52,
// e_phentsize@54, e_phnum@56 for the 64-byte Ehdr; p_type@0, p_flags@4,
// p_offset@8, p_vaddr@16, p_paddr@24, p_filesz@32, p_memsz@40 for each
// 56-byte Phdr entry. No section headers are written (e_shoff=0,
// e_shnum=0) -- exactly what elf.rs's own doc comment already says a real
// ELF LOADER (as opposed to a linker) never needs, since only the program
// header table is consulted at load time.
//
// This is the real decision Milestone 68's own closing disclosure named
// as the honest next step: "begin the minimal x86_64 assembler/ELF-linker
// work so compiled output can become a real, standalone, on-disk
// executable runnable through this kernel's own real exec() rather than
// only via this milestone's in-process function-pointer shortcut." Checked
// directly against this repo before deciding exactly what to build: no
// x86_64 ASSEMBLER exists anywhere in this codebase (Milestone 67/68 both
// independently confirmed and deferred it), and Milestone 68's own codegen
// already emits real machine-code BYTES directly, not textual assembly --
// so there is nothing for an assembler to assemble here; what is
// genuinely missing, and genuinely sufficient, is a LINKER-shaped step
// that wraps already-real machine code in a real container format the
// kernel's real, pre-existing exec()/ELF-loading path (Milestone 36/45,
// kernel/src/elf.rs + process::create_process_from_elf(), completely
// UNCHANGED by this milestone) can load and run. No relocations are
// needed (this subset's codegen never emits a reference to an external
// symbol or a linker-resolved address -- every jump/call this grammar can
// produce is either PC-relative-free arithmetic or one of the syscall/
// register sequences above), so a real linker's usual relocation-
// processing job is genuinely absent here, not skipped -- a full x86_64
// assembler remains real, disclosed, deferred future work for whenever
// this subset's grammar grows control flow/functions and codegen starts
// needing textual assembly as an intermediate representation between
// itself and a SEPARATE linking step.
//
// The real load address baked in below (0x0000_5555_5000_0000) is NOT a
// new, independently-chosen constant -- checked directly against BOTH
// kernel/src/usertest.rs's own `pub const USER_CODE_ADDR: u64 =
// 0x_5555_5000_0000` AND tools/cc_src/linker.ld's own `. =
// 0x0000555550000000` (the load address cc.elf -- this very program -- is
// itself linked at) before hardcoding it here: the SAME address every
// other real, externally-built *_ELF_BYTES payload in this whole project
// already links against. Hardcoded, not imported, because userspace has
// no way to read a kernel-internal `pub(crate)` constant across the
// syscall boundary -- the same reason every *_src/linker.ld in this repo
// already repeats this literal address itself rather than sharing it from
// one place.
const ELF_LOAD_ADDR: u64 = 0x0000_5555_5000_0000;
const ELF_EHDR_SIZE: u64 = 64;
const ELF_PHDR_SIZE: u64 = 56;

/// Real, raw-pointer byte writer into an in-progress ELF image buffer --
/// same "no `[]` indexing on a runtime-variable index" discipline this
/// whole file's top doc comment establishes, mirrored from CodeBuf's own
/// push_bytes() above but writing at a caller-supplied absolute offset
/// (an ELF image's fields are NOT emitted in strictly increasing order --
/// e.g. e_phoff at offset 32 names where the Phdr table starts before any
/// Phdr byte itself is written) rather than always appending.
unsafe fn img_put_bytes(img: u64, off: u64, bytes: &[u8]) {
    let mut i: u64 = 0;
    for &b in bytes {
        unsafe { core::ptr::write((img + off + i) as *mut u8, b) };
        i += 1;
    }
}
unsafe fn img_put_u8(img: u64, off: u64, v: u8) {
    unsafe { core::ptr::write((img + off) as *mut u8, v) };
}
unsafe fn img_put_u16(img: u64, off: u64, v: u16) {
    unsafe { img_put_bytes(img, off, &v.to_le_bytes()) };
}
unsafe fn img_put_u32(img: u64, off: u64, v: u32) {
    unsafe { img_put_bytes(img, off, &v.to_le_bytes()) };
}
unsafe fn img_put_u64(img: u64, off: u64, v: u64) {
    unsafe { img_put_bytes(img, off, &v.to_le_bytes()) };
}

/// The real ELF64 writer itself: wraps `code_len` real machine-code bytes
/// at `code_ptr` (STANDALONE-mode output -- see CodegenMode -- ending in
/// a real sys_exit() sequence, not `leave; ret`) in a real, minimal,
/// well-formed ELF64 ET_EXEC image, malloc()-allocated (not a stack
/// array, same reason as every other buffer in this file) and returned as
/// `(image_ptr, image_len)`, or `(0, 0)` on malloc() failure.
///
/// Layout chosen to be the smallest real, honestly-valid ELF64 this
/// kernel's own loader (process::create_process_from_elf()) will accept:
/// ONE PT_LOAD segment, `p_offset=0` (the WHOLE file -- header, the
/// single Phdr, and the code -- is that one segment's content, the same
/// "headers are part of the mapped segment, code starts partway through
/// it" shape a real linker's own default output already uses), `p_vaddr
/// = ELF_LOAD_ADDR` (real, checked page-aligned -- `0x...000` low 12 bits
/// are all zero), `e_entry = ELF_LOAD_ADDR + (ELF_EHDR_SIZE +
/// ELF_PHDR_SIZE)` (the real first byte of the actual code, immediately
/// after the header+Phdr bytes preceding it) -- independently checked
/// against process::create_process_from_elf()'s own real
/// `entry_in_segment` validation (entry must fall within
/// `[p_vaddr, p_vaddr+p_memsz)`) before relying on it: true here since
/// `p_memsz = ELF_EHDR_SIZE + ELF_PHDR_SIZE + code_len > ELF_EHDR_SIZE +
/// ELF_PHDR_SIZE` for any `code_len > 0`. Total image size for every
/// program this subset's grammar can currently produce (CodeBuf::new(512)
/// caps generated code at 512 bytes) is at most 120 + 512 = 632 bytes --
/// one 4 KiB page (this loader's own real per-segment page accounting,
/// checked against MAX_PAGES_PER_ELF_SEGMENT/MAX_TOTAL_ELF_PAGES in
/// process.rs, both far larger than the 1 page this ever needs), and well
/// under fs::MAX_FILE_BYTES (4096) -- so, unlike Milestone 67's own
/// cc.elf (8000 bytes, over that same cap), an ELF this function builds
/// can be written through the REAL on-disk filesystem via the ordinary
/// open()/fdwrite()/close() syscalls with no special-casing needed.
///
/// MILESTONE 72: gained one new parameter, `entry_off` -- the real byte
/// offset (WITHIN `code_ptr`) where the process should actually START
/// executing, 0 for every call site before this milestone (Milestone
/// 68-71's single-function CodegenMode::Standalone output always begins
/// at the very first byte of its own CodeBuf, so `entry_off == 0` there
/// is a real, unchanged invariant, not a guess) but potentially nonzero
/// now that gen_program() above can emit MULTIPLE functions into one
/// buffer, with "main" not necessarily first. `compile_standalone_elf()`
/// below (the pre-existing, single-function path) still always passes
/// `0`; `compile_program_standalone_elf()` (new this milestone) passes
/// gen_program()'s own real `main_offset`.
unsafe fn build_elf64_standalone(code_ptr: u64, code_len: u64, entry_off: u64) -> (u64, u64) {
    let header_len = ELF_EHDR_SIZE + ELF_PHDR_SIZE;
    let total_len = header_len + code_len;
    let img = unsafe { malloc(total_len) };
    if img == 0 {
        return (0, 0);
    }
    unsafe { memset(img, 0, total_len) };

    // e_ident[0..16]: magic + class/data/version + 9 reserved/padding
    // bytes -- bytes 8..16 stay zero (already memset above), matching
    // real ELFOSABI_NONE/EI_ABIVERSION=0/EI_PAD convention.
    unsafe { img_put_u8(img, 0, 0x7F) };
    unsafe { img_put_u8(img, 1, b'E') };
    unsafe { img_put_u8(img, 2, b'L') };
    unsafe { img_put_u8(img, 3, b'F') };
    unsafe { img_put_u8(img, 4, 2) }; // EI_CLASS = ELFCLASS64
    unsafe { img_put_u8(img, 5, 1) }; // EI_DATA = ELFDATA2LSB
    unsafe { img_put_u8(img, 6, 1) }; // EI_VERSION = EV_CURRENT

    let entry = ELF_LOAD_ADDR + header_len + entry_off;
    unsafe { img_put_u16(img, 16, 2) }; // e_type = ET_EXEC
    unsafe { img_put_u16(img, 18, 0x3E) }; // e_machine = EM_X86_64
    unsafe { img_put_u32(img, 20, 1) }; // e_version = EV_CURRENT
    unsafe { img_put_u64(img, 24, entry) }; // e_entry
    unsafe { img_put_u64(img, 32, ELF_EHDR_SIZE) }; // e_phoff -- Phdr table right after Ehdr
    unsafe { img_put_u64(img, 40, 0) }; // e_shoff = 0 (no section headers)
    unsafe { img_put_u32(img, 48, 0) }; // e_flags
    unsafe { img_put_u16(img, 52, ELF_EHDR_SIZE as u16) }; // e_ehsize
    unsafe { img_put_u16(img, 54, ELF_PHDR_SIZE as u16) }; // e_phentsize
    unsafe { img_put_u16(img, 56, 1) }; // e_phnum = 1
    unsafe { img_put_u16(img, 58, 0) }; // e_shentsize
    unsafe { img_put_u16(img, 60, 0) }; // e_shnum
    unsafe { img_put_u16(img, 62, 0) }; // e_shstrndx

    // The one real Elf64_Phdr, at file offset 64 (== e_phoff above).
    let ph = ELF_EHDR_SIZE;
    unsafe { img_put_u32(img, ph, 1) }; // p_type = PT_LOAD
    unsafe { img_put_u32(img, ph + 4, 5) }; // p_flags = PF_R | PF_X
    unsafe { img_put_u64(img, ph + 8, 0) }; // p_offset = 0 (whole file is this segment)
    unsafe { img_put_u64(img, ph + 16, ELF_LOAD_ADDR) }; // p_vaddr
    unsafe { img_put_u64(img, ph + 24, ELF_LOAD_ADDR) }; // p_paddr (unused by this loader, set for real-format completeness)
    unsafe { img_put_u64(img, ph + 32, total_len) }; // p_filesz
    unsafe { img_put_u64(img, ph + 40, total_len) }; // p_memsz
    unsafe { img_put_u64(img, ph + 48, 0x1000) }; // p_align

    unsafe { memcpy(img + header_len, code_ptr, code_len) };

    (img, total_len)
}

/// MILESTONE 69: real, shared end-to-end verification logic for CASE
/// 8/9/10 below -- writes a real ELF64 image to a real path on the
/// on-disk filesystem via the ordinary open()/fdwrite()/close() syscalls,
/// then fork()s: the CHILD calls the kernel's own real exec() syscall
/// (Milestone 36/45's pre-existing, UNCHANGED elf::parse() +
/// process::exec_elf() path) against that exact on-disk file -- on
/// success this call NEVER RETURNS (the child's whole process image is
/// replaced, jumping directly into the real, freshly-compiled entry this
/// macro's own caller built), so the only way `sys_exit(250)` below is
/// ever reached is a real exec() FAILURE -- a distinct, diagnosable
/// marker exit code none of this subset's own compiled `return` values
/// could ever produce (this self-test's arithmetic stays well under 250).
/// The PARENT (this macro's own caller, still the original cc.elf
/// process) fork()s, then real sys_wait()s for the child and decodes the
/// real WAIT encoding usertest.rs's own syscall 8 doc comment establishes
/// (bits 0-7 = reaped pid, bits 8-15 = the real exit code, bit 16 = 1 iff
/// the child actually reached its own exit() -- WaitOutcome::Exited, not
/// Killed/Signaled), evaluating to true only if the child both genuinely
/// exited AND its real exit code matches `$expected_code` exactly.
///
/// **A real, pre-existing kernel bug this milestone found, and a REAL
/// dead end before finding the actual robust fix -- both disclosed, not
/// hidden**: the path bytes' address must stay correct in the CHILD after
/// `sys_fork()` returns -- but this kernel's real `fork()` (checked
/// directly against `process::fork()`/`Process::pending_resume` in
/// `kernel/src/process.rs`) only ever CAPTURES and RESTORES the parent's
/// `rip`/`rsp` for the child (`pending_resume: Option<(u64, u64)>`,
/// exactly two values) -- every OTHER general-purpose register is real,
/// genuinely UNRESTORED kernel-side state at the moment the child resumes.
/// Confirmed the hard way: an early version called
/// `sys_exec(path.as_ptr() as u64, path.len() as u64)` with `path: &[u8]`
/// an ordinary FUNCTION PARAMETER, hitting a real, hardware-recorded page
/// fault (`Accessed Address: 0x13`) the instant the child read a
/// register-cached copy of `path`'s pointer that never survived the
/// fork(). A SECOND real attempt (also disclosed, not erased from
/// history) tried fixing this by round-tripping the pointer through a
/// dedicated `static mut` via `write_volatile`/`read_volatile` -- a real,
/// link-time-constant ADDRESS, immune to the register problem in theory
/// -- and it fixed the ORIGINAL symptom, but independent re-testing (with
/// this milestone's OTHER real fix, the CHILD_EXCURSION_STACK bump below,
/// already in place) kept reproducing the exact same `0x13` fault
/// intermittently, layout-sensitive in a way that was never fully
/// root-caused. **The real, robust fix actually used here**: never pass
/// the path bytes through ANY value that has to cross the `sys_fork()`
/// boundary at all -- not a register, not a static's CONTENT. This macro
/// splices `$path` directly into the CHILD branch's own source text at
/// EVERY expansion site (CASE 8/9/10 below each get their own independent
/// copy of this code, each with ITS OWN path literal spliced in) -- since
/// `$path` is always a `const PATHn: &[u8] = b"..."` reference, the
/// compiler materializes its address as a fresh immediate at that exact
/// point in the CHILD's own compiled code, exactly like the plain
/// `w(b"...")` diagnostic messages elsewhere in this file already do
/// successfully (proven working in the child on every single run) --
/// zero dependence on any register OR memory content surviving the
/// fork() boundary, only on the CODE ITSELF being present, which
/// Milestone 69's OTHER real fix (extra_frames now carrying real vaddrs,
/// see that field's own doc comment on Process in kernel/src/process.rs)
/// already independently guarantees.
macro_rules! write_exec_and_check {
    ($path:expr, $elf_ptr:expr, $elf_len:expr, $expected_code:expr) => {{
        let __elf_ptr: u64 = $elf_ptr;
        let __elf_len: u64 = $elf_len;
        let __expected_code: u64 = $expected_code;
        let __path: &[u8] = $path;
        if __elf_ptr == 0 {
            w(b"  build_elf64_standalone returned null (malloc failure)\n");
            false
        } else {
            let __fd = sys_open_trunc(__path.as_ptr() as u64, __path.len() as u64);
            if __fd == SYSCALL_FAIL {
                w(b"  sys_open failed\n");
                false
            } else {
                let __written = sys_fdwrite(__fd, __elf_ptr, __elf_len);
                sys_close(__fd);
                if __written != __elf_len {
                    w(b"  sys_fdwrite wrote fewer bytes than the real ELF image's own length -- real, disclosed short-write\n");
                    false
                } else {
                    let __fork_result = sys_fork();
                    if __fork_result == 0 {
                        // CHILD: `$path` spliced directly here -- see this
                        // macro's own top doc comment for exactly why that
                        // (not a register, not a static round-trip) is the
                        // real, robust fix.
                        let __child_path: &[u8] = $path;
                        sys_exec(__child_path.as_ptr() as u64, __child_path.len() as u64);
                        // Only reached if exec() genuinely failed.
                        w(b"  child sys_exec() FAILED, returned instead of replacing this process\n");
                        sys_exit(250);
                    }
                    if __fork_result == SYSCALL_FAIL {
                        w(b"  sys_fork failed\n");
                        false
                    } else {
                        let __wait_raw = sys_wait(__fork_result);
                        if __wait_raw == SYSCALL_FAIL {
                            w(b"  sys_wait failed\n");
                            false
                        } else {
                            let __code = (__wait_raw >> 8) & 0xFF;
                            let __exited = (__wait_raw >> 16) & 1 == 1;
                            w(b"  real on-disk ELF written, real fork()+exec()+wait() -- child exited=");
                            w(if __exited { b"true" } else { b"false" });
                            w(b" real exit code=");
                            write_u64_dec(__code);
                            w(b" (expected exited=true code=");
                            write_u64_dec(__expected_code);
                            w(b")\n");
                            __exited && __code == __expected_code
                        }
                    }
                }
            }
        }
    }};
}

/// Real helper used only by the fresh CASE 5/6/7 sources below: lex()
/// then parse_function() a source buffer, returning the resulting
/// FuncDef pointer. Does not distinguish a lex failure from a parse
/// failure (both fold to `Err(())`) -- CASE 2 and CASE 3 above already
/// independently prove BOTH real error paths individually with exact
/// hand-predicted failure points, so this helper only needs the success
/// path to be real for these three cases, which are all deliberately
/// lexically/syntactically valid (CASE 7 is a real SEMANTIC error --
/// undeclared variable -- caught by gen_function() below, not by lex()
/// or parse_function()).
unsafe fn lex_and_parse(src_ptr: u64, src_len: u64) -> Result<u64, ()> {
    let toks_ptr = unsafe { malloc((MAX_TOKENS * core::mem::size_of::<Token>()) as u64) };
    let n = match unsafe { lex(src_ptr, src_len, toks_ptr, MAX_TOKENS as u64) } {
        Ok(n) => n,
        Err(_) => return Err(()),
    };
    let mut p = Parser { toks_ptr, ntoks: n, pos: 0 };
    match unsafe { p.parse_function() } {
        Ok(f) => Ok(f),
        Err(_) => Err(()),
    }
}

type GenFn = unsafe extern "C" fn() -> u64;

/// MILESTONE 70: real, shared "compile a fresh source and actually call
/// it" helper -- factors out the lex_and_parse() + gen_function(Callable)
/// + transmute-and-call sequence CASE 5/6 (Milestone 68) and CASE 11-16
/// (Milestone 70) below all independently need, each with its own fresh
/// source. Added specifically to keep `cc.elf`'s own compiled SIZE real
/// and under `process::MAX_PAGES_PER_ELF_SEGMENT`'s real, fixed 4-page
/// cap (checked directly against kernel/src/process.rs, not guessed at)
/// -- before this helper existed, each case's own copy of this
/// match-inside-match control flow was independently compiled, and
/// Milestone 70's own eight new cases pushed the whole binary to a real,
/// confirmed 5-page PT_LOAD segment, an actual `run_loaded_elf_process
/// (cc.elf) returned Err` self-test failure on a real boot -- not a
/// hypothetical concern. `None` folds together "didn't lex/parse" and
/// "gen_function returned Err" the same way lex_and_parse() above already
/// folds lex/parse failures -- every case using this helper below is
/// deliberately lexically/semantically valid, so only the success path
/// needs to be real.
unsafe fn compile_and_run_callable(src_ptr: u64, src_len: u64) -> Option<u64> {
    // MILESTONE 87: throw away everything the PREVIOUS compilation left
    // on the heap (its result is already a plain value the caller has;
    // its tokens/AST/CodeBuf are dead). Keeps the ~50-compile self-test
    // inside the fixed per-process heap. `src` itself is a `&[u8]`
    // static, never on the heap, so it survives this.
    unsafe { heap_reset() };
    let func = match unsafe { lex_and_parse(src_ptr, src_len) } {
        Ok(f) => f,
        Err(_) => return None,
    };
    let buf = match unsafe { gen_function(src_ptr, func, CodegenMode::Callable) } {
        Ok(b) => b,
        Err(_) => return None,
    };
    let f: GenFn = unsafe { core::mem::transmute::<u64, GenFn>(buf.ptr) };
    Some(unsafe { f() })
}

/// MILESTONE 70: the same real size-driven DRY refactor as
/// compile_and_run_callable() above, for the Standalone+ELF64 side --
/// factors out lex_and_parse() + gen_function(Standalone) +
/// build_elf64_standalone() (everything CASE 9/10 (Milestone 69) and
/// CASE 17/18 (Milestone 70) below need BEFORE reaching
/// write_exec_and_check!()'s own real fork()/exec() boundary -- that
/// macro itself stays a macro, unrefactored, for the real reason its own
/// doc comment gives, this helper only covers the part that safely CAN
/// be a function). Returns `(0, 0)` on any real failure, the same
/// sentinel build_elf64_standalone() itself already uses for a malloc()
/// failure, so callers only need one check.
unsafe fn compile_standalone_elf(src_ptr: u64, src_len: u64) -> (u64, u64) {
    let func = match unsafe { lex_and_parse(src_ptr, src_len) } {
        Ok(f) => f,
        Err(_) => return (0, 0),
    };
    let buf = match unsafe { gen_function(src_ptr, func, CodegenMode::Standalone) } {
        Ok(b) => b,
        Err(_) => return (0, 0),
    };
    unsafe { build_elf64_standalone(buf.ptr, buf.len, 0) }
}

/// MILESTONE 72: the multi-function analogue of lex_and_parse() above --
/// lex()es then parse_program()s (not parse_function()) a source buffer,
/// returning the resulting FuncDef LIST's own head. Same "fold every
/// failure to Err(())" shape as lex_and_parse() itself; every source
/// passed to this helper below is deliberately lexically/syntactically
/// valid.
unsafe fn lex_and_parse_program(src_ptr: u64, src_len: u64) -> Result<u64, ()> {
    let toks_ptr = unsafe { malloc((MAX_TOKENS * core::mem::size_of::<Token>()) as u64) };
    let n = match unsafe { lex(src_ptr, src_len, toks_ptr, MAX_TOKENS as u64) } {
        Ok(n) => n,
        Err(_) => return Err(()),
    };
    let mut p = Parser { toks_ptr, ntoks: n, pos: 0 };
    match unsafe { p.parse_program() } {
        Ok(head) => Ok(head),
        Err(_) => Err(()),
    }
}

/// MILESTONE 72: the multi-function, Callable-mode analogue of
/// compile_and_run_callable() above -- lex_and_parse_program() +
/// gen_program(Callable) + transmute-and-call, but calling the resulting
/// "main" entry point (at `buf.ptr + main_off`, NOT necessarily
/// `buf.ptr` itself -- see gen_program()'s own doc comment for why) --
/// every self-test source using this helper below declares `main`
/// with ZERO parameters of its own (the interesting real parameter-
/// passing this milestone tests happens INSIDE main, at ITS OWN call
/// site to some other function -- see e.g. CASE 24 below), so the
/// existing zero-argument `GenFn` type Milestone 68 already established
/// is still the right shape to call through here, unchanged.
unsafe fn compile_and_run_program_callable(src_ptr: u64, src_len: u64) -> Option<u64> {
    unsafe { heap_reset() }; // MILESTONE 87 -- see compile_and_run_callable()
    let prog = match unsafe { lex_and_parse_program(src_ptr, src_len) } {
        Ok(p) => p,
        Err(_) => return None,
    };
    let (buf, main_off) = match unsafe { gen_program(src_ptr, prog, CodegenMode::Callable) } {
        Ok(r) => r,
        Err(_) => return None,
    };
    let f: GenFn = unsafe { core::mem::transmute::<u64, GenFn>(buf.ptr + main_off) };
    Some(unsafe { f() })
}

/// MILESTONE 72: the multi-function, Standalone+ELF64 analogue of
/// compile_standalone_elf() above -- lex_and_parse_program() +
/// gen_program(Standalone) + build_elf64_standalone(), threading
/// gen_program()'s own real `main_off` through as build_elf64_standalone's
/// new `entry_off` parameter so the resulting ELF image's own `e_entry`
/// points at "main"'s real first byte even when main isn't the first
/// function in the buffer.
unsafe fn compile_program_standalone_elf(src_ptr: u64, src_len: u64) -> (u64, u64) {
    // MILESTONE 87: reset the heap first. The RETURNED (elf_ptr, elf_len)
    // stays valid until the NEXT compile helper runs -- every caller
    // writes the ELF to disk via `write_exec_and_check!` immediately, so
    // that window is always long enough.
    unsafe { heap_reset() };
    let prog = match unsafe { lex_and_parse_program(src_ptr, src_len) } {
        Ok(p) => p,
        Err(_) => return (0, 0),
    };
    let (buf, main_off) = match unsafe { gen_program(src_ptr, prog, CodegenMode::Standalone) } {
        Ok(r) => r,
        Err(_) => return (0, 0),
    };
    unsafe { build_elf64_standalone(buf.ptr, buf.len, main_off) }
}

// =======================================================================
// Self-test -- three real, hand-computed cases: (1) a valid tiny C
// function, checking BOTH the exact lexer token stream (kind sequence
// and key payload values) AND the exact AST shape the parser builds
// from it; (2) a deliberate parse error (a missing `;`), proving the
// parser's error path is real, not just an unexercised Err variant;
// (3) a deliberate lex error (an unrecognized character), proving the
// SAME for the lexer. All three run for real, in this real ring-3
// process, against this file's own real lex()/parse_function().
// =======================================================================

#[unsafe(link_section = ".text.start")]
#[unsafe(no_mangle)]
pub extern "C" fn _start() -> ! {
    unsafe {
        w(b"milestone 67: cc (subset-C lexer + parser, no codegen yet) starting\n");

        // -------------------------------------------------------------
        // CASE 1: a real, valid tiny C program.
        //   int main() {
        //       int x;
        //       x = 40 + 2;
        //       return x;
        //   }
        // Hand-computed expected token stream (19 tokens including a
        // final EOF -- derived by hand, token by token, from the
        // source literal below before this test was ever run):
        //   0 int  1 IDENT(main)  2 (  3 )  4 {
        //   5 int  6 IDENT(x)  7 ;
        //   8 IDENT(x)  9 =  10 INTLIT(40)  11 +  12 INTLIT(2)  13 ;
        //   14 return  15 IDENT(x)  16 ;
        //   17 }  18 EOF
        // Hand-computed expected AST: FuncDef("main", 3 statements):
        //   stmt0 = DECL(x)
        //   stmt1 = ASSIGN(x, BINARY('+', INTLIT(40), INTLIT(2)))
        //   stmt2 = RETURN(IDENT(x))
        // -------------------------------------------------------------
        const SRC1: &[u8] = b"int main() {\n    int x;\n    x = 40 + 2;\n    return x;\n}\n";
        let src1_ptr = SRC1.as_ptr() as u64;
        let src1_len = SRC1.len() as u64;

        // Token buffers are malloc()ed, NOT stack arrays -- this
        // process's real user stack is exactly ONE 4KiB page
        // (process::create_loaded_elf_process()'s own single
        // `allocate_frame()` call for `stack_frame`, confirmed against
        // that code directly). `MAX_TOKENS` (64) `Token`s at 24 bytes
        // each is 1536 bytes; three such buffers (one per test case
        // below) would alone be 4608 bytes -- already over budget
        // before any other local. An early version of this file used
        // plain `[Token; MAX_TOKENS]` stack arrays for all three and
        // hit a REAL stack overflow confirmed the hard way: a genuine
        // `milestone 41: SIGSEGV` write fault a few bytes below the
        // stack's own top, silently swallowed by
        // run_loaded_elf_process() returning `Ok` regardless (that
        // function only proves "no panic, no double fault", a real,
        // disclosed, narrower check than "the program's own logic
        // actually ran") -- so this process's own "milestone 67: cc"
        // lines never appeared in the serial log at all despite the
        // kernel-side wrapper reporting a clean run. Fixed by moving
        // every token buffer onto the heap instead, the same "buffers
        // live in a malloc()ed allocation, not on the stack" pattern
        // stdiotest_src's own `File.rbuf`/`File.wbuf` already establish.
        let toks1_ptr = malloc((MAX_TOKENS * core::mem::size_of::<Token>()) as u64);
        let lex1 = lex(src1_ptr, src1_len, toks1_ptr, MAX_TOKENS as u64);
        let (lex1_ok, ntoks1) = match lex1 {
            Ok(n) => (true, n),
            Err(_) => (false, 0u64),
        };
        write_check(b"case1_lex_ok=", lex1_ok);
        w(b"  ntoks1=");
        write_u64_dec(ntoks1);
        w(b"\n");
        let ntoks1_ok = ntoks1 == 19;
        write_check(b"case1_ntoks_is_19=", ntoks1_ok);

        const EXPECTED_KINDS1: [u8; 19] = [
            TOK_INT, TOK_IDENT, TOK_LPAREN, TOK_RPAREN, TOK_LBRACE, TOK_INT, TOK_IDENT, TOK_SEMI, TOK_IDENT, TOK_ASSIGN,
            TOK_INTLIT, TOK_PLUS, TOK_INTLIT, TOK_SEMI, TOK_RETURN, TOK_IDENT, TOK_SEMI, TOK_RBRACE, TOK_EOF,
        ];
        let kinds1_ptr = EXPECTED_KINDS1.as_ptr() as u64;
        let mut kinds1_ok = ntoks1_ok;
        if ntoks1_ok {
            let mut i: u64 = 0;
            while i < 19 {
                let got = tok_read(toks1_ptr, i).kind;
                let expected = core::ptr::read((kinds1_ptr + i) as *const u8);
                if got != expected {
                    kinds1_ok = false;
                }
                i += 1;
            }
        }
        write_check(b"case1_token_kinds_match=", kinds1_ok);

        // Real payload spot-checks: the identifier at token 1 is really
        // "main" (not just any TOK_IDENT), and the two integer literals
        // (tokens 10, 12) really carry the values 40 and 2.
        let t1 = tok_read(toks1_ptr, 1);
        let name_main_ok = word_is(src1_ptr, t1.ident_off as u64, t1.ident_len as u64, b"main");
        let t10 = tok_read(toks1_ptr, 10);
        let t12 = tok_read(toks1_ptr, 12);
        let intlit_vals_ok = t10.int_val == 40 && t12.int_val == 2;
        write_check(b"case1_token_payloads_match=", name_main_ok && intlit_vals_ok);

        let mut parser1 = Parser { toks_ptr: toks1_ptr, ntoks: ntoks1, pos: 0 };
        let parse1 = parser1.parse_function();
        let (parse1_ok, func1) = match parse1 {
            Ok(f) => (true, f),
            Err(_) => (false, 0u64),
        };
        write_check(b"case1_parse_ok=", parse1_ok);

        let mut ast1_ok = false;
        if parse1_ok {
            let f = core::ptr::read(func1 as *const FuncDef);
            let name_ok = word_is(src1_ptr, f.name_off as u64, f.name_len as u64, b"main");
            let count_ok = f.stmt_count == 3;
            write_check(b"case1_func_name_is_main=", name_ok);
            write_check(b"case1_stmt_count_is_3=", count_ok);

            let mut shape_ok = name_ok && count_ok;
            if count_ok {
                let s1 = core::ptr::read(f.body as *const StmtNode);
                let s1_ok = s1.kind == STMT_DECL && word_is(src1_ptr, s1.ident_off as u64, s1.ident_len as u64, b"x");
                write_check(b"case1_stmt0_is_decl_x=", s1_ok);

                let s2 = core::ptr::read(s1.next as *const StmtNode);
                let s2_target_ok = s2.kind == STMT_ASSIGN && word_is(src1_ptr, s2.ident_off as u64, s2.ident_len as u64, b"x");
                let e2 = core::ptr::read(s2.expr as *const ExprNode);
                let e2_shape_ok = e2.kind == EXPR_BINARY && e2.op == b'+';
                let e2l = core::ptr::read(e2.left as *const ExprNode);
                let e2r = core::ptr::read(e2.right as *const ExprNode);
                let e2_vals_ok = e2l.kind == EXPR_INTLIT && e2l.int_val == 40 && e2r.kind == EXPR_INTLIT && e2r.int_val == 2;
                let s2_ok = s2_target_ok && e2_shape_ok && e2_vals_ok;
                write_check(b"case1_stmt1_is_assign_x_40plus2=", s2_ok);

                let s3 = core::ptr::read(s2.next as *const StmtNode);
                let s3_kind_ok = s3.kind == STMT_RETURN;
                let e3 = core::ptr::read(s3.expr as *const ExprNode);
                let e3_ok = e3.kind == EXPR_IDENT && word_is(src1_ptr, e3.ident_off as u64, e3.ident_len as u64, b"x");
                let s3_ok = s3_kind_ok && e3_ok;
                write_check(b"case1_stmt2_is_return_x=", s3_ok);

                shape_ok = shape_ok && s1_ok && s2_ok && s3_ok;
            }
            ast1_ok = shape_ok;
        }

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 2: a deliberate parse ERROR -- missing `;` after `int x`.
        //   int main() { int x return x; }
        // Hand-computed tokens: 0 int 1 main 2 ( 3 ) 4 { 5 int 6 x
        //   7 return 8 x 9 ; 10 } 11 EOF
        // parse_stmt() for "int x" consumes tokens 5,6 then calls
        // expect(TOK_SEMI) with self.pos == 7, but token 7 is
        // TOK_RETURN, not TOK_SEMI -- hand-predicted real failure:
        // Err(UnexpectedToken(7)).
        // -------------------------------------------------------------
        const SRC2: &[u8] = b"int main() { int x return x; }";
        let src2_ptr = SRC2.as_ptr() as u64;
        let src2_len = SRC2.len() as u64;
        let toks2_ptr = malloc((MAX_TOKENS * core::mem::size_of::<Token>()) as u64);
        let lex2 = lex(src2_ptr, src2_len, toks2_ptr, MAX_TOKENS as u64);
        let (lex2_ok, ntoks2) = match lex2 {
            Ok(n) => (true, n),
            Err(_) => (false, 0u64),
        };
        write_check(b"case2_lex_ok=", lex2_ok);

        let mut parser2 = Parser { toks_ptr: toks2_ptr, ntoks: ntoks2, pos: 0 };
        let parse2 = parser2.parse_function();
        let case2_ok = match parse2 {
            Err(ParseError::UnexpectedToken(pos)) => {
                w(b"  case2 real parse error at token index=");
                write_u64_dec(pos);
                w(b"\n");
                pos == 7
            }
            _ => false,
        };
        write_check(b"case2_real_parse_error_at_token7=", case2_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 3: a deliberate lex ERROR -- a single unrecognized
        // character. Hand-predicted: lex() hits '@' at offset 0 (the
        // very first byte), before any token is written -- real
        // failure: Err(UnknownChar(0)).
        // -------------------------------------------------------------
        const SRC3: &[u8] = b"@";
        let src3_ptr = SRC3.as_ptr() as u64;
        let src3_len = SRC3.len() as u64;
        let toks3_ptr = malloc((MAX_TOKENS * core::mem::size_of::<Token>()) as u64);
        let lex3 = lex(src3_ptr, src3_len, toks3_ptr, MAX_TOKENS as u64);
        let case3_ok = match lex3 {
            Err(LexError::UnknownChar(off)) => {
                w(b"  case3 real lex error at byte offset=");
                write_u64_dec(off);
                w(b"\n");
                off == 0
            }
            _ => false,
        };
        write_check(b"case3_real_lex_error_at_offset0=", case3_ok);

        w(b"\n");

        let overall = lex1_ok
            && ntoks1_ok
            && kinds1_ok
            && name_main_ok
            && intlit_vals_ok
            && parse1_ok
            && ast1_ok
            && lex2_ok
            && case2_ok
            && case3_ok;

        w(b"OVERALL=");
        w(if overall { b"PASS" } else { b"FAIL" });
        w(b"\n\n");

        // ===============================================================
        // MILESTONE 68: real codegen -- compile an AST to real x86_64
        // machine code, then ACTUALLY CALL the result and check the real
        // returned value, rather than just inspecting the emitted bytes'
        // shape. Four cases: (4) reuses CASE 1's already-verified
        // func1 AST (int x; x = 40 + 2; return x;) -- proves codegen
        // against the SAME tree Milestone 67 already proved was built
        // correctly, expected result 42; (5) and (6) are two FRESH
        // source programs, lexed+parsed+compiled+executed from scratch,
        // proving codegen is real and general rather than hard-coded to
        // CASE 1's own shape -- (5) exercises two variables, '*', and
        // '-' (a = 6; b = a * 7; return b - 1; -> 41); (6) exercises
        // '/' (the trickiest encoding, cqo+idiv) together with real
        // operator precedence (x / 4 + 3, division binding tighter than
        // addition even with no parentheses -> 100/4+3 = 28); (7) a
        // deliberate SEMANTIC error -- a reference to an undeclared
        // variable -- proving gen_function()'s own Err path is real, the
        // same "prove the Err path is real" discipline CASE 2/CASE 3
        // above already established for the lexer/parser.
        // ===============================================================
        w(b"milestone 68: cc codegen (real x86_64 machine code, direct execution) starting\n");

        // -------------------------------------------------------------
        // CASE 4: reuse CASE 1's func1 (int x; x = 40 + 2; return x;).
        // Hand-computed expected result: 42.
        // -------------------------------------------------------------
        let case4_ok = if parse1_ok && ast1_ok {
            match gen_function(src1_ptr, func1, CodegenMode::Callable) {
                Ok(buf4) => {
                    let f4: GenFn = core::mem::transmute::<u64, GenFn>(buf4.ptr);
                    let result4 = f4();
                    w(b"  case4 returned=");
                    write_u64_dec(result4);
                    w(b" (expected 42)\n");
                    result4 == 42
                }
                Err(_) => {
                    w(b"  case4 gen_function(func1) returned Err unexpectedly\n");
                    false
                }
            }
        } else {
            w(b"  case4 skipped -- CASE 1's own func1 was not valid\n");
            false
        };
        write_check(b"case4_codegen_exec_returns_42=", case4_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 5: a fresh source, two variables, '*' and '-'.
        //   int main() { int a; int b; a = 6; b = a * 7; return b - 1; }
        // Hand-computed: a=6, b=6*7=42, return 42-1=41.
        // -------------------------------------------------------------
        const SRC5: &[u8] = b"int main() {\n    int a;\n    int b;\n    a = 6;\n    b = a * 7;\n    return b - 1;\n}\n";
        let case5_result = compile_and_run_callable(SRC5.as_ptr() as u64, SRC5.len() as u64);
        w(b"  case5 returned=");
        if let Some(r) = case5_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 41)\n");
        let case5_ok = case5_result == Some(41);
        write_check(b"case5_two_vars_mul_sub_returns_41=", case5_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 6: a fresh source, '/' and real precedence (division
        // binds tighter than '+' with no parentheses needed).
        //   int main() { int x; x = 100; return x / 4 + 3; }
        // Hand-computed: 100 / 4 = 25, 25 + 3 = 28.
        // -------------------------------------------------------------
        const SRC6: &[u8] = b"int main() {\n    int x;\n    x = 100;\n    return x / 4 + 3;\n}\n";
        let case6_result = compile_and_run_callable(SRC6.as_ptr() as u64, SRC6.len() as u64);
        w(b"  case6 returned=");
        if let Some(r) = case6_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 28)\n");
        let case6_ok = case6_result == Some(28);
        write_check(b"case6_division_and_precedence_returns_28=", case6_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 7: a deliberate SEMANTIC error -- `z` is never declared.
        //   int main() { int y; return z; }
        // Hand-computed byte offset of `z` in the source below: counting
        // "int main() { int y; return z; }" byte by byte, `z` is the
        // 28th byte, offset 27 (0-indexed) -- real predicted failure:
        // Err(UndeclaredVariable(27)).
        // -------------------------------------------------------------
        const SRC7: &[u8] = b"int main() { int y; return z; }";
        let src7_ptr = SRC7.as_ptr() as u64;
        let src7_len = SRC7.len() as u64;
        let case7_ok = match lex_and_parse(src7_ptr, src7_len) {
            Ok(func7) => match gen_function(src7_ptr, func7, CodegenMode::Callable) {
                Err(CodeGenError::UndeclaredVariable(off)) => {
                    w(b"  case7 real codegen error -- undeclared variable at byte offset=");
                    write_u64_dec(off);
                    w(b"\n");
                    off == 27
                }
                _ => {
                    w(b"  case7 expected Err(UndeclaredVariable(27)), got something else\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case7 lex_and_parse() returned Err unexpectedly (source should lex+parse fine)\n");
                false
            }
        };
        write_check(b"case7_undeclared_variable_error_at_offset27=", case7_ok);

        w(b"\n");

        let overall_m68 = case4_ok && case5_ok && case6_ok && case7_ok;
        w(b"OVERALL_M68=");
        w(if overall_m68 { b"PASS" } else { b"FAIL" });
        w(b"\n\n");

        // ===============================================================
        // MILESTONE 69: real ELF64 writer + the kernel's own real, pre-
        // existing exec() path -- Tier 3's third slice. Three cases, the
        // strongest verification this whole tier has produced yet: each
        // one compiles subset-C source to STANDALONE-mode machine code
        // (CodegenMode::Standalone -- ending in a real sys_exit(result)
        // sequence, not `leave; ret`), wraps it in a real ELF64 image via
        // build_elf64_standalone(), writes those real bytes to a real
        // path on the real on-disk filesystem, then fork()s+exec()s+
        // wait()s for real (write_exec_and_check() above) -- the CHILD
        // process's entire image is replaced by the kernel's own real,
        // UNCHANGED exec() syscall (Milestone 36/45) running the file
        // this SAME cc.elf process just wrote, and the PARENT observes
        // the real exit code that child's own compiled `return` produced,
        // via the kernel's own real wait() syscall (Milestone 43) --
        // not the in-process function-pointer shortcut Milestone 68 used
        // (still available above, still exercised by CASE 4/5/6/7,
        // completely unchanged): (8) reuses CASE 1's already-verified
        // func1 AST, recompiled in Standalone mode (proving the SAME
        // frontend AST now has TWO independently real backends), expected
        // exit code 42; (9) a fresh source, two variables, '*' and '-'
        // (int p; int q; p = 9; q = p * 3; return q - 2; -- hand-computed
        // p=9, q=27, 27-2=25), proving the real on-disk/exec/wait path is
        // general, not hard-coded to CASE 8's own shape; (10) a fresh
        // source exercising '/' (the trickiest encoding, cqo+idiv) with
        // real precedence, through the SAME real on-disk/exec/wait path
        // (int x; x = 100; return x / 4 + 3; -- hand-computed 100/4=25,
        // 25+3=28).
        // ===============================================================
        w(b"milestone 69: cc ELF64 writer + real exec() (real on-disk executable, real kernel exec()) starting\n");

        // -------------------------------------------------------------
        // CASE 8: reuse CASE 1's func1 (int x; x = 40 + 2; return x;),
        // recompiled in Standalone mode. Hand-computed expected exit
        // code: 42 (same value CASE 4 already proved in-process).
        // -------------------------------------------------------------
        const PATH8: &[u8] = b"ccout1";
        let case8_ok = if parse1_ok && ast1_ok {
            match gen_function(src1_ptr, func1, CodegenMode::Standalone) {
                Ok(buf8) => {
                    let (elf8_ptr, elf8_len) = build_elf64_standalone(buf8.ptr, buf8.len, 0);
                    write_exec_and_check!(PATH8, elf8_ptr, elf8_len, 42)
                }
                Err(_) => {
                    w(b"  case8 gen_function(func1, Standalone) returned Err unexpectedly\n");
                    false
                }
            }
        } else {
            w(b"  case8 skipped -- CASE 1's own func1 was not valid\n");
            false
        };
        write_check(b"case8_real_elf_exec_returns_42=", case8_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 9: a fresh source, two variables, '*' and '-'.
        //   int main() {
        //       int p;
        //       int q;
        //       p = 9;
        //       q = p * 3;
        //       return q - 2;
        //   }
        // Hand-computed: p=9, q=9*3=27, return 27-2=25.
        // -------------------------------------------------------------
        const SRC9: &[u8] = b"int main() {\n    int p;\n    int q;\n    p = 9;\n    q = p * 3;\n    return q - 2;\n}\n";
        const PATH9: &[u8] = b"ccout2";
        let (elf9_ptr, elf9_len) = compile_standalone_elf(SRC9.as_ptr() as u64, SRC9.len() as u64);
        let case9_ok = if elf9_ptr == 0 {
            w(b"  case9 compile_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH9, elf9_ptr, elf9_len, 25)
        };
        write_check(b"case9_fresh_source_real_elf_exec_returns_25=", case9_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 10: a fresh source, '/' and real precedence, through the
        // SAME real on-disk-ELF + real exec() + real wait() path.
        //   int main() {
        //       int x;
        //       x = 100;
        //       return x / 4 + 3;
        //   }
        // Hand-computed: 100 / 4 = 25, 25 + 3 = 28.
        // -------------------------------------------------------------
        const SRC10: &[u8] = b"int main() {\n    int x;\n    x = 100;\n    return x / 4 + 3;\n}\n";
        // Real, disclosed reuse -- NOT a fresh 'ccout3' path: checked
        // directly against a real boot log before choosing this, not
        // guessed -- fs.rs's real on-disk directory has a fixed
        // MAX_ENTRIES cap (8), and by CASE 10 every OTHER milestone's own
        // seeded test files (testelf, altentry, pipetest, argvtarget,
        // stdiotest, ...) plus CASE 8/9's own 'ccout1'/'ccout2' already
        // fill it -- confirmed the hard way, a real
        // `fs::write_file('ccout3') FAILED: directory full (max 8
        // entries)` on an actual boot. Reusing CASE 8's own 'ccout1' path
        // costs nothing real: by the time CASE 10 runs, CASE 8's child
        // has already exec()'d and exited, so 'ccout1' is an ordinary,
        // unopened file on disk -- open()/fdwrite()/close() overwrite its
        // content in place, no new directory entry needed, and CASE 10's
        // own fork()+exec()+wait() below still exercises the exact same
        // real on-disk-ELF + real kernel exec() path CASE 8/9 already
        // proved, just against a recycled filename.
        const PATH10: &[u8] = PATH8;
        let (elf10_ptr, elf10_len) = compile_standalone_elf(SRC10.as_ptr() as u64, SRC10.len() as u64);
        let case10_ok = if elf10_ptr == 0 {
            w(b"  case10 compile_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH10, elf10_ptr, elf10_len, 28)
        };
        write_check(b"case10_division_real_elf_exec_returns_28=", case10_ok);

        w(b"\n");

        let overall_m69 = case8_ok && case9_ok && case10_ok;
        w(b"OVERALL_M69=");
        w(if overall_m69 { b"PASS" } else { b"FAIL" });
        w(b"\n\n");

        // ===============================================================
        // MILESTONE 70: comparison operators + if/else -- the subset-C
        // grammar's first real control flow. Seven cases: (11) proves ALL
        // SIX comparison operators produce the real hand-computed 0/1 in
        // one combined, decimal-digit-per-operator compiled program (no
        // if/else involved yet), executed in-process (CodegenMode::
        // Callable, same shortcut CASE 4/5/6/7 use) -- combined into ONE
        // case rather than six/two separate ones as a real, disclosed
        // size trade-off (see its own comment below: cc.elf's own
        // compiled size is a real, hard cap this milestone had to budget
        // against); (13)/(14) are the SAME if/else SOURCE compiled and
        // run twice with different input data, genuinely exercising BOTH
        // the true and the false branch of one real conditional-jump
        // program, still in-process; (15)/(16) are an if WITHOUT an else
        // (the single-jz, no-jmp codegen shape), also both branches,
        // proving the fallthrough-when-false case emits correctly and the
        // trailing safety-backstop epilogue is never needed if the body's
        // own `return` already ran; (17)/(18) re-run the SAME if/else
        // source as (13)/(14) but through the real on-disk-ELF + real
        // kernel exec() + real wait() path Milestone 69 established
        // (write_exec_and_check!), the strongest verification this tier
        // has -- a real compiled program with genuine control flow,
        // written to a real file, exec()'d by the kernel's own real
        // exec() syscall, both branches, not just the in-process
        // shortcut.
        // ===============================================================
        w(b"milestone 70: cc comparisons + if/else (real conditional jumps, both branches) starting\n");

        // -------------------------------------------------------------
        // CASE 11: all SIX comparison operators, standalone (no if/else
        // yet), in ONE compiled+executed program -- a real, deliberate
        // size-vs-case-count trade-off (see this milestone's own README
        // disclosure: cc.elf's own compiled size is a real, hard cap --
        // process::MAX_PAGES_PER_ELF_SEGMENT -- and eighteen separate
        // fully-wrapped cases would not fit under it). Each comparison's
        // real 0/1 result is weighted by a distinct power of ten and
        // summed, so the single decimal result still hand-verifies EVERY
        // operator independently (a wrong bit anywhere changes a
        // different decimal digit): a=5, b=3 -- a<b=0, a>b=1, a==b=0,
        // a!=b=1, a<=b=0, a>=b=1 -- hand-computed
        // 0 + 1*10 + 0*100 + 1*1000 + 0*10000 + 1*100000 = 101010.
        // -------------------------------------------------------------
        const SRC11: &[u8] =
            b"int main(){int a;int b;a=5;b=3;return (a<b)+(a>b)*10+(a==b)*100+(a!=b)*1000+(a<=b)*10000+(a>=b)*100000;}";
        let case11_result = compile_and_run_callable(SRC11.as_ptr() as u64, SRC11.len() as u64);
        w(b"  case11 (all 6 comparisons, a=5,b=3) returned=");
        if let Some(r) = case11_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 101010)\n");
        write_check(b"case11_all_six_comparisons_return_101010=", case11_result == Some(101010));

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 13/14: the SAME if/else source, compiled and run TWICE
        // with different input data -- genuinely exercising both real
        // branches of one real conditional-jump program, in-process.
        //   int main() { int x; x = <10 or 2>;
        //       if (x > 5) { return 1; } else { return 0; } }
        // -------------------------------------------------------------
        const SRC13: &[u8] = b"int main(){int x;x=10;if(x>5){return 1;}else{return 0;}}";
        let case13_result = compile_and_run_callable(SRC13.as_ptr() as u64, SRC13.len() as u64);
        w(b"  case13 (x=10, TRUE branch) returned=");
        if let Some(r) = case13_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        write_check(b"case13_if_else_true_branch_returns_1=", case13_result == Some(1));

        w(b"\n");

        const SRC14: &[u8] = b"int main(){int x;x=2;if(x>5){return 1;}else{return 0;}}";
        let case14_result = compile_and_run_callable(SRC14.as_ptr() as u64, SRC14.len() as u64);
        w(b"  case14 (x=2, FALSE branch) returned=");
        if let Some(r) = case14_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 0)\n");
        write_check(b"case14_if_else_false_branch_returns_0=", case14_result == Some(0));

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 15/16: an if WITHOUT an else -- the single-jz, no-jmp
        // codegen shape (see gen_stmt_list()'s own doc comment) --
        // same source, run twice with different data, both branches.
        //   int main() { int x; int y; x = <1 or 0>; y = <0 or 7>;
        //       if (x == 1) { y = 42; } return y; }
        // -------------------------------------------------------------
        const SRC15: &[u8] = b"int main(){int x;int y;x=1;y=0;if(x==1){y=42;}return y;}";
        let case15_result = compile_and_run_callable(SRC15.as_ptr() as u64, SRC15.len() as u64);
        w(b"  case15 (x=1, TRUE branch taken) returned=");
        if let Some(r) = case15_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 42)\n");
        write_check(b"case15_if_no_else_true_branch_returns_42=", case15_result == Some(42));

        w(b"\n");

        const SRC16: &[u8] = b"int main(){int x;int y;x=0;y=7;if(x==1){y=42;}return y;}";
        let case16_result = compile_and_run_callable(SRC16.as_ptr() as u64, SRC16.len() as u64);
        w(b"  case16 (x=0, FALSE branch, fell through) returned=");
        if let Some(r) = case16_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 7)\n");
        write_check(b"case16_if_no_else_false_branch_returns_7=", case16_result == Some(7));

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 17/18: the SAME if/else source as CASE 13/14, through the
        // real on-disk-ELF + real kernel exec() + real wait() path
        // (write_exec_and_check!, Milestone 69) -- the strongest real
        // verification available: a genuinely compiled program with real
        // control flow, written to a real file, exec()'d by the kernel's
        // own real, unmodified exec() syscall, BOTH branches. Reuses
        // CASE 8/10's own 'ccout1' path (see CASE 10's own doc comment
        // for exactly why -- fs.rs's real 8-entry directory cap is
        // already at capacity by this point in the same boot); by CASE
        // 17's turn, CASE 10's own child has already exec()'d and
        // exited, so 'ccout1' is again an ordinary, unopened file.
        // -------------------------------------------------------------
        const PATH17: &[u8] = PATH8;
        let (elf17_ptr, elf17_len) = compile_standalone_elf(SRC13.as_ptr() as u64, SRC13.len() as u64);
        let case17_ok = if elf17_ptr == 0 {
            w(b"  case17 compile_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH17, elf17_ptr, elf17_len, 1)
        };
        write_check(b"case17_real_elf_exec_if_else_true_branch_returns_1=", case17_ok);

        w(b"\n");

        const PATH18: &[u8] = PATH8;
        let (elf18_ptr, elf18_len) = compile_standalone_elf(SRC14.as_ptr() as u64, SRC14.len() as u64);
        let case18_ok = if elf18_ptr == 0 {
            w(b"  case18 compile_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH18, elf18_ptr, elf18_len, 0)
        };
        write_check(b"case18_real_elf_exec_if_else_false_branch_returns_0=", case18_ok);

        w(b"\n");

        let case11_ok = case11_result == Some(101010);
        let case13_ok = case13_result == Some(1);
        let case14_ok = case14_result == Some(0);
        let case15_ok = case15_result == Some(42);
        let case16_ok = case16_result == Some(7);

        let overall_m70 = case11_ok && case13_ok && case14_ok && case15_ok && case16_ok && case17_ok && case18_ok;
        w(b"OVERALL_M70=");
        w(if overall_m70 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // ===============================================================
        // MILESTONE 71: while-loops -- real backward-jump codegen
        // (emit_jmp_back(), see CodeBuf's own doc comment above) on top
        // of Milestone 70's forward-patch jz/jmp machinery, reused
        // unchanged for the loop's own exit branch. Four cases: (19) the
        // ordinary path -- a real sum-1-to-5 loop, condition true
        // several times then false, exercising both the backward jump
        // AND the forward exit jump in the same run; (20) the
        // zero-iteration edge case -- condition false on the very FIRST
        // check, proving the backward jmp is never even reached/executed
        // when the loop body never runs; (21) a while LOOP containing an
        // IF (combining Milestone 70 and 71 control flow in one program,
        // nested forward+backward jump patching); (22) the SAME source as
        // (19), through the real on-disk-ELF + real kernel exec() + real
        // wait() path (write_exec_and_check!, Milestone 69/70's own
        // strongest verification), reusing PATH8 ('ccout1') the same
        // real reason CASE 10/17/18's own doc comments already give (by
        // this point in the boot, CASE 18's own child has already exec()'d
        // and exited, so the path is again an ordinary, unopened file).
        // ===============================================================
        w(b"milestone 71: cc while-loops (real backward-jump codegen) starting\n");

        // -------------------------------------------------------------
        // CASE 19: int main(){int i;int sum;i=1;sum=0;
        //              while(i<=5){sum=sum+i;i=i+1;}return sum;}
        // Hand-computed: 1+2+3+4+5 = 15.
        // -------------------------------------------------------------
        const SRC19: &[u8] = b"int main(){int i;int sum;i=1;sum=0;while(i<=5){sum=sum+i;i=i+1;}return sum;}";
        let case19_result = compile_and_run_callable(SRC19.as_ptr() as u64, SRC19.len() as u64);
        w(b"  case19 (sum 1..5 via while) returned=");
        if let Some(r) = case19_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 15)\n");
        write_check(b"case19_while_sum_1_to_5_returns_15=", case19_result == Some(15));

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 20: same shape, condition FALSE on the first check -- the
        // loop body must never run, proving the forward exit jz alone
        // (no backward jmp ever taken) is correct on its own.
        //   int main(){int i;int sum;i=10;sum=99;
        //       while(i<5){sum=0;i=i+1;}return sum;}
        // -------------------------------------------------------------
        const SRC20: &[u8] = b"int main(){int i;int sum;i=10;sum=99;while(i<5){sum=0;i=i+1;}return sum;}";
        let case20_result = compile_and_run_callable(SRC20.as_ptr() as u64, SRC20.len() as u64);
        w(b"  case20 (while condition false from the start, zero iterations) returned=");
        if let Some(r) = case20_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 99, unchanged)\n");
        write_check(b"case20_while_zero_iterations_returns_99=", case20_result == Some(99));

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 21: an if INSIDE a while -- Milestone 70's forward-patched
        // jz/jmp and Milestone 71's forward exit jz + backward jmp all in
        // one program, nested.
        //   int main(){int i;int count;i=0;count=0;
        //       while(i<10){if(i==3){count=count+1;}i=i+1;}return count;}
        // count is incremented exactly once (when i==3), so expected 1.
        // -------------------------------------------------------------
        const SRC21: &[u8] = b"int main(){int i;int count;i=0;count=0;while(i<10){if(i==3){count=count+1;}i=i+1;}return count;}";
        let case21_result = compile_and_run_callable(SRC21.as_ptr() as u64, SRC21.len() as u64);
        w(b"  case21 (if nested inside while, counts i==3 only) returned=");
        if let Some(r) = case21_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        write_check(b"case21_if_nested_in_while_returns_1=", case21_result == Some(1));

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 22: the SAME source as CASE 19, through the real
        // on-disk-ELF + real kernel exec() + real wait() path -- the
        // strongest real verification available for a genuine loop, not
        // just the in-process function-pointer shortcut.
        // -------------------------------------------------------------
        const PATH22: &[u8] = PATH8;
        let (elf22_ptr, elf22_len) = compile_standalone_elf(SRC19.as_ptr() as u64, SRC19.len() as u64);
        let case22_ok = if elf22_ptr == 0 {
            w(b"  case22 compile_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH22, elf22_ptr, elf22_len, 15)
        };
        write_check(b"case22_real_elf_exec_while_sum_1_to_5_returns_15=", case22_ok);

        w(b"\n");

        let case19_ok = case19_result == Some(15);
        let case20_ok = case20_result == Some(99);
        let case21_ok = case21_result == Some(1);

        let overall_m71 = case19_ok && case20_ok && case21_ok && case22_ok;
        w(b"OVERALL_M71=");
        w(if overall_m71 { b"PASS" } else { b"FAIL" });
        w(b"\n\n");

        // ===============================================================
        // MILESTONE 72: real function parameters and function calls --
        // Tier 3's sixth slice, and the "next genuinely bigger step"
        // Milestone 71's own closing disclosure named. A real x86_64
        // calling convention (integer arguments 1-4 in RDI/RSI/RDX/RCX,
        // chosen to extend this kernel's own real syscall-ABI register
        // ordering rather than invent a new one -- see param_reg()'s own
        // doc comment above), real per-function stack frames (parameters
        // spilled to their own rbp-relative slots in the callee's own
        // prologue, sharing the same flat slot scheme locals already
        // use), and real call-site codegen (push every evaluated argument,
        // pop them back in reverse into the real argument registers, a
        // real `call rel32`, an ordinary callee `leave; ret` bringing
        // control back with the result already in RAX). Honestly scoped:
        // exactly `program := function+` up to MAX_FUNCS (4) functions,
        // up to MAX_PARAMS (4) integer parameters per function/arguments
        // per call, a callee must be defined at or before its own caller
        // in source order (self-recursion is mechanically reachable but
        // NOT tested/verified; forward calls and mutual recursion are not
        // supported at all yet -- see gen_program()'s own doc comment
        // above for the full disclosure). Six cases: (23) a real
        // multi-function AST-shape check (parse_program() builds a real
        // 2-function list, in real source order, with the real declared
        // parameter names/count on each); (24) the ordinary in-process
        // call path -- a 2-parameter function combining both its
        // arguments in a way that is only correct if BOTH were passed and
        // read in the right order (`combine(7,5) = 7*10+5 = 75`; a
        // swapped-argument bug would instead print 57); (25) and (26)
        // the same idea generalized to 3 and 4 parameters respectively,
        // exercising RDX and RCX (the two argument registers CASE 24
        // alone never reaches); (27) a call used as another call's own
        // ARGUMENT expression, plus a variable as an argument, combining
        // several real codegen paths (gen_expr's own recursion, a local
        // variable read, and two separate real `call` sites) in one
        // program; (28) CASE 24's exact same source, through the real
        // on-disk-ELF + kernel exec() + wait() path Milestone 69
        // established -- the strongest real verification available, same
        // as CASE 22/CASE 18's own precedent; (29) a deliberate SEMANTIC
        // error -- a call to a name matching no declared function --
        // proving gen_expr()'s new CodeGenError::UndeclaredFunction path
        // is real, the same "prove the Err path is real" discipline
        // CASE 2/3/7 already established.
        // ===============================================================
        w(b"milestone 72: cc function parameters and calls (real x86_64 calling convention) starting\n");

        // -------------------------------------------------------------
        // CASE 23: real multi-function parse -- AST shape check.
        //   int combine(int a, int b) { return a * 10 + b; }
        //   int main() { return combine(7, 5); }
        // Hand-verified expected shape: a 2-function list, in source
        // order -- first "combine" (2 parameters, "a" then "b"), then
        // "main" (0 parameters), "main".next == 0 (end of list).
        // -------------------------------------------------------------
        const SRC23: &[u8] = b"int combine(int a, int b) { return a * 10 + b; } int main() { return combine(7, 5); }";
        let src23_ptr = SRC23.as_ptr() as u64;
        let src23_len = SRC23.len() as u64;
        let toks23_ptr = malloc((MAX_TOKENS * core::mem::size_of::<Token>()) as u64);
        let lex23 = lex(src23_ptr, src23_len, toks23_ptr, MAX_TOKENS as u64);
        let case23_ok = match lex23 {
            Ok(ntoks23) => {
                let mut parser23 = Parser { toks_ptr: toks23_ptr, ntoks: ntoks23, pos: 0 };
                match parser23.parse_program() {
                    Ok(prog23) => {
                        let f_combine = core::ptr::read(prog23 as *const FuncDef);
                        let combine_name_ok = word_is(src23_ptr, f_combine.name_off as u64, f_combine.name_len as u64, b"combine");
                        let combine_params_ok = f_combine.param_count == 2
                            && word_is(src23_ptr, param_read(f_combine.params_ptr, 0).name_off as u64, param_read(f_combine.params_ptr, 0).name_len as u64, b"a")
                            && word_is(src23_ptr, param_read(f_combine.params_ptr, 1).name_off as u64, param_read(f_combine.params_ptr, 1).name_len as u64, b"b");
                        write_check(b"case23_first_func_is_combine_with_2_params=", combine_name_ok && combine_params_ok);
                        if f_combine.next == 0 {
                            w(b"  case23 combine.next was 0, expected a linked 'main'\n");
                            false
                        } else {
                            let f_main = core::ptr::read(f_combine.next as *const FuncDef);
                            let main_ok = word_is(src23_ptr, f_main.name_off as u64, f_main.name_len as u64, b"main")
                                && f_main.param_count == 0
                                && f_main.next == 0;
                            write_check(b"case23_second_func_is_main_with_0_params_and_ends_list=", main_ok);
                            combine_name_ok && combine_params_ok && main_ok
                        }
                    }
                    Err(_) => {
                        w(b"  case23 parse_program() failed unexpectedly\n");
                        false
                    }
                }
            }
            Err(_) => {
                w(b"  case23 lex() failed unexpectedly\n");
                false
            }
        };
        write_check(b"case23_multifunction_ast_shape=", case23_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 24: the ordinary in-process call path -- a real 2-argument
        // function whose result is only correct if BOTH arguments were
        // passed AND read in the right order.
        //   int combine(int a, int b) { return a * 10 + b; }
        //   int main() { return combine(7, 5); }
        // Hand-computed: 7*10+5 = 75 (a swapped a/b would give 5*10+7=57
        // instead -- a real, distinguishing check, not just "some number
        // came back").
        // -------------------------------------------------------------
        const SRC24: &[u8] = SRC23;
        let case24_result = compile_and_run_program_callable(SRC24.as_ptr() as u64, SRC24.len() as u64);
        w(b"  case24 (combine(7,5) = 7*10+5, real 2-param call) returned=");
        if let Some(r) = case24_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 75)\n");
        let case24_ok = case24_result == Some(75);
        write_check(b"case24_two_param_call_returns_75=", case24_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 25: three parameters (exercises RDX, the third argument
        // register, for the first time).
        //   int mix(int a, int b, int c) { return a * 100 + b * 10 + c; }
        //   int main() { return mix(1, 2, 3); }
        // Hand-computed: 1*100+2*10+3 = 123.
        // -------------------------------------------------------------
        const SRC25: &[u8] =
            b"int mix(int a, int b, int c) { return a * 100 + b * 10 + c; } int main() { return mix(1, 2, 3); }";
        let case25_result = compile_and_run_program_callable(SRC25.as_ptr() as u64, SRC25.len() as u64);
        w(b"  case25 (mix(1,2,3), real 3-param call) returned=");
        if let Some(r) = case25_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 123)\n");
        let case25_ok = case25_result == Some(123);
        write_check(b"case25_three_param_call_returns_123=", case25_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 26: four parameters (exercises RCX, the fourth and last
        // register this milestone's own convention supports).
        //   int mix4(int a,int b,int c,int d){return a*1000+b*100+c*10+d;}
        //   int main() { return mix4(1, 2, 3, 4); }
        // Hand-computed: 1*1000+2*100+3*10+4 = 1234.
        // -------------------------------------------------------------
        const SRC26: &[u8] = b"int mix4(int a, int b, int c, int d) { return a * 1000 + b * 100 + c * 10 + d; } int main() { return mix4(1, 2, 3, 4); }";
        let case26_result = compile_and_run_program_callable(SRC26.as_ptr() as u64, SRC26.len() as u64);
        w(b"  case26 (mix4(1,2,3,4), real 4-param call) returned=");
        if let Some(r) = case26_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1234)\n");
        let case26_ok = case26_result == Some(1234);
        write_check(b"case26_four_param_call_returns_1234=", case26_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 27: a call used as another call's own argument expression,
        // plus a local variable as an argument -- combining gen_expr's
        // own recursion, a variable read, and two separate real `call`
        // sites in one program.
        //   int add(int a, int b) { return a + b; }
        //   int main() { int x; x = 5; return add(x, add(1, 2)); }
        // Hand-computed: add(1,2) = 3; add(5, 3) = 8.
        // -------------------------------------------------------------
        const SRC27: &[u8] = b"int add(int a, int b) { return a + b; } int main() { int x; x = 5; return add(x, add(1, 2)); }";
        let case27_result = compile_and_run_program_callable(SRC27.as_ptr() as u64, SRC27.len() as u64);
        w(b"  case27 (add(x, add(1,2)) with x=5, nested call as argument) returned=");
        if let Some(r) = case27_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 8)\n");
        let case27_ok = case27_result == Some(8);
        write_check(b"case27_nested_call_as_argument_returns_8=", case27_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 28: CASE 24's exact same source, through the real
        // on-disk-ELF + real kernel exec() + real wait() path -- the
        // strongest real verification available for a genuine
        // multi-function program with real parameter passing, not just
        // the in-process function-pointer shortcut. Reuses PATH8
        // ('ccout1') the same real reason CASE 10/17/18/22's own doc
        // comments already give.
        // -------------------------------------------------------------
        const PATH28: &[u8] = PATH8;
        let (elf28_ptr, elf28_len) = compile_program_standalone_elf(SRC24.as_ptr() as u64, SRC24.len() as u64);
        let case28_ok = if elf28_ptr == 0 {
            w(b"  case28 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH28, elf28_ptr, elf28_len, 75)
        };
        write_check(b"case28_real_elf_exec_two_param_call_returns_75=", case28_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 29: a deliberate SEMANTIC error -- a call to `foo`, a name
        // matching no declared function.
        //   int main() { return foo(1, 2); }
        // Hand-computed byte offset of `foo` in the source below: counting
        // "int main() { return foo(1, 2); }" byte by byte, `f` is the
        // 21st byte, offset 20 (0-indexed) -- real predicted failure:
        // Err(UndeclaredFunction(20)).
        // -------------------------------------------------------------
        const SRC29: &[u8] = b"int main() { return foo(1, 2); }";
        let src29_ptr = SRC29.as_ptr() as u64;
        let src29_len = SRC29.len() as u64;
        let case29_ok = match lex_and_parse_program(src29_ptr, src29_len) {
            Ok(prog29) => match gen_program(src29_ptr, prog29, CodegenMode::Callable) {
                Err(CodeGenError::UndeclaredFunction(off)) => {
                    w(b"  case29 real codegen error -- undeclared function at byte offset=");
                    write_u64_dec(off);
                    w(b"\n");
                    off == 20
                }
                _ => {
                    w(b"  case29 expected Err(UndeclaredFunction(20)), got something else\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case29 lex_and_parse_program() returned Err unexpectedly (source should lex+parse fine)\n");
                false
            }
        };
        write_check(b"case29_undeclared_function_error_at_offset20=", case29_ok);

        w(b"\n");

        let overall_m72 =
            case23_ok && case24_ok && case25_ok && case26_ok && case27_ok && case28_ok && case29_ok;
        w(b"OVERALL_M72=");
        w(if overall_m72 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // ===============================================================
        // MILESTONE 73: real verification (not new grammar) of direct
        // self-recursion -- the one item Milestone 72's own closing
        // disclosure explicitly left "mechanically reachable but NOT
        // tested or verified". Two cases, both using ONLY grammar that
        // already existed before this milestone (if/else, arithmetic, one
        // int parameter): (30) the ordinary in-process call path, a
        // function that calls ITSELF with a decremented argument and adds
        // its own parameter to the recursive result AFTER that call
        // returns -- a real, distinguishing check, not just "some number
        // came back": if the recursive call's own fresh stack frame
        // somehow clobbered the CALLER's own `n` slot instead of getting
        // its own independent one (the exact kind of bug a naive/shared-
        // frame implementation would have), the result would not be 55;
        // (31) a DIFFERENT self-recursive function (multiplication, not
        // addition, so a case-30-only bug that happened to cancel out
        // wouldn't also cancel out here) run through the real on-disk-ELF
        // + kernel exec() + wait() path Milestone 69 established -- the
        // strongest verification tier, same as Milestone 70/71/72's own
        // precedent. Both use small, hand-verified recursion depths (10
        // and 5 respectively) -- see this file's own top Milestone 73 doc
        // comment for why deeper self-recursion is a real, disclosed,
        // UNMEASURED risk against this kernel's single 4KiB per-process
        // stack page, not attempted here.
        // ===============================================================
        w(b"milestone 73: cc direct self-recursion verification starting\n");

        // -------------------------------------------------------------
        // CASE 30: the ordinary in-process call path -- a function that
        // calls itself with a decremented argument, adding its own
        // parameter to the recursive result AFTER the recursive call
        // returns (so `n` must survive across that nested call, in ITS
        // OWN caller-frame slot, not a shared/global one).
        //   int sum_to(int n) { if (n == 0) { return 0; }
        //                       else { return n + sum_to(n - 1); } }
        //   int main() { return sum_to(10); }
        // Hand-computed: 10+9+8+...+1+0 = 55. Recursion depth 11 (n = 10
        // down to 0) -- a real, small, hand-verified depth.
        // -------------------------------------------------------------
        const SRC30: &[u8] =
            b"int sum_to(int n) { if (n == 0) { return 0; } else { return n + sum_to(n - 1); } } int main() { return sum_to(10); }";
        let case30_result = compile_and_run_program_callable(SRC30.as_ptr() as u64, SRC30.len() as u64);
        w(b"  case30 (sum_to(10) = 10+9+...+0, real self-recursive call) returned=");
        if let Some(r) = case30_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 55)\n");
        let case30_ok = case30_result == Some(55);
        write_check(b"case30_self_recursive_sum_returns_55=", case30_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 31: a DIFFERENT self-recursive function (multiplication,
        // not addition -- a case-30-only bug that happened to cancel out
        // wouldn't also cancel out here), run through the real
        // on-disk-ELF + real kernel exec() + real wait() path -- the
        // strongest real verification available, same as CASE 28's own
        // precedent. Reuses PATH8 ('ccout1'), the same real reason
        // CASE 10/17/18/22/28's own doc comments already give.
        //   int fact(int n) { if (n == 0) { return 1; }
        //                     else { return n * fact(n - 1); } }
        //   int main() { return fact(5); }
        // Hand-computed: 5*4*3*2*1*1 = 120. Recursion depth 6 (n = 5 down
        // to 0) -- a real, small, hand-verified depth, well under this
        // kernel's single 4KiB per-process stack page.
        // -------------------------------------------------------------
        const SRC31: &[u8] =
            b"int fact(int n) { if (n == 0) { return 1; } else { return n * fact(n - 1); } } int main() { return fact(5); }";
        const PATH31: &[u8] = PATH8;
        let (elf31_ptr, elf31_len) = compile_program_standalone_elf(SRC31.as_ptr() as u64, SRC31.len() as u64);
        let case31_ok = if elf31_ptr == 0 {
            w(b"  case31 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH31, elf31_ptr, elf31_len, 120)
        };
        write_check(b"case31_real_elf_exec_self_recursive_factorial_returns_120=", case31_ok);

        w(b"\n");

        let overall_m73 = case30_ok && case31_ok;
        w(b"OVERALL_M73=");
        w(if overall_m73 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // ===============================================================
        // MILESTONE 74: real verification of genuine FORWARD calls and
        // genuine MUTUAL recursion -- the real dependency-order
        // restriction Milestone 72 disclosed and Milestone 73 explicitly
        // left unattempted ("a genuinely UNSUPPORTED case, not just
        // untested"). Two cases, both using ONLY grammar that already
        // existed before this milestone (if/else, arithmetic, one int
        // parameter, function calls): (32) the ordinary in-process call
        // path, `main` (defined FIRST) calling a helper function defined
        // AFTER it -- impossible under Milestone 72/73's own source-order
        // restriction, a real, distinguishing check of the forward-patch
        // machinery itself (a wrong rel32 patch would jump into garbage
        // mid-instruction or into an unrelated function's own bytes, not
        // silently produce a near-miss number); (33) genuine MUTUAL
        // recursion -- is_even() (defined first) calls is_odd() (a real
        // FORWARD reference, defined after it) and is_odd() calls
        // is_even() (a real BACKWARD reference, resolved immediately) --
        // through the real on-disk-ELF + kernel exec() + wait() path
        // Milestone 69 established, the strongest verification tier, same
        // as Milestone 70-73's own precedent. Recursion depth 11
        // (is_even(10) alternates down through is_odd(9), is_even(8), ...,
        // is_even(0)) -- the same real, small, hand-verified envelope
        // Milestone 73's own CASE 30 already used and disclosed, not a
        // new or larger stack-depth risk.
        // ===============================================================
        w(b"milestone 74: cc forward-call and mutual-recursion verification starting\n");

        // -------------------------------------------------------------
        // CASE 32: the ordinary in-process call path -- `main` is defined
        // FIRST in the source and calls `add5`, defined AFTER it -- a
        // genuine forward reference, mechanically impossible to express
        // under Milestone 72/73's own "callee must be defined at or
        // before its own caller" restriction (there is no way to reorder
        // this source to avoid it: `main` must call `add5`, and `add5`'s
        // own body does not call `main`).
        //   int main() { return add5(10); }
        //   int add5(int x) { return x + 5; }
        // Hand-computed: 10 + 5 = 15.
        // -------------------------------------------------------------
        const SRC32: &[u8] = b"int main() { return add5(10); } int add5(int x) { return x + 5; }";
        let case32_result = compile_and_run_program_callable(SRC32.as_ptr() as u64, SRC32.len() as u64);
        w(b"  case32 (main calls add5, defined AFTER it -- real forward call) returned=");
        if let Some(r) = case32_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 15)\n");
        let case32_ok = case32_result == Some(15);
        write_check(b"case32_forward_call_add5_returns_15=", case32_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 33: genuine MUTUAL recursion, run through the real
        // on-disk-ELF + real kernel exec() + real wait() path -- the
        // strongest real verification available, same as CASE 31's own
        // precedent. Reuses PATH8 ('ccout1'), the same real reason
        // CASE 10/17/18/22/28/31's own doc comments already give.
        //   int is_even(int n) { if (n == 0) { return 1; }
        //                         else { return is_odd(n - 1); } }
        //   int is_odd(int n) { if (n == 0) { return 0; }
        //                        else { return is_even(n - 1); } }
        //   int main() { return is_even(10); }
        // `is_even`'s own call to `is_odd` is a real FORWARD reference
        // (is_odd is defined AFTER is_even in the source, so is_odd's own
        // code_off is still the real UNRESOLVED_CODE_OFF sentinel at the
        // point is_even's own body is compiled -- exercising this
        // milestone's own new placeholder/patch-list path). `is_odd`'s
        // own call to `is_even` is a real BACKWARD reference (is_even was
        // already compiled by the time is_odd's own body compiles --
        // exercising the ORIGINAL, unmodified emit_call() fast path).
        // Both directions of the SAME mutually-recursive pair are real
        // and exercised in one program, not just one direction in
        // isolation. Hand-computed: is_even(10) -> is_odd(9) -> is_even(8)
        // -> ... -> is_even(0) = 1 (true) -- 11 real stack frames,
        // alternating between the two functions, the same real, small,
        // hand-verified envelope CASE 30 already used.
        // -------------------------------------------------------------
        const SRC33: &[u8] =
            b"int is_even(int n) { if (n == 0) { return 1; } else { return is_odd(n - 1); } } int is_odd(int n) { if (n == 0) { return 0; } else { return is_even(n - 1); } } int main() { return is_even(10); }";
        const PATH33: &[u8] = PATH8;
        let (elf33_ptr, elf33_len) = compile_program_standalone_elf(SRC33.as_ptr() as u64, SRC33.len() as u64);
        let case33_ok = if elf33_ptr == 0 {
            w(b"  case33 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH33, elf33_ptr, elf33_len, 1)
        };
        write_check(b"case33_real_elf_exec_mutual_recursion_is_even_10_returns_1=", case33_ok);

        w(b"\n");

        let overall_m74 = case32_ok && case33_ok;
        w(b"OVERALL_M74=");
        w(if overall_m74 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // ===============================================================
        // MILESTONE 75: real, end-to-end verification of this kernel's
        // real stack-overflow safety -- the one significant OPEN SAFETY
        // item Milestone 73's own closing disclosure first flagged and
        // Milestone 74's own closing disclosure re-confirmed still open,
        // now closed by a real kernel-side diagnostic
        // (`kernel/src/interrupts.rs`'s `STACK_GUARD_REGION_SIZE`) plus
        // this one new case. See this file's own top Milestone 75 doc
        // comment for the full design reasoning (why the underlying
        // safety already existed as a byproduct of this kernel's virtual
        // address layout, and why this milestone verifies rather than
        // reinvents it).
        // ===============================================================
        w(b"milestone 75: cc real stack-overflow safety verification starting\n");

        // -------------------------------------------------------------
        // CASE 34: genuine, UNCONDITIONAL, unbounded self-recursion --
        // no base case at all, so this program can only terminate one of
        // two ways: run off this kernel's single 4 KiB per-process stack
        // page (the real, intended, distinguishing outcome), or hang
        // forever. Run through the real on-disk-ELF + kernel exec() +
        // wait() path (Milestone 69) -- deliberately NOT the in-process
        // Callable path (see this file's own top Milestone 75 doc
        // comment for exactly why: that path runs on cc.elf's OWN
        // current stack, in cc.elf's OWN process, and would take down
        // this very self-test harness before OVERALL_M75 could ever
        // print). Success here means the real kernel `wait()` syscall
        // reports `WaitOutcome::Signaled` (bit 17 of the encoding
        // usertest.rs's own syscall dispatch documents) and specifically
        // NOT `WaitOutcome::Exited` -- a program with no base case
        // reaching its own exit() would itself be a serious,
        // distinguishing bug, not a plausible success path.
        //   int spin(int n) { return spin(n + 1); }
        //   int main() { return spin(0); }
        // -------------------------------------------------------------
        const SRC34: &[u8] = b"int spin(int n) { return spin(n + 1); } int main() { return spin(0); }";
        const PATH34: &[u8] = PATH8;
        let (elf34_ptr, elf34_len) = compile_program_standalone_elf(SRC34.as_ptr() as u64, SRC34.len() as u64);
        let case34_ok = if elf34_ptr == 0 {
            w(b"  case34 compile_program_standalone_elf failed\n");
            false
        } else {
            let fd34 = sys_open_trunc(PATH34.as_ptr() as u64, PATH34.len() as u64);
            if fd34 == SYSCALL_FAIL {
                w(b"  case34 sys_open failed\n");
                false
            } else {
                let written34 = sys_fdwrite(fd34, elf34_ptr, elf34_len);
                sys_close(fd34);
                if written34 != elf34_len {
                    w(b"  case34 sys_fdwrite wrote fewer bytes than the real ELF image's own length -- real, disclosed short-write\n");
                    false
                } else {
                    let fork34_result = sys_fork();
                    if fork34_result == 0 {
                        let child34_path: &[u8] = PATH34;
                        sys_exec(child34_path.as_ptr() as u64, child34_path.len() as u64);
                        w(b"  case34 child sys_exec() FAILED, returned instead of replacing this process\n");
                        sys_exit(250);
                    }
                    if fork34_result == SYSCALL_FAIL {
                        w(b"  case34 sys_fork failed\n");
                        false
                    } else {
                        let wait34_raw = sys_wait(fork34_result);
                        if wait34_raw == SYSCALL_FAIL {
                            w(b"  case34 sys_wait failed\n");
                            false
                        } else {
                            let exited34 = (wait34_raw >> 16) & 1 == 1;
                            let signaled34 = (wait34_raw >> 17) & 1 == 1;
                            w(b"  real on-disk ELF written, real fork()+exec()+wait() -- unconditional unbounded recursion -- child exited=");
                            w(if exited34 { b"true" } else { b"false" });
                            w(b" signaled=");
                            w(if signaled34 { b"true" } else { b"false" });
                            w(b" (expected exited=false signaled=true -- caught by this kernel's real stack-overflow guard, not a silent exit)\n");
                            signaled34 && !exited34
                        }
                    }
                }
            }
        };
        write_check(b"case34_unbounded_recursion_stack_overflow_signals_not_exits=", case34_ok);

        w(b"\n");

        let overall_m75 = case34_ok;
        w(b"OVERALL_M75=");
        w(if overall_m75 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // ===============================================================
        // MILESTONE 76: real grammar growth -- unary minus and the two
        // real short-circuit logical operators `&&`/`||`. See this file's
        // own top Milestone 76 doc comment for the full design reasoning
        // (why these two were picked over MAX_FUNCS/MAX_PARAMS/arrays-
        // pointers/more C types, and why `&&`/`||` genuinely short-circuit
        // rather than being faked with an unconditional-evaluate bitwise
        // AND/OR).
        // ===============================================================
        w(b"milestone 76: cc unary-minus and short-circuit &&/|| verification starting\n");

        // -------------------------------------------------------------
        // CASE 35: the ordinary in-process Callable path -- unary minus
        // combined with `&&` and an existing comparison operator, real
        // double negation (`-a`, where `a` itself already holds a
        // negative value) exercised as a RETURN expression operand, not
        // just an isolated assignment.
        //   int main() {
        //       int a; a = -3;
        //       int r;
        //       if (a < 0 && a > -10) { r = 1; } else { r = 0; }
        //       return r + -a;
        //   }
        // Hand-computed: a = -3. `a < 0` is true (-3 < 0). `a > -10` is
        // true (-3 > -10, i.e. -3 is the real, deliberate unary-minus
        // application to INTLIT 10 -- this lexer has no negative-literal
        // token at all, `-10` is always UNARY('-', INTLIT(10)), checked
        // directly against lex()'s own unchanged digit-scanning loop
        // above before relying on it here). Both true -> `&&` short-
        // circuits to evaluating BOTH sides (a genuinely true left
        // operand does not skip the right, only a false one does -- see
        // gen_expr()'s own OP_LOGAND codegen comment) and yields 1, so
        // r = 1. `-a` = -(-3) = 3 (unary minus applied to an IDENT,
        // proving it isn't only wired up for INTLIT operands). Final:
        // r + -a = 1 + 3 = 4.
        // -------------------------------------------------------------
        const SRC35: &[u8] = b"int main() { int a; a = -3; int r; if (a < 0 && a > -10) { r = 1; } else { r = 0; } return r + -a; }";
        let case35_result = compile_and_run_program_callable(SRC35.as_ptr() as u64, SRC35.len() as u64);
        w(b"  case35 (unary minus + && + comparison, in-process) returned=");
        if let Some(r) = case35_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 4)\n");
        let case35_ok = case35_result == Some(4);
        write_check(b"case35_unary_minus_and_logical_and_returns_4=", case35_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 36: the real on-disk-ELF + kernel exec()+wait() path --
        // this milestone's own strongest verification tier, same
        // precedent as every milestone since 69 -- `||`'s own real
        // short-circuit behavior, unary minus used as a CALL ARGUMENT
        // (`classify(-5)`, proving unary minus composes with the
        // existing Milestone 72 call-argument grammar, not just with
        // assign/return), and a function call inside the `||`'s own
        // right operand (proving the right operand genuinely is ordinary
        // executable code, not a restricted sub-grammar).
        //   int classify(int x) {
        //       if (x < 0 || x > 100) { return 0; } else { return 1; }
        //   }
        //   int main() {
        //       return classify(-5) + classify(50) + classify(200);
        //   }
        // Hand-computed: classify(-5): -5 < 0 is true -> `||` short-
        // circuits (the true LEFT operand of `||` skips the right
        // entirely -- see gen_expr()'s own OP_LOGOR codegen comment) ->
        // result 1 -> returns 0. classify(50): 50 < 0 false, so the right
        // operand (50 > 100) genuinely DOES run -> also false -> `||` is
        // false -> returns 1. classify(200): 200 < 0 false, right operand
        // runs, 200 > 100 true -> `||` true -> returns 0. Sum:
        // 0 + 1 + 0 = 1, the real kernel wait() exit code this case
        // checks for.
        // -------------------------------------------------------------
        const SRC36: &[u8] =
            b"int classify(int x) { if (x < 0 || x > 100) { return 0; } else { return 1; } } int main() { return classify(-5) + classify(50) + classify(200); }";
        const PATH36: &[u8] = PATH8;
        let (elf36_ptr, elf36_len) = compile_program_standalone_elf(SRC36.as_ptr() as u64, SRC36.len() as u64);
        let case36_ok = if elf36_ptr == 0 {
            w(b"  case36 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH36, elf36_ptr, elf36_len, 1)
        };
        write_check(b"case36_real_elf_exec_logical_or_shortcircuit_returns_1=", case36_ok);

        w(b"\n");

        let overall_m76 = case35_ok && case36_ok;
        w(b"OVERALL_M76=");
        w(if overall_m76 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 37: the ordinary in-process Callable path -- all five new
        // binary bitwise operators (&, |, ^, <<, >>) plus the new unary
        // `~`, combined in one real expression over real variables (not
        // just literals -- IDENT operands exercise find_var() through
        // this milestone's own new codegen paths, the same "not only
        // INTLIT" discipline Milestone 76's own CASE 35 established for
        // unary minus).
        //   int main() {
        //       int a; a = 12;
        //       int b; b = 10;
        //       int r;
        //       r = (a & b) + (a | b) + (a ^ b) + (~a) + (a << 2) + (b >> 1);
        //       return r;
        //   }
        // Hand-computed: a=12 (0b1100), b=10 (0b1010). a&b = 0b1000 = 8.
        // a|b = 0b1110 = 14. a^b = 0b0110 = 6. ~a = -(12+1) = -13 (two's
        // complement bitwise NOT of a positive int is always -(a+1) --
        // checked directly against that identity before using it here,
        // not assumed). a<<2 = 48. b>>1 = 5 (arithmetic shift of a
        // positive value is identical to logical shift here, so this
        // case alone does not distinguish SAR from SHR -- CASE 38 below
        // is deliberately unaffected by that same gap since its own
        // shift operand is also non-negative; a real SAR-vs-SHR-on-a-
        // negative-value distinguishing case is genuinely NOT covered by
        // either case here, a real, disclosed scope gap, not hidden).
        // Sum: 8 + 14 + 6 + (-13) + 48 + 5 = 68.
        // -------------------------------------------------------------
        const SRC37: &[u8] = b"int main() { int a; a = 12; int b; b = 10; int r; r = (a & b) + (a | b) + (a ^ b) + (~a) + (a << 2) + (b >> 1); return r; }";
        let case37_result = compile_and_run_program_callable(SRC37.as_ptr() as u64, SRC37.len() as u64);
        w(b"  case37 (bitwise &,|,^,~,<<,>> combined, in-process) returned=");
        if let Some(r) = case37_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 68)\n");
        let case37_ok = case37_result == Some(68);
        write_check(b"case37_bitwise_operators_combined_returns_68=", case37_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 38: the real on-disk-ELF + kernel exec()+wait() path --
        // this milestone's own strongest verification tier, same
        // precedent as every milestone since 69 -- and a REAL operator-
        // PRECEDENCE regression test, not just "does it compute at
        // all": `&` binding looser than `==`, and `<<` binding looser
        // than `+`, are both real, classic C precedence traps (a
        // language design choice widely considered a historical
        // mistake, but this subset's job is to match real C, not to
        // relitigate it) -- checked directly against the C standard's
        // own precedence table before writing this case, and this case
        // is deliberately built so a WRONG precedence gives a
        // DIFFERENT, distinguishable numeric result, not a
        // coincidentally-identical one.
        //   int combine(int x, int y) { return x + y; }
        //   int main() {
        //       int a; a = 6;
        //       int b; b = 2;
        //       int c; c = 2;
        //       int r;
        //       r = a & b == c;
        //       return combine(r, 1 << 2 + 1);
        //   }
        // Hand-computed (CORRECT real-C precedence): `a & b == c` parses
        // as `a & (b == c)` (== binds tighter than &) = 6 & (2==2) =
        // 6 & 1 = 0b110 & 0b001 = 0, so r = 0. `1 << 2 + 1` parses as
        // `1 << (2 + 1)` (+ binds tighter than <<) = 1 << 3 = 8.
        // combine(0, 8) = 8. If precedence were WRONG (`&` binding
        // tighter than `==`, or `<<` binding tighter than `+`), this
        // would instead compute (6&2)==2 -> 1, and (1<<2)+1 -> 5,
        // giving combine(1,5)=6 -- a genuinely different, real
        // discriminating result, not a case that happens to pass either
        // way.
        // -------------------------------------------------------------
        const SRC38: &[u8] =
            b"int combine(int x, int y) { return x + y; } int main() { int a; a = 6; int b; b = 2; int c; c = 2; int r; r = a & b == c; return combine(r, 1 << 2 + 1); }";
        const PATH38: &[u8] = PATH8;
        let (elf38_ptr, elf38_len) = compile_program_standalone_elf(SRC38.as_ptr() as u64, SRC38.len() as u64);
        let case38_ok = if elf38_ptr == 0 {
            w(b"  case38 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH38, elf38_ptr, elf38_len, 8)
        };
        write_check(b"case38_real_elf_exec_bitwise_precedence_returns_8=", case38_ok);

        w(b"\n");

        let overall_m79 = case37_ok && case38_ok;
        w(b"OVERALL_M79=");
        w(if overall_m79 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // CASE 39: the ordinary in-process Callable path -- real logical
        // NOT (`!`) in every shape that matters: applied to a zero and a
        // nonzero operand (result must be EXACTLY 1 / EXACTLY 0, not
        // merely "nonzero"), doubled (`!!b` normalizes any nonzero to a
        // clean 1), applied to a comparison's own 0/1 result, and as the
        // left operand of `&&`. IDENT operands (not just literals)
        // exercise find_var() through the new codegen path, same "not
        // only INTLIT" discipline CASE 35/37 established.
        //   int main() {
        //       int a; a = 0;
        //       int b; b = 7;
        //       int r;
        //       r = (!a) + (!b) + (!!b) + (!(a < b)) + (!a && b);
        //       return r;
        //   }
        // Hand-computed: !a = !0 = 1. !b = !7 = 0. !!b = !(!7) = !0 = 1.
        // !(a < b) = !(0 < 7) = !1 = 0. (!a && b) = (1 && 7) = 1 (`&&`
        // normalizes to a clean 1). Sum: 1 + 0 + 1 + 0 + 1 = 3. Any bug
        // that leaves `!` producing a raw non-0/1 value (e.g. a bitwise
        // ~ by mistake, -1 for a true operand) would blow this exact sum.
        // -------------------------------------------------------------
        const SRC39: &[u8] = b"int main() { int a; a = 0; int b; b = 7; int r; r = (!a) + (!b) + (!!b) + (!(a < b)) + (!a && b); return r; }";
        let case39_result = compile_and_run_program_callable(SRC39.as_ptr() as u64, SRC39.len() as u64);
        w(b"  case39 (logical ! in five shapes, in-process) returned=");
        if let Some(r) = case39_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 3)\n");
        let case39_ok = case39_result == Some(3);
        write_check(b"case39_logical_not_five_shapes_returns_3=", case39_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 40: the real on-disk-ELF + kernel exec()+wait() path --
        // this milestone's own strongest verification tier, same
        // precedent as every milestone since 69 -- and a REAL operator-
        // PRECEDENCE regression test: unary `!` binds TIGHTER than `*`
        // (and transitively than `+`), real C's own ordering, checked
        // against the C precedence table before writing this, and this
        // case is built so a WRONG precedence gives a DIFFERENT,
        // distinguishable result.
        //   int main() {
        //       int a; a = 0;
        //       int b; b = 3;
        //       int r;
        //       r = !a * b;
        //       return r + !(b - 3);
        //   }
        // Hand-computed (CORRECT precedence, `!` tighter than `*`):
        // `!a * b` = (!0) * 3 = 1 * 3 = 3, so r = 3. `b - 3` = 0,
        // `!(b - 3)` = !0 = 1. return 3 + 1 = 4. If `!` bound LOOSER
        // than `*` (parsing `!a * b` as `!(a * b)`), this would compute
        // `!(0 * 3)` = !0 = 1, so r = 1, and `1 + !(3 - 3)` = 1 + 1 = 2
        // -- a genuinely different, real discriminating result, not one
        // that passes either way.
        // -------------------------------------------------------------
        const SRC40: &[u8] =
            b"int main() { int a; a = 0; int b; b = 3; int r; r = !a * b; return r + !(b - 3); }";
        const PATH40: &[u8] = PATH8;
        let (elf40_ptr, elf40_len) = compile_program_standalone_elf(SRC40.as_ptr() as u64, SRC40.len() as u64);
        let case40_ok = if elf40_ptr == 0 {
            w(b"  case40 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH40, elf40_ptr, elf40_len, 4)
        };
        write_check(b"case40_real_elf_exec_logical_not_precedence_returns_4=", case40_ok);

        w(b"\n");

        let overall_m83 = case39_ok && case40_ok;
        w(b"OVERALL_M83=");
        w(if overall_m83 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // CASE 41: the in-process Callable path -- every one of the nine
        // compound-assignment operators applied in sequence to a single
        // variable, each intermediate result hand-computed so a bug in
        // any one operator's desugaring blows the final number.
        //   int main() {
        //       int x; x = 100;
        //       x += 5;    x -= 20;   x *= 3;    x /= 2;   x &= 60;
        //       x |= 3;    x ^= 1;    x <<= 2;   x >>= 1;
        //       return x;
        //   }
        // Hand-computed: 100 -> +5=105 -> -20=85 -> *3=255 -> /2=127
        // (integer) -> &60: 127=0b1111111 & 60=0b0111100 = 0b0111100 =
        // 60 -> |3: 0b111100 | 0b000011 = 0b111111 = 63 -> ^1: 63^1 =
        // 62 -> <<2: 62<<2 = 248 -> >>1: 248>>1 = 124. Return 124.
        // -------------------------------------------------------------
        const SRC41: &[u8] = b"int main() { int x; x = 100; x += 5; x -= 20; x *= 3; x /= 2; x &= 60; x |= 3; x ^= 1; x <<= 2; x >>= 1; return x; }";
        let case41_result = compile_and_run_program_callable(SRC41.as_ptr() as u64, SRC41.len() as u64);
        w(b"  case41 (all nine compound-assign ops chained, in-process) returned=");
        if let Some(r) = case41_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 124)\n");
        let case41_ok = case41_result == Some(124);
        write_check(b"case41_compound_assign_all_nine_ops_returns_124=", case41_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 42: the real on-disk-ELF + kernel exec()+wait() path --
        // this milestone's strongest tier, same precedent as every
        // milestone since 69 -- and a REAL precedence-regression test:
        // the RHS of a compound assignment is a full `logic_or`, this
        // subset's lowest-precedence production, so `x += 3 * 4` MUST
        // parse as `x = x + (3*4)` and `x <<= 1 + 1` as `x = x <<
        // (1+1)`. Built so a wrong binding gives a different number.
        //   int main() {
        //       int x; x = 2;
        //       x += 3 * 4;   // x = x + (3*4) = 2 + 12 = 14
        //       x <<= 1 + 1;  // x = x << (1+1) = 14 << 2 = 56
        //       return x;
        //   }
        // Hand-computed (CORRECT precedence): 2 + 12 = 14, then 14 << 2
        // = 56. If `+=`'s RHS bound only a `factor` (or tighter than
        // `*`), `x += 3` then `* 4` is a dangling `* 4` -> parse error
        // or `(x+3)*4 = 20`; if `<<=`'s RHS bound tighter than `+`,
        // `(x<<1)+1 = 29`. 56 is reachable only with real C precedence.
        // -------------------------------------------------------------
        const SRC42: &[u8] =
            b"int main() { int x; x = 2; x += 3 * 4; x <<= 1 + 1; return x; }";
        const PATH42: &[u8] = PATH8;
        let (elf42_ptr, elf42_len) = compile_program_standalone_elf(SRC42.as_ptr() as u64, SRC42.len() as u64);
        let case42_ok = if elf42_ptr == 0 {
            w(b"  case42 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH42, elf42_ptr, elf42_len, 56)
        };
        write_check(b"case42_real_elf_exec_compound_assign_precedence_returns_56=", case42_ok);

        w(b"\n");

        let overall_m84 = case41_ok && case42_ok;
        w(b"OVERALL_M84=");
        w(if overall_m84 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 85: no new grammar and no new codegen -- it closes
        // the one specifically-disclosed verification hole Milestone 79's
        // own closing note left open and Milestone 84's re-confirmed:
        // "SAR vs. SHR is genuinely UNDISTINGUISHED by any test case
        // (both only ever shift a non-negative value, where the two
        // encodings produce identical results)". `gen_expr()`'s OP_SHR
        // arm emits `emit_sar_rax_cl()` (SAR, 0x48 0xD3 0xF8 -- the
        // arithmetic, sign-preserving shift), NOT SHR (logical). Every
        // prior test shifts a non-negative value, where SAR and SHR give
        // the identical bit pattern, so a latent bug that emitted SHR
        // instead would have passed all of them. These two cases shift a
        // genuinely NEGATIVE value and then read its sign, so SAR
        // (result stays negative) and SHR (result becomes a large
        // positive) produce different, distinguishable answers. `<<`
        // needs no arithmetic variant -- SHL is bit-identical for signed
        // and unsigned operands -- so it is not separately re-verified.
        //
        // CASE 43 (in-process Callable path): -8 >> 1. SAR = -4 (bit 63
        // set), so `a < 0` is true and the program returns `0 - a` = 4.
        // If OP_SHR emitted SHR, `-8` (0xFFFFFFFFFFFFFFF8) >> 1 =
        // 0x7FFFFFFFFFFFFFFC, a large positive, `a < 0` false, return
        // 111 -- a completely different result.
        // -------------------------------------------------------------
        const SRC43: &[u8] = b"int main() { int a; a = -8; a = a >> 1; if (a < 0) { return 0 - a; } return 111; }";
        let case43_result = compile_and_run_program_callable(SRC43.as_ptr() as u64, SRC43.len() as u64);
        w(b"  case43 (>> on a negative value is arithmetic (SAR), in-process) returned=");
        if let Some(r) = case43_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 4)\n");
        let case43_ok = case43_result == Some(4);
        write_check(b"case43_shift_right_negative_is_arithmetic_returns_4=", case43_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 44 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier, same precedent as every milestone
        // since 69): -16 >> 2. SAR = -4, so `x < 0` is true and the
        // program exits 7. With SHR it would be a large positive, `x <
        // 0` false, exit 9. The `< 0` comparison is what amplifies the
        // sign-bit difference into an observable exit code (a bare `>>`
        // result's LOW 8 bits are identical under SAR and SHR -- only
        // bit 63 differs -- so the exit code must be routed through a
        // sign test, not returned directly).
        // -------------------------------------------------------------
        const SRC44: &[u8] =
            b"int main() { int x; x = -16; x = x >> 2; if (x < 0) { return 7; } return 9; }";
        const PATH44: &[u8] = PATH8;
        let (elf44_ptr, elf44_len) = compile_program_standalone_elf(SRC44.as_ptr() as u64, SRC44.len() as u64);
        let case44_ok = if elf44_ptr == 0 {
            w(b"  case44 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH44, elf44_ptr, elf44_len, 7)
        };
        write_check(b"case44_real_elf_exec_shift_right_negative_is_arithmetic_returns_7=", case44_ok);

        w(b"\n");

        let overall_m85 = case43_ok && case44_ok;
        w(b"OVERALL_M85=");
        w(if overall_m85 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 86: `for` loops, desugared to `init; while (cond) {
        // body step; }`. No new codegen -- Milestone 71's STMT_WHILE is
        // reused verbatim.
        //
        // CASE 45 (in-process Callable path): a real counting loop whose
        // init, condition, step, and body all matter.
        //   int main() {
        //       int s; s = 0;
        //       int i;
        //       for (i = 1; i < 5; i += 1) { s += i; }
        //       return s;
        //   }
        // Hand-computed: i = 1,2,3,4 (stops at 5), s = 1+2+3+4 = 10. An
        // off-by-one in the desugared condition or a dropped step gives a
        // different sum.
        // -------------------------------------------------------------
        const SRC45: &[u8] = b"int main() { int s; s = 0; int i; for (i = 1; i < 5; i += 1) { s += i; } return s; }";
        let case45_result = compile_and_run_program_callable(SRC45.as_ptr() as u64, SRC45.len() as u64);
        w(b"  case45 (for loop sum 1..4, in-process) returned=");
        if let Some(r) = case45_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 10)\n");
        let case45_ok = case45_result == Some(10);
        write_check(b"case45_for_loop_sum_returns_10=", case45_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 46 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier): factorial via a `for` loop whose
        // step is a Milestone-84 compound assignment and whose body
        // mutates a second variable, so the desugar order (body BEFORE
        // step, step INSIDE the while body not after it) is what makes
        // the number come out right.
        //   int main() {
        //       int p; p = 1;
        //       int i;
        //       for (i = 1; i < 6; i += 1) { p *= i; }
        //       return p;
        //   }
        // Hand-computed: p = 1*1*2*3*4*5 = 120. If `step` ran before the
        // body, p would be 2*3*4*5*6 = 720 (low 8 bits 0xD0 = 208, not
        // 120); a dropped final iteration gives 24. 120 is reachable only
        // with the correct `body then step` desugar.
        // -------------------------------------------------------------
        const SRC46: &[u8] =
            b"int main() { int p; p = 1; int i; for (i = 1; i < 6; i += 1) { p *= i; } return p; }";
        const PATH46: &[u8] = PATH8;
        let (elf46_ptr, elf46_len) = compile_program_standalone_elf(SRC46.as_ptr() as u64, SRC46.len() as u64);
        let case46_ok = if elf46_ptr == 0 {
            w(b"  case46 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH46, elf46_ptr, elf46_len, 120)
        };
        write_check(b"case46_real_elf_exec_for_loop_factorial_returns_120=", case46_ok);

        w(b"\n");

        let overall_m86 = case45_ok && case46_ok;
        w(b"OVERALL_M86=");
        w(if overall_m86 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 87: `break` and `continue`.
        //
        // CASE 47 (in-process Callable path): both, inside a `while`.
        //   int main() {
        //       int s; s = 0;
        //       int i; i = 0;
        //       while (i < 100) {
        //           i += 1;
        //           if (i > 10) { break; }
        //           if (i == 3) { continue; }
        //           s += i;
        //       }
        //       return s;
        //   }
        // Trace: i runs 1..10 (break at i == 11); i == 3 skips its own
        // `s += i`. s = (1+2+..+10) - 3 = 55 - 3 = 52. `break` from
        // inside a nested `if`, and `continue` re-checking the `while`
        // condition, are both exercised.
        // -------------------------------------------------------------
        const SRC47: &[u8] = b"int main() { int s; s = 0; int i; i = 0; while (i < 100) { i += 1; if (i > 10) { break; } if (i == 3) { continue; } s += i; } return s; }";
        let case47_result = compile_and_run_program_callable(SRC47.as_ptr() as u64, SRC47.len() as u64);
        w(b"  case47 (break + continue in a while, in-process) returned=");
        if let Some(r) = case47_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 52)\n");
        let case47_ok = case47_result == Some(52);
        write_check(b"case47_break_and_continue_in_while_returns_52=", case47_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 48 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier): `break` and `continue` inside a
        // `for` loop. This is the case that proves `continue` in a
        // `for` RUNS THE STEP CLAUSE (real C semantics) rather than
        // jumping straight to the condition:
        //   int main() {
        //       int s; s = 0;
        //       int i;
        //       for (i = 0; i < 20; i += 1) {
        //           if (i == 5) { break; }
        //           if (i == 2) { continue; }
        //           s += i;
        //       }
        //       return s;
        //   }
        // Trace: i runs 0..4 (break at i == 5); i == 2 skips its own
        // `s += i` but STILL runs `i += 1`. s = 0+1+3+4 = 8.
        // If `continue` skipped the step, i would stay 2 forever -- the
        // program would hang and `wait()` would never return an exit
        // code at all, let alone 8. If `break` were a no-op, the loop
        // would run to i == 19 and s would be 190 - 2 = 188. Exit code
        // 8 is reachable only with both working correctly.
        // -------------------------------------------------------------
        const SRC48: &[u8] =
            b"int main() { int s; s = 0; int i; for (i = 0; i < 20; i += 1) { if (i == 5) { break; } if (i == 2) { continue; } s += i; } return s; }";
        const PATH48: &[u8] = PATH8;
        let (elf48_ptr, elf48_len) = compile_program_standalone_elf(SRC48.as_ptr() as u64, SRC48.len() as u64);
        let case48_ok = if elf48_ptr == 0 {
            w(b"  case48 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH48, elf48_ptr, elf48_len, 8)
        };
        write_check(b"case48_real_elf_exec_break_continue_in_for_runs_step_returns_8=", case48_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 49: a deliberate SEMANTIC error -- `break` with no
        // enclosing loop must be Err(CodeGenError::BreakOutsideLoop),
        // the same "prove the new Err path is real, not an unexercised
        // variant" discipline CASE 7 (UndeclaredVariable) and CASE 29
        // (UndeclaredFunction) already established.
        //   int main() { break; return 1; }
        // -------------------------------------------------------------
        const SRC49: &[u8] = b"int main() { break; return 1; }";
        let src49_ptr = SRC49.as_ptr() as u64;
        let src49_len = SRC49.len() as u64;
        let case49_ok = match lex_and_parse(src49_ptr, src49_len) {
            Ok(func49) => match gen_function(src49_ptr, func49, CodegenMode::Callable) {
                Err(CodeGenError::BreakOutsideLoop) => {
                    w(b"  case49 real codegen error -- break outside any loop\n");
                    true
                }
                _ => {
                    w(b"  case49 expected Err(BreakOutsideLoop), got something else\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case49 lex_and_parse() returned Err unexpectedly (source should lex+parse fine)\n");
                false
            }
        };
        write_check(b"case49_break_outside_loop_is_BreakOutsideLoop=", case49_ok);

        w(b"\n");

        let overall_m87 = case47_ok && case48_ok && case49_ok;
        w(b"OVERALL_M87=");
        w(if overall_m87 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        sys_exit(if overall && overall_m68 && overall_m69 && overall_m70 && overall_m71 && overall_m72 && overall_m73 && overall_m74 && overall_m75 && overall_m76 && overall_m79 && overall_m83 && overall_m84 && overall_m85 && overall_m86 && overall_m87 { 0 } else { 1 });
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
