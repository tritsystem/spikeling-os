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
// MILESTONE 88 adds the combined declaration-with-initializer,
// `int IDENT ("=" logic_or)? ";"`, and the same in a `for` init clause
// (`for (int i = 0; ...)`, the one shape Milestone 86's `for` could not
// express). `int i = e;` is DESUGARED at parse time to `int i;`
// immediately followed by `i = e;` -- a two-node chain (STMT_DECL ->
// STMT_ASSIGN), which `parse_stmt_list_until_rbrace()` already handles
// since Milestone 86 walks to a chain's real tail. ZERO new codegen,
// zero new AST kinds: STMT_DECL is unchanged (still what `collect_vars`
// counts) and the synthesized STMT_ASSIGN takes the exact
// `finish_ident_assign()` path a plain `i = e;` already takes. Only a
// plain `=` starts an initializer -- a compound op after `int i` reads
// an uninitialized variable and is left to fall through as a parse
// error. Two new self-test cases: CASE 50 (in-process -- two
// initializers, the second referencing the first -> 80) and CASE 51
// (real on-disk-ELF -- `for (int i = 1; i <= 5; i += 1) s += i` -> 15).
//
// Still genuinely open after Milestone 88: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); no arrays/pointers, no additional C types; no
// `switch`/`goto`; the compiler self-test still leaks per-compile
// (bounded by `heap_reset()` since Milestone 87, not fixed); one 4 KiB
// stack page per process.
//
// MILESTONE 89 adds the `%` (modulo) operator and its `%=` compound
// form -- the last missing arithmetic operator in real C's `* / %`
// group. `%` binds at the exact same precedence level as `*` and `/`,
// left-associative (`parse_term` just gains it as a third alternative).
// Codegen reuses `/`'s own `cqo; idiv rcx` -- `idiv` already produces
// the remainder in RDX alongside the quotient in RAX -- and adds ONE
// new encoding, `emit_mov_rax_rdx()`, to land the remainder in RAX per
// gen_expr()'s "value in RAX" postcondition. `%=` drops into Milestone
// 84's compound-assignment machinery with binop `b'%'`; its lexing is
// the same "2-char before 1-char" order every other compound already
// uses. Two new self-test cases: CASE 52 (in-process -- `%`, `/` and
// `%=` combined -> 5) and CASE 53 (real on-disk-ELF -- a
// `%`-vs-`*` precedence-regression test, `20 % 7 * 2` -> 12 not 6).
//
// Still genuinely open after Milestone 89: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); no arrays/pointers, no additional C types; no
// `switch`/`goto`; the compiler self-test still leaks per-compile
// (bounded, not fixed); one 4 KiB stack page per process.
//
// MILESTONE 90 adds the conditional (ternary) operator, `cond ? a : b`.
// One new parser layer, `parse_ternary`, sits directly above
// `parse_logic_or` (looser than `||`, looser than everything except
// assignment -- real C ordering) and is now what every "parse a whole
// expression" position calls. Right-associative (`a?b:c?d:e` is
// `a?b:(c?d:e)`), the middle operand a full expression. One new EXPR
// kind whose codegen IS gen_stmt_list::STMT_IF's exact jz/jmp/patch
// shape, but as an expression leaving its value in RAX: exactly one of
// the two branches runs. ZERO new CodeBuf encodings. Field reuse --
// EXPR_TERNARY's `left`/`right`/`call_args_ptr` = cond/then/else. Two
// new self-test cases: CASE 54 (both ways, in-process -> 73) and CASE
// 55 (ternary as a sub-expression in real arithmetic, on-disk ELF
// -> 12).
//
// Still genuinely open after Milestone 90: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); no arrays/pointers, no additional C types; no
// `switch`/`goto`; the compiler self-test still leaks per-compile
// (bounded, not fixed); one 4 KiB stack page per process.
//
// MILESTONE 91 adds `switch` / `case` / `default` --
//   switch_stmt := "switch" "(" ternary ")" "{"
//                    ("case" INTLIT ":" stmt*)* ("default" ":" stmt*)? "}"
// -- with real C fall-through, `break` (jumps to the switch end, not
// out of an enclosing loop), and an optional single `default`. The
// discriminant is evaluated ONCE and spilled to a reserved
// switch-scratch frame slot (`rbp - (nvars+1)*8`; every function's
// frame now carries one extra slot -- 8 wasted bytes for a switch-free
// function, disclosed). Dispatch is a linear `cmp rax, imm32; jz body`
// per case (one new CodeBuf encoding, `emit_cmp_rax_imm32`); no match
// jumps to `default` (or the end). Case bodies are emitted in source
// order and fall through into each other and into `default`.
// `gen_stmt_list` gains a `break_ok` value parameter (true inside a
// loop body OR a switch case body); `break` now checks it instead of
// `loop_top`. `break`'s placeholders reuse Milestone 87's `LOOP_BRK`
// arena with the same per-scope save/restore, so a switch nested in a
// loop (or vice versa) resolves each `break`/`continue` to the right
// target. `collect_vars` learns to walk STMT_SWITCH/STMT_CASE bodies.
//
// Real, disclosed scope cuts: case labels are bare INTLITs (not
// constant expressions), fitting in 32 bits (sign-extended); `default`
// is always lowered as the LAST label (its body emitted after every
// `case` body, and the no-match jump targets it), so a `default`
// written textually in the MIDDLE of a switch does not fall through
// from the preceding case the way a C compiler would -- put `default`
// last, or give it a `break`. No duplicate-label check. No `switch` on
// a non-`int` (this subset has only `int` anyway).
//
// Two new self-test cases: CASE 56 (in-process -- a match that falls
// through into the next case, a `break`, a `default` not taken -> 25)
// and CASE 57 (real on-disk-ELF -- a `switch` inside a `for`, proving
// the switch `break` does not escape the loop and `default` runs on a
// no-match -> 131).
//
// Still genuinely open after Milestone 91: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); no arrays/pointers, no additional C types; no `goto`; the
// compiler self-test still leaks per-compile (bounded, not fixed); one
// 4 KiB stack page per process.
//
// MILESTONE 92 adds `goto LABEL ;` and `LABEL :` -- completing this
// subset's C control flow. A label is an ordinary IDENT followed by
// `:` in statement position (one token of lookahead disambiguates it
// from an assignment; `L:` is a standalone STMT_LABEL, whatever follows
// is the next list node). Codegen keeps a per-function label table
// (name -> buf offset) and a pending-goto list (name -> jmp
// placeholder), both in module `static mut` arenas reset per function
// (`goto_reset()`, same pattern as Milestone 87). A `goto` to an
// already-seen label is a real backward `jmp`; to a not-yet-seen label
// it is a placeholder resolved either when that label is emitted
// (forward-patched to land exactly there) or, if the label is never
// defined anywhere in the function, reported as
// `CodeGenError::UndefinedLabel`. ZERO new CodeBuf encodings -- reuses
// `emit_jmp_back`/`emit_jmp_placeholder`/`patch_rel32`. Disclosed:
// labels share the ordinary identifier namespace (a variable and a
// label with the same name is not diagnosed, though it is harmless --
// they live in different tables); a bare `L:` immediately before `}` is
// accepted (C requires a following statement pre-C23). Three new
// self-test cases: CASE 58 (in-process -- a backward goto forming a
// loop and a forward goto skipping a statement -> 10), CASE 59 (the
// UndefinedLabel error path), CASE 60 (real on-disk-ELF -- goto as an
// early-exit clear out of a `for` loop, skipping the post-loop
// statement -> 7).
//
// Still genuinely open after Milestone 92: `MAX_FUNCS`/`MAX_PARAMS`
// (4 each); no arrays/pointers, no additional C types (only `int`);
// the compiler self-test still leaks per-compile (bounded, not fixed);
// one 4 KiB stack page per process. This subset's C control flow is
// now COMPLETE (if/else, while, for, break/continue, switch,
// goto/labels, function calls, recursion).
//
// MILESTONE 93 raises `MAX_FUNCS` 4 -> 8 and `MAX_TOKENS` 128 -> 256 --
// the bump Milestones 76/79/84/90 each explicitly weighed and DEFERRED
// for lack of "a concrete program driving it". Now that full C control
// flow is in place (Milestone 92), CASE 61 is that program: six
// functions, one forward call, > 128 tokens. Both caps are transient
// `malloc()`ed scratch buffers freed at process exit, so raising them
// has zero effect on cc.elf's own compiled/linked size -- exactly the
// reasoning MAX_TOKENS' own 64 -> 128 bump (Milestone 70) used.
// `MAX_PARAMS` stays 4: only four argument registers, and stack-passed
// arguments are genuine new codegen (a separate milestone). One new
// self-test case, CASE 61 (in-process, six functions + a forward call
// -> 30).
//
// Still genuinely open after Milestone 93: `MAX_PARAMS` (4 -- see
// above); no arrays/pointers; only the `int` type; the compiler
// self-test still leaks per-compile (bounded, not fixed); one 4 KiB
// stack page per process.
//
// MILESTONE 94 adds `//` line and `/* */` block comments -- a
// lexer-only change (the parser and codegen never see a comment).
// `//` runs to the next newline or end of input; `/* ... */` runs to
// the first `*/`, no nesting (real C). An unterminated block comment
// is a real LexError pointing at its opening `/`. The comment check
// sits right after whitespace-skipping and before the `/=` (Milestone
// 89) multi-char check, so `a /= 4` still lexes as an operator. Two
// new self-test cases: CASE 62 (in-process -- a program peppered with
// both comment styles, a block comment between two tokens, a `/=`
// right after a `//` -> 30) and CASE 63 (the unterminated-comment
// LexError path).
//
// Still genuinely open after Milestone 94: `MAX_PARAMS` (4, needs
// stack-passed arguments); no arrays/pointers; only the `int` type;
// the compiler self-test still leaks per-compile (bounded, not fixed);
// one 4 KiB stack page per process.
//
// MILESTONE 95 adds `0x`/`0X` hex and `0b`/`0B` binary integer
// literals -- a lexer-only change. A leading `0` followed by `x`/`X`
// scans hex digits (`0`-`9`, `a`-`f`, `A`-`F`); by `b`/`B` scans
// binary digits (`0`/`1`). Both prefixes are unambiguous against every
// pre-existing decimal literal, so the decimal scan is byte-for-byte
// unchanged. A prefix with no following digit (`0x`, `0b`) is a real
// LexError pointing where a digit was due. Deliberate cuts: NO C octal
// (a leading `0` with more digits stays decimal -- `010` is ten), no
// digit separators, no `U`/`L` suffixes. CASE 64 (in-process -- all
// three bases in one expression: `0xFF + 0b1010 + 0x10` -> 281) and
// CASE 65 (the empty-`0x` LexError path).
//
// MILESTONE 96 is a regression fix, not a feature. `case8` (the one
// ELF-exec self-test that reused CASE 1's already-parsed `func1` AST
// instead of re-parsing) had been exiting 0 instead of 42 -- and so
// `OVERALL_M69=FAIL` -- on every boot since Milestone 87. Cause:
// Milestone 87 added `heap_reset()` on entry of the compile helpers to
// bound the self-test's per-compile leak; that rewinds the bump
// allocator between CASE 1 and CASE 8, so CASE 4/5/6/7's intervening
// compiles overwrite `func1`'s nodes and `gen_function` codegens a
// stale function. Fix: CASE 8 now `heap_reset()`s and re-parses CASE
// 1's exact source in place (same intent -- CASE 1's function through
// the Standalone/ELF64 backend -- with no dependency on cross-compile
// heap survival). New check `OVERALL_M96 = case8_ok && overall_m69`.
//
// MILESTONE 97 adds character literals -- a lexer-only change. `'c'`
// lexes to an ordinary TOK_INTLIT whose value is the character's byte;
// the parser and codegen are untouched (a char literal IS an integer
// literal to everything downstream). Common single-character escapes
// are recognized: `\n \t \r \0 \\ \' \"`. Deliberate cuts: no
// multi-character constants, no `\xNN` / octal `\NNN` numeric escapes,
// no wide/unicode literals. An empty `''`, an unterminated `'`, an
// unknown `\escape`, or a literal not closed exactly one character
// later is a real LexError at the opening quote. CASE 66 (in-process --
// `'A' + ('a'-'A') + '\n' + '\0'` -> 107) and CASE 67 (the
// multi-char-constant LexError path).
//
// MILESTONE 98 adds unary `+` -- a parse-time no-op. `parse_unary()`
// consumes a leading `TOK_PLUS` and returns the operand's own parse
// directly: no EXPR_UNARY node, no codegen, same tightest-precedence
// slot as unary `-`/`~`/`!`, composes with all of them (`-+-x`, `+!x`
// parse). It does not force non-negativity or any conversion (no
// unsigned/narrower types exist here). CASE 68 (`+x + +3 * +2` -> 11)
// and CASE 69 (`-+-x` with x=10 -> 10).
//
// MILESTONE 99 adds `do { body } while (cond);` -- the third and last
// loop form, with real codegen (a distinct STMT_DOWHILE kind, NOT a
// flag on STMT_WHILE). Body-first, condition and its backward jump at
// the bottom, so the body always runs at least once. `break` and
// `continue` bind to it through the exact LOOP_BRK/LOOP_CONT arena
// save/restore STMT_WHILE uses; `continue` targets the bottom
// condition re-check via the same forward-patched-placeholder path a
// `for` loop's `continue` uses to reach its step clause (the body is
// recursed with loop_is_for = 1). Zero new CodeBuf encodings. CASE 70
// (in-process, sum 0..4 -> 10), CASE 71 (in-process, `break` +
// `continue` -> 12), CASE 72 (on-disk ELF + real exec(), factorial
// -> 120). This subset's C control flow is now: if/else, while, for,
// do-while, break/continue, switch/case/default, goto/labels, calls,
// direct + mutual recursion.
//
// MILESTONE 100 (kernel, not this file): the per-process user stack
// grew from one 4 KiB page to eight (32 KiB) -- this compiler's
// recursive-descent parser had run the single page off the bottom.
//
// MILESTONE 101 adds multiple comma-separated declarators after one
// `int` -- parser-only. `int a, b = 2, c;` desugars to the flat chain
// `DECL(a) -> DECL(b) -> ASSIGN(b = 2) -> DECL(c)`; each declarator may
// carry its own `= <ternary>` initializer, and a later one may
// reference an earlier declarator in the same list. Statement level and
// `for` init clause. No new AST kind, no new codegen. A dangling `,` is
// a real ParseError. CASE 73 (mixed init -> 8), CASE 74 (sequential
// dependency -> 11), CASE 75 (dangling-comma error). This is also the
// end-to-end proof of Milestone 100: this exact change overflowed the
// old single stack page and now runs with room to spare.
//
// MILESTONE 102 adds a real second type, `char`, alongside `int` --
// this subset's first genuine width distinction. `char c = e;` parses
// through the exact same make_decl()/multi-declarator machinery
// Milestone 101 just built (statement level and `for`-init, with or
// without an initializer, comma-chained, mixed with `int` declarators
// in the same block), now carrying a real type tag (`is_char`,
// threaded through StmtNode's otherwise-dead-for-DECL `then_body`
// field into a new `VarSlot.is_char` byte). Codegen stays almost
// entirely untouched: every stack slot is still a full 8 bytes
// (collect_vars_rec()'s layout is unchanged) and STMT_ASSIGN's store
// is still the same unconditional full-width `mov [rbp+off], rax`
// (emit_mov_rbp_off_rax(), untouched) -- the ONLY new codegen is
// EXPR_IDENT's read of a `char` slot, which now emits a real `movsx
// rax, byte [rbp+off]` (emit_movsx_rax_rbp_off_byte(), new) instead of
// the full-width load, sign-extending just the slot's low byte. Store-
// full/narrow-on-read this way, for FREE, real 8-bit truncation on
// overflow (`char c = 321;` reads back `65`) AND real signed-char
// wraparound (`char c = 200;` reads back `-56`, matching a real
// x86_64 SysV compiler's default `char` signedness) with no separate
// truncate-on-store codegen at all, because store and (narrowed) load
// touch the identical low byte of the same little-endian slot. CASE 76
// (basic char decl + arithmetic -> 66), CASE 77 (truncation-on-store,
// `char c = 321` -> `65`), CASE 78 (signed sign-extension, `char c =
// 200; c == -56` -> `1`), CASE 79 (char as a `while`-loop counter,
// in-process -> `55`; see CASE 79's OWN doc comment below for why this
// one deliberately stays on the in-process Callable path instead of
// this milestone's usual real on-disk-ELF + exec() tier -- a real,
// disclosed kernel-heap-margin bug this milestone's own development
// found, not an oversight).
//
// MILESTONE 103 closes one of Milestone 102's own disclosed gaps: `char`
// as a function PARAMETER type and a function RETURN type, not just a
// local-variable type. `char` parameters reuse Milestone 102's narrowed-
// read mechanism with ZERO new codegen -- `parse_params()` now records
// each parameter's own `is_char` (a new `ParamInfo.is_char` byte,
// `TOK_INT`/`TOK_CHAR` per parameter, the same choice `make_decl()`
// already offers at declaration sites), and `collect_vars_for_function()`
// copies it straight into that parameter's own `VarSlot` -- the
// prologue's own register-spill (`emit_mov_rbp_off_reg()`) is completely
// untouched, still spilling the full incoming register exactly as
// before; only the parameter's own later narrowed READ (Milestone 102's
// existing `EXPR_IDENT` path) makes it real. A `char` RETURN type is the
// one genuinely NEW piece of codegen: `emit_movsx_rax_al()` (new -- `48
// 0F BE C0`, register-direct `movsx rax, al`) narrows RAX in place right
// before the epilogue, gated by a new `FuncDef.is_char_return` byte
// threaded through `gen_stmt_list()`'s new `is_char_return` parameter
// (a real per-function constant, passed unchanged through every one of
// its recursive calls, exactly like `vars_ptr`/`funcs_ptr`/`mode`
// already are) -- STMT_RETURN is the only arm that reads it. This one is
// genuinely new because a return value lives in a register with no
// stack slot to narrow through, unlike every other `char` use so far.
// CASE 80 (char parameter, basic -> `66`), CASE 81 (char parameter
// truncates an out-of-range ARGUMENT -> `65`, the calling-convention
// analogue of Milestone 102's CASE 77), CASE 82 (char RETURN type
// truncates -> `44`, the one case that actually exercises
// `emit_movsx_rax_al()` -- without it this would return the untruncated
// `300`), CASE 83 (char parameter and char return CHAINED across two
// real functions -> `43`), CASE 84 (char parameter is signed -> `1`,
// the parameter-side analogue of Milestone 102's CASE 78). All five
// stay on the in-process Callable path -- see Milestone 102's own CASE
// 79 doc comment for the real, disclosed kernel-heap-margin bug that
// makes adding MORE real fork()/exec()/wait() cycles to this suite
// risky until a future milestone actually resolves it.
//
// Still genuinely open after Milestone 103: `MAX_PARAMS` (4, needs
// stack-passed arguments); no arrays/pointers; there is still no
// `sizeof` or any other width-introspection, and `char` remains the
// only non-`int` type (no `short`/`long`/`unsigned`); the compiler
// self-test still leaks per-compile (bounded by `heap_reset()`, not
// eliminated); the real, still-UNRESOLVED kernel-heap-margin bug
// Milestone 102 discovered and disclosed (see its own CASE 79 doc
// comment) -- a genuine, concrete candidate for a near-future milestone,
// now blocking two milestones' worth of "add one more real exec() self-
// test case" in a row, not just one.
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
//
// MILESTONE 105: closes Milestone 102's own disclosed CASE 79 gap now
// that Milestone 104 (see kernel/src/heap_guard.rs and README.md's own
// entry) actually root-caused and fixed the real kernel-heap-margin bug
// that made CASE 79 stay on the in-process Callable path instead of the
// real on-disk-ELF + fork()/exec()/wait() tier every other case since
// Milestone 69 uses. CASE 79 itself is UNCHANGED as a compiler test --
// same source, same hand-computed expected value (55) -- only its own
// verification tier moved, from `compile_and_run_program_callable()` to
// `compile_program_standalone_elf()` + `write_exec_and_check!` (CASE
// 72's own established pattern, reused rather than reinvented). This is
// a real fifth exec()/wait() cycle added to this suite, on top of the
// four Milestone 102 already found sitting right at the OLD 100 KiB
// budget's edge -- verified for real against a live boot below (see
// README.md's own Milestone 105 entry), not merely assumed safe from
// Milestone 104's own arithmetic.
//
// MILESTONE 107 raises MAX_PARAMS 4 -> 8 by adding real stack-passed
// argument codegen -- the exact landmine a prior session's own disclosed
// groundwork identified but deliberately did not implement (see this
// repo's README.md for that prior session's full note): the old call-site
// codegen pushed every argument LEFT-TO-RIGHT then popped them back off in
// REVERSE order into the (at most 4) real argument registers, which only
// worked because every argument was register-destined; with more than 4
// arguments some are stack-destined instead, and the old scheme buries
// them under the register-destined ones in a way that cannot be popped out
// without disturbing the rest. Fixed by reversing the push loop itself
// (push argument index `argc-1` first, ... `0` last) and popping ASCENDING
// (`0..min(argc, MAX_REG_PARAMS)`) instead of descending -- the exact fix
// that prior session's own groundwork proposed, but this milestone
// actually VERIFIED the one open precondition it flagged rather than
// trusting it: whether this subset-C language can express any argument
// sub-expression with an observable side effect, which would make
// evaluation order (now right-to-left instead of left-to-right) a real,
// externally visible behavior change. Checked directly against this
// file's own grammar, not assumed -- this subset-C has (1) no assignment
// EXPRESSION at all, only STMT_ASSIGN, a statement (`x = e;` is not
// itself a value anywhere an argument position could use it -- see
// parse_ternary()'s own production chain, which never bottoms out at
// assignment), (2) no `++`/`--` tokens in the lexer at all, and (3) no
// global or static variables of any kind (every VarSlot is a function-
// local stack slot -- see collect_vars_for_function()'s own doc comment)
// and no I/O a compiled program can perform (no print/write primitive is
// exposed to compiled subset-C source; a compiled program's only
// externally observable effect, ever, is its own single final
// sys_exit(result)). With no assignment expressions, no increment/
// decrement, no mutable shared state, and no I/O reachable from inside an
// expression, no subset-C argument sub-expression can have ANY effect an
// argument evaluated before or after it could observe -- so flipping
// evaluation order is genuinely unobservable here, not merely assumed
// safe by analogy to real C's own well-known unspecified-order allowance.
// The new stack-argument tail this leaves behind lands exactly where a
// real callee's own prologue reads it from (`[rbp+16]`, `[rbp+24]`, ...)
// with zero extra push/pop work -- see the EXPR_CALL codegen's own updated
// doc comment (gen_expr(), below) for the full derivation, and
// gen_program()'s own updated prologue parameter-spill loop for the
// receiving side. CASE 85 (`mix5`, five parameters -- the first call this
// whole codegen has ever made with a genuinely stack-passed argument) and
// CASE 86 (`mix8`, all eight of MAX_PARAMS -- four register, four stack)
// each return a base-10-digit-weighted encoding of every argument's own
// value (the same technique CASE 26's own `mix4` already established for
// proving all 4 registers land correctly) so a single scalar return value
// proves every argument -- register- AND stack-passed alike -- landed in
// its own correct position, not just that some commutative combination of
// them came out right. CASE 87 proves the cap itself: a 9-parameter
// function definition (MAX_PARAMS + 1) is still a real, disclosed
// ParseError::TooManyParams -- the cap moved, it was not removed.
//
// MILESTONE 108: single-dimension fixed-size `int` array declarations,
// indexed reads, and indexed writes -- a real, deliberately narrow first
// slice of the longest-standing gap this file's own "still genuinely
// open" disclosures have named in every compiler-frontend milestone
// since Milestone 83 ("no arrays/pointers"), the same "narrow, honestly
// scoped first slice, not the whole feature at once" discipline `char`
// itself used across Milestone 102-105. Grammar delta:
//   decl_stmt   := "int" IDENT ("[" INTLIT "]")? ";" | ... (unchanged otherwise)
//   factor      := ... | IDENT "[" cond_expr "]"
//   assign_stmt := IDENT "[" cond_expr "]" "=" cond_expr ";" | ... (unchanged otherwise)
// `int arr[N];` (`N` a real positive INTLIT, not a general cond_expr --
// real C requires a constant expression for a plain array declarator's
// own bound too, so this is a genuine subset of real C grammar, not an
// invented restriction) reserves `N` contiguous 8-byte stack slots
// (collect_vars_rec()'s own new per-STMT_DECL branch, see its doc
// comment for the full derivation); `arr[idx]` as an expression operand
// (EXPR_INDEX, new) or as an assignment target (STMT_ASSIGN_INDEX, new
// -- deliberately its own statement kind, not a flag on STMT_ASSIGN, the
// same "new kind over a reused kind with a flag" choice STMT_DOWHILE
// already made over a flag on STMT_WHILE) computes a real RUNTIME
// address (`&arr[0] - idx*8`, four newly hand-verified CodeBuf
// encodings: emit_lea_rax_rbp_off(), emit_shl_rcx_imm8(),
// emit_mov_rax_mem_rax(), emit_mov_mem_rcx_rax() -- see each one's own
// doc comment for the full Intel SDM derivation) rather than folding the
// index into a compile-time-constant disp8 the way every scalar
// variable access this codegen has ever emitted, since Milestone 68,
// could. `idx` is any real cond_expr, not just a literal -- CASE 89
// below indexes with a real `while`-loop counter, proving this is
// genuine runtime addressing, not literal-index sugar. Scope,
// deliberately real and disclosed: `int` arrays only (no `char[]`); no
// array initializer lists (`int arr[3] = {1,2,3};` is not in this
// grammar); no multi-dimensional arrays; no arrays as function
// parameters or return values (no array-to-pointer decay exists in this
// subset at all -- there being no pointer type yet either, the other
// half of the "arrays/pointers" gap this milestone's own name still only
// partially closes); no compound-assignment operators on an array target
// (`arr[i] += 1;` is real, disclosed future work); no bounds checking
// (an out-of-range index reads/writes whatever real stack memory the
// computed address lands on, exactly the same real undefined behavior an
// unchecked C array access already has on genuine hardware -- not a new
// gap this codegen introduces, and consistent with this file's own
// long-standing "no bounds-check panic path, so no core::fmt gets
// pulled into this freestanding panic=abort binary" reason for never
// having runtime bounds checks anywhere in this codegen); a bare array
// name used as an expression with no index (`return arr;`) is not
// rejected at parse or codegen time -- it silently reads whatever is at
// the array's own element-0 slot as if it were an ordinary scalar, a
// real, disclosed simplification of this first slice rather than a
// hard error, since detecting it would need a real symbol table
// tracking each name's own declared shape, which this codegen does not
// build (find_var()'s own linear scan resolves a name to an offset only,
// with no "is this an array" bit threaded back to either call site).
//
// MILESTONE 109: single-level `int` pointer declaration, address-of, and
// dereference (read AND write) -- the other half of the "arrays/pointers"
// gap Milestone 108's own disclosure named as still open, a real,
// deliberately narrow first slice, the same "narrow, honestly scoped
// slice, not the whole feature at once" discipline `char` (Milestone
// 102-105) and arrays (Milestone 108) both already used. Grammar delta:
//   decl_stmt   := "int" "*" IDENT ("=" logic_or)? ";" | ... (unchanged otherwise)
//   unary       := "&" IDENT | "*" unary | ("-"|"~"|"!") unary | factor
//   stmt        := "*" IDENT "=" cond_expr ";" | ... (unchanged otherwise)
// `int *p;` (parse_pointer_decl(), new) produces an ORDINARY scalar
// STMT_DECL, byte-identical in shape to a plain `int p;` -- a pointer's
// own runtime VALUE is just another 8-byte stack slot holding an address,
// exactly like an ordinary `int` holds a number, so collect_vars_rec()/
// find_var()/VarSlot need no new field and no new stack-layout logic at
// all (checked directly: this milestone touches ZERO lines in either
// function). `&x` (OP_ADDR, new EXPR_UNARY op) computes `x`'s own stack
// address via `emit_lea_rax_rbp_off()` -- Milestone 108's own array-base-
// address encoding, reused completely UNCHANGED, an ordinary scalar's
// `rbp`-relative slot being exactly as valid an LEA operand as an array's
// element-0 slot already was. `*p` as an rvalue (OP_DEREF, new EXPR_UNARY
// op) evaluates `p` for its VALUE then loads through it via
// `emit_mov_rax_mem_rax()` -- Milestone 108's own array-element-load
// encoding, also reused completely unchanged. `*p = expr;`
// (STMT_ASSIGN_DEREF, new statement kind, the same "new kind over a
// reused kind with a flag" choice STMT_ASSIGN_INDEX/STMT_DOWHILE already
// made) reads `p`'s value, evaluates the RHS, stores through
// `emit_mov_mem_rcx_rax()` -- Milestone 108's own array-element-store
// encoding, also reused completely unchanged. This milestone needed
// **zero new CodeBuf encodings**: a single-level pointer read/write is,
// mechanically, exactly an array-element read/write with the index
// computation folded away (the address comes directly from a variable's
// own value instead of a runtime `base - idx*8` computation), so the
// exact three encodings Milestone 108 already hand-verified against the
// Intel SDM cover the complete real machine-code surface this milestone
// needs. Scope, deliberately real and disclosed: single-level pointers to
// `int` only (no `char *`, no `int **`); NO pointer arithmetic (`p + 1`
// compiles as ordinary integer addition on a pointer's own raw address
// value -- ADDS to it, unscaled, exactly the "real integer op, no type-
// aware scaling" behavior this codegen already gives every other value,
// since there is no symbol table anywhere in this file tracking a
// variable's own declared shape for `+`'s codegen to consult -- not
// rejected, not silently made "safe", a real, disclosed gap, not new
// here: EXPR_INDEX's own bare-array-name simplification, Milestone 108,
// already established the same "no shape-tracking, so no shape-aware
// rejection" precedent this reuses); NO array-to-pointer decay (so still
// no arrays as function parameters or return values); NO pointers as
// function parameters or return values either (a real, deliberate scope
// cut for THIS milestone, disclosed future work, not attempted -- the
// calling-convention plumbing Milestone 72/107 built has no reason it
// COULDN'T carry a pointer value exactly like any other `int`-sized
// value, this just was not exercised or verified here); `&`'s own operand
// is restricted to a single plain IDENT (no `&arr[i]`, no `&*p`, no
// `&(expr)` -- parse_unary()'s own Milestone 109 doc comment has the full
// grammar reasoning); no `*p += 1;`-style compound assignment through a
// pointer (the same real gap Milestone 108 already left open for
// `arr[i] += 1;`, now disclosed again for the pointer case, not newly
// introduced); no NULL-pointer concept and no bounds/validity checking of
// any kind on a dereference (an uninitialized or wild pointer value
// dereferences to whatever real memory the computed address lands on,
// the same real undefined behavior an unchecked C pointer dereference
// already has on genuine hardware -- consistent with this file's own
// long-standing "no bounds-check panic path, so no core::fmt gets pulled
// into this freestanding panic=abort binary" reason for never having any
// runtime safety check anywhere in this codegen, Milestone 108's own
// array-bounds disclosure being the most recent precedent).
//
// MILESTONE 110: real array-to-pointer decay, and pointers -- and,
// through decay, arrays -- crossing a real function-call boundary for
// the first time. Grammar delta:
//   param       := ("int" "*"? | "char") IDENT | ... (unchanged otherwise)
//   function    := ("int" "*"? | "char") IDENT "(" params? ")" "{" stmt* "}"
// (both `*`s optional, `char *`/`char` return unaffected). `int *p` as a
// parameter (`parse_params()`) and `int *` as a return type
// (`parse_function()`) both need NO new `ParamInfo`/`FuncDef` field and
// NO codegen change -- a pointer value is register/stack-passed and
// returned exactly like an ordinary `int` (Milestone 109's own design
// point), so accepting the grammar IS the complete feature on both
// sides.
//
// The real new codegen is a single new `VarSlot` field, `is_array`
// (`collect_vars_rec()`'s own array-declarator branch sets it 1 for an
// array's element-0 slot, 0 for everything else -- every scalar,
// pointer, and parameter, plus an array's own blank filler elements),
// threaded through `find_var()`'s now-three-element return tuple to
// every one of its six call sites. Two real behavior changes follow
// from it: (1) `EXPR_IDENT`'s own read of a bare array name now DECAYS
// -- computes the slot's own ADDRESS (`emit_lea_rax_rbp_off()`,
// Milestone 108's own array-base encoding, reused unchanged) instead of
// loading the 8-byte VALUE sitting in element 0, closing Milestone
// 108's own disclosed "a bare array name silently reads its own
// element-0 slot as an ordinary scalar" simplification as a real side
// effect, not a second deliberate change -- and since `EXPR_CALL`'s own
// argument codegen is ordinary `gen_expr()` recursion with zero
// special-casing, `foo(arr)` now passes a real address automatically,
// no new call-site codegen needed at all; (2) `EXPR_INDEX`/
// `STMT_ASSIGN_INDEX` (`p[idx]`/`p[idx] = expr;`) now compute their own
// base address two different ways depending on `is_array`: an ARRAY's
// base is the slot's own address (`emit_lea_rax_rbp_off()`, unchanged
// from Milestone 108); a POINTER's base is the slot's own VALUE
// (`emit_mov_rax_rbp_off()`, an ordinary scalar read, reused unchanged)
// -- real C's own `p[idx] == *(p+idx)` rule, now genuinely true here
// for a pointer variable too, not only a declared array. Both
// subtract-based index arithmetic sequences are otherwise UNCHANGED
// from Milestone 108 -- see `EXPR_INDEX`'s own doc comment (below) for
// why reusing the identical subtract direction for a decayed pointer is
// not a coincidence but a structural consequence of this codegen's own
// downward-growing array layout. **Zero new x86_64 encodings**: every
// instruction this milestone's new codegen paths emit
// (`emit_lea_rax_rbp_off()`/`emit_mov_rax_rbp_off()`/`emit_mov_rax_
// mem_rax()`/`emit_mov_mem_rcx_rax()`) already existed before Milestone
// 108 finished, the fourth milestone in a row (108, 109, now 110) to
// add real new capability purely by branching among machine-code
// sequences Milestone 108 already hand-verified against the Intel SDM.
//
// Scope, deliberately real and disclosed: `int` only (no `char *`
// parameters/returns, matching every other `char`-pointer restriction
// this file already carries); a pointer parameter's own declarator is
// `int *name` only, no `int name[]` bracket-array parameter sugar (real
// C treats the two as equivalent; this subset accepts only the pointer
// spelling, a genuine, deliberate narrowing); no multi-level pointer
// parameters/returns (`int **`, unreachable anyway -- see Milestone
// 109's own scope); STILL no pointer arithmetic, so an array/pointer
// argument beyond simple whole-array decay (e.g. passing `&arr[2]` to
// start a callee partway through) is not reachable (`&arr[2]` itself
// remains real, disclosed non-grammar -- Milestone 109's own `&`
// restriction, untouched here); no new error path this milestone adds
// (this codegen still has no type system of any kind, so passing a
// plain `int` where a pointer is semantically expected, or vice versa,
// is compiled without complaint exactly as it already silently was --
// the same "no shape-tracking, so no shape-aware rejection" limitation
// Milestone 108/109 both already disclosed, not new here); `arr = expr;`
// (assigning directly to a bare array name, always silently legal since
// Milestone 108, since STMT_ASSIGN's own codegen never consulted
// `is_array`) is now a genuine, disclosed ASYMMETRY with the read side
// this milestone did not resolve -- reading `arr` decays to its
// address, but writing to `arr` directly still overwrites element 0's
// raw VALUE, unchanged from Milestone 108's own original codegen; a
// real, honest gap for a future milestone to either reject at codegen
// time or otherwise address, not silently smoothed over here.
//
// MILESTONE 111: no grammar change at all -- resolves exactly the
// `arr = expr;` read/write asymmetry Milestone 110's own closing
// disclosure named above, the second of the two candidates that
// milestone's own README.md entry offered next (`char *` being the
// other). `STMT_ASSIGN`'s codegen arm (`gen_stmt_list()`, below) now
// looks up the target's `is_array` flag (`find_var()`'s own Milestone
// 110 third field, already threaded everywhere -- no new plumbing
// needed) BEFORE compiling the RHS, and returns a new
// `CodeGenError::ArrayNotAssignable(u64)` (carrying the array name's own
// source byte offset, the same convention `UndeclaredVariable`/
// `UndefinedLabel` already use) instead of ever reaching the store. Real
// C's own answer to "assign to an array" is "not a modifiable lvalue,
// reject it" -- this milestone makes the identical call rather than
// inventing whole-array-copy semantics this subset's flat 8-byte
// STMT_ASSIGN store was never built to perform. Because Milestone 84's
// compound-assignment operators (`+=`, `-=`, ...) desugar to an ordinary
// `STMT_ASSIGN` at PARSE time (parse_assign_stmt(), unchanged), this one
// check also correctly rejects `arr += 1;` and every other `op=` form
// through the exact same codegen arm -- not a second check, a genuine
// side effect of fixing the one shared path. Every other array/pointer
// gap this file's own Milestone 108/109/110 sections already disclosed
// (no `char *`, no `int **`, no pointer arithmetic, no bounds checking,
// `int name[]` parameter sugar, no `*p += 1;`/`arr[i] += 1;` compound
// element assignment) remains open and untouched by this milestone --
// this is a single, narrow, honestly-scoped fix to one named asymmetry,
// not a broader array/pointer pass.
//
// MILESTONE 112: real `char *` pointers -- the OTHER candidate Milestone
// 110's own closing disclosure named alongside the `arr = expr;`
// asymmetry Milestone 111 just resolved. A deliberately narrow first
// slice, the same discipline every prior array/pointer milestone (108
// through 111) already used: `char *p;` declaration (`parse_pointer_
// decl()`, extended with a new `is_char_pointee` parameter -- `char`
// immediately followed by `*` is now ALSO a pointer declarator,
// TOK_CHAR's own arm in parse_stmt() gaining the exact one-token-
// lookahead check TOK_INT's arm already had), `&c` (already worked
// unconditionally for any variable, `int` or `char`, zero change), and a
// real byte-narrowing dereference-READ (`*p`). Grammar delta: none at
// all beyond the one new lookahead branch -- `*`/`&` already exist in
// every grammar position they need.
//
// Unlike Milestone 109/110 (which needed ZERO new x86_64 encodings,
// every new capability built purely by branching among sequences
// Milestone 108 already hand-verified), this milestone needs exactly
// ONE genuinely new encoding: `emit_movsx_rax_mem_rax_byte()`
// (`MOVSX RAX, BYTE PTR [RAX]`, REX.W `0F BE` /r, ModRM `0x00`) --
// real char*-dereference-read narrowing. Reasoning for why it's needed
// at all: `char *`'s only real correctness requirement THIS milestone
// exercises is that `*p` reads back the pointee's own low byte,
// sign-extended, exactly matching what an ordinary `char` SCALAR's own
// read already does (`char c = 300;` reads back `44`; `int *`'s
// dereference has never narrowed anything, since `int` never narrows).
// Since a `char` variable's own stack slot is STILL a full 8 bytes
// (unchanged since Milestone 102 -- `char` never got a packed 1-byte
// slot, only a narrower READ), the existing `emit_mov_rax_mem_rax()`
// (`int *`'s own full-width dereference-read) would read back the
// pointee's RAW, un-narrowed 8-byte value -- wrong for a `char *`. The
// new encoding is a real, deliberate COMBINATION of two halves each
// independently already hand-verified against the Intel SDM by an
// earlier milestone: `emit_movsx_rax_rbp_off_byte()`'s own opcode bytes
// (Milestone 102) and `emit_mov_rax_mem_rax()`'s own `[RAX]`
// register-indirect ModRM byte (Milestone 108) -- see that function's
// own doc comment for the full derivation.
//
// The one genuinely new `VarSlot` field this milestone adds,
// `is_char_ptr` (see that field's own doc comment for the full
// reasoning), is a real, DELIBERATELY SEPARATE property from `is_char`:
// a `char *p;`'s own slot still has `is_char == 0` (reading `p` ITSELF,
// e.g. `q = p;`, must load the full 8-byte address unnarrowed) and
// `is_char_ptr == 1` (reading `*p` should narrow). Threaded from
// `parse_pointer_decl()`'s new boolean parameter into `s.expr`
// (otherwise always dead for a pointer-declarator STMT_DECL -- the
// pointer's own optional initializer is handled by a separately chained
// STMT_ASSIGN, exactly as before this milestone, so `expr` was never
// used for anything on THIS statement kind) through to
// `collect_vars_rec()`'s VarSlot construction, and out through
// `find_var()`'s now-four-element return tuple (a new SEVENTH call
// site, `EXPR_UNARY`'s own `OP_DEREF` arm, which never called
// `find_var()` at all before this milestone -- `int *` dereference never
// needed to distinguish anything about the pointee).
//
// `*p = expr;` (the WRITE side, `STMT_ASSIGN_DEREF`) is DELIBERATELY
// UNCHANGED -- always the ordinary full 8-byte store, for both `int *`
// and `char *` alike, exactly matching the convention an ordinary `char`
// SCALAR's own store already uses (`char c = expr;` also always writes
// the full 8 bytes, narrowing only ever happening on read). Since the
// pointee, in every case this subset can reach, is always another
// VarSlot's own full 8-byte stack slot (no packed multi-char buffer
// exists anywhere in this language -- no `char[]`, no `malloc`-backed
// byte buffer), writing all 8 bytes through the pointer lands on the
// identical bytes an ordinary direct write to that same variable already
// would -- a real, deliberate design choice, not an oversight.
//
// A real, deliberate, DISCLOSED scope restriction on the READ side:
// `*p`'s char-narrowing is only recognized when `p` is syntactically a
// plain `EXPR_IDENT` (checked directly on `e.left`'s own AST node kind).
// This is not a limitation that costs anything reachable in THIS
// milestone's own scope -- `char *` cannot cross a function-call
// boundary (no `char *` parameters or return values, the same real,
// deliberate scope cut Milestone 109 itself used for `int *` before
// Milestone 110 later closed it), there is no array-to-pointer decay to
// a `char *` (no `char[]` exists anywhere in this subset), and there is
// no pointer arithmetic -- so a local `char *` DECLARATION, optionally
// initialized via `&c`, is the ONLY way this milestone's own grammar can
// ever construct a `char *` value, and every such value is always named
// by a plain IDENT at its one real use site (`*p`). A future milestone
// extending `char *` across a call boundary the way Milestone 110 did
// for `int *` would need to revisit this restriction (and would likely
// need real, declared-type tracking through `ParamInfo`/`FuncDef`, which
// does not exist today for ANY type in this codegen).
//
// Scope, deliberately real and disclosed: single-level `char *` only (no
// `char **`); no `char *` function parameters or return values; no
// array-to-pointer decay (no `char[]` exists to decay from); no pointer
// arithmetic (the same standing gap `int *` has carried since Milestone
// 109); no `p[idx]` byte-scaled indexing through a `char *` --
// `EXPR_INDEX`/`STMT_ASSIGN_INDEX` are UNTOUCHED by this milestone, so
// `char *cp; return cp[0];` would compile and run but still perform the
// pre-existing `int`-style 8-byte-scaled index arithmetic and an
// un-narrowed 8-byte load, a real, disclosed, NOT silently-corrected gap
// for a future milestone (this asymmetry with `*p`'s own real narrowing
// is deliberately not resolved here, honestly named rather than
// papered over); no `*p += 1;`-style compound assignment through a
// `char *` (the same standing gap `int *` already has); no NULL-pointer
// concept and no bounds/validity checking on any dereference (the same
// standing gap every pointer milestone since 109 has carried, for the
// same reasons).
//
// MILESTONE 113: `char *` crossing a real function-call boundary --
// parameters AND return values -- for the first time, the exact
// candidate Milestone 112's own closing disclosure named ("a future
// milestone extending `char *` across a call boundary ... would likely
// need real, declared-type tracking through `ParamInfo`/`FuncDef`,
// which does not exist today for ANY type in this codegen"), the same
// step Milestone 110 already took for `int *` after Milestone 109.
// Grammar delta:
//   param       := ("int" "*"? | "char" "*"?) IDENT | ... (unchanged otherwise)
//   function    := ("int" "*"? | "char" "*"?) IDENT "(" params? ")" ...
// (both `*`s optional after EITHER keyword now; previously `char`'s own
// `*` was only legal in a local `decl_stmt`, never here).
//
// **The one genuinely new field**: `ParamInfo::is_char_ptr` (a new `u8`
// on a struct that, until now, had exactly one field, `is_char`) --
// `parse_params()`'s own `char` arm now checks for a following `*`
// exactly the way its `int` arm already did (Milestone 110), but for
// `char` this genuinely changes what gets recorded: a `char *p`
// parameter must end up with `is_char == 0` (its OWN value is an
// address, never narrowed on read) and `is_char_ptr == 1` (a
// dereference `*p` through it should narrow) -- the identical
// `is_char`/`is_char_ptr` split `VarSlot` has carried for a LOCAL `char
// *` since Milestone 112, now reaching a parameter's own `ParamInfo` for
// the first time. `collect_vars_for_function()` threads `p.is_char_ptr`
// straight into the parameter's own `VarSlot` (previously hardcoded 0,
// per Milestone 112's own explicit "always 0 for a parameter" comment,
// now genuinely variable) -- the ONLY codegen change this milestone
// makes anywhere. `EXPR_UNARY`'s own `OP_DEREF` arm (Milestone 112)
// already reads this field generically through `find_var()`, with zero
// awareness of whether a given `VarSlot` came from a parameter or a
// local declaration, so a parameter's own `*p` narrows correctly with
// literally no change at that call site -- exactly the "accepting the
// grammar plus threading the one flag IS the complete feature" shape
// Milestone 110 already established for `int *` crossing a call
// boundary. `int *` itself needs no equivalent new field (unchanged from
// Milestone 110): an `int *` parameter's `VarSlot` is already
// `is_char: 0, is_char_ptr: 0`, the correct shape for "an ordinary,
// unnarrowed 8-byte pointer value" with no dedicated flag needed at all.
//
// **The return-type half needs no new field either** -- `parse_function()`'s
// own `char` branch now also checks for a following `*`, but the
// EXISTING `is_char_return: bool` already asked exactly the right
// question ("narrow this function's return value on the way out?"):
// `char *f(...)` sets it `false` (a pointer return, whether to `int` or
// `char`, must never be narrowed -- narrowing would sign-extend the
// returned ADDRESS's own low byte into a corrupted, essentially random
// address, not merely give back a wrong VALUE), the identical treatment
// `int *f(...)` already got from Milestone 110. `STMT_RETURN`'s own
// codegen (unchanged) still just checks the one boolean.
//
// **Zero new x86_64 encodings**: this milestone is pure grammar-plus-one-
// field-threading, the same "real capability, zero new machine code"
// shape Milestone 110 already achieved for `int *` crossing a call
// boundary -- every encoding this milestone's own new test cases exercise
// (register/stack argument passing, RAX-based return, the narrowing
// `emit_movsx_rax_mem_rax_byte()` load) was already hand-verified by an
// earlier milestone (107, 110, 112 respectively).
//
// **A real, disclosed, newly-REACHABLE instance of an old restriction**:
// Milestone 112's own `*p` narrowing is recognized only when the
// dereferenced operand is syntactically a plain `EXPR_IDENT` -- in
// Milestone 112's own scope this cost nothing reachable (no `char *`
// return value existed, so a call result could never even be the operand
// of `*`). THIS milestone makes `char *f(...)` a real return type, so
// `*f(&c)` -- dereferencing a call result DIRECTLY, with no intervening
// local-variable assignment -- is now a real, reachable program for the
// first time, and it falls through to the ordinary un-narrowed load,
// exactly like any other non-IDENT dereference operand already does.
// This milestone does not resolve that restriction (see CASE 106 below
// for the real, measured proof, not just an assumed claim) -- narrowing
// a call result directly would need `gen_expr()`'s own `EXPR_CALL` arm to
// carry pointee-type information forward, real new plumbing genuinely out
// of scope here, honestly disclosed rather than silently left implicit
// or papered over with a case that avoids exercising it.
//
// Scope, deliberately real and disclosed: exactly the two grammar
// positions named above, nothing broader -- still no `char **` anywhere;
// still no array-to-pointer decay to a `char *` (no `char[]` exists to
// decay from, so unlike `int *`/array parameters there is no equivalent
// of Milestone 110's own `sum3(arr)`-style decay case here); still no
// pointer arithmetic; still no `int name[]`/`char name[]` bracket-array
// parameter sugar; still no `p[idx]` byte-scaled indexing through a
// `char *` (Milestone 112's own disclosed gap, untouched, now ALSO
// reachable through a parameter -- `EXPR_INDEX`/`STMT_ASSIGN_INDEX` are
// just as untouched by THIS milestone as they were by 112); still no
// `*p += 1;`-style compound assignment through any pointer; still no type
// system of any kind, so a mismatched `char *`/`int *` argument or return
// is accepted without complaint exactly as every other shape mismatch
// already silently is; still no NULL-pointer concept and no
// bounds/validity checking on any dereference.
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
// MILESTONE 89: the `%` (modulo) operator and its `%=` compound form.
// `%` binds at the same precedence as `*` and `/` (real C: they are one
// level). Codegen reuses `/`'s exact `cqo; idiv rcx` -- `idiv` already
// produces the remainder in RDX alongside the quotient in RAX, so `%`
// is one extra `mov rax, rdx` (one new CodeBuf encoding). `%=` slots
// into Milestone 84's compound-assignment machinery with binop `b'%'`.
const TOK_PERCENT: u8 = 46;   // "%"
const TOK_PERCENTEQ: u8 = 47; // "%="
// MILESTONE 90: the conditional (ternary) operator `? :`. Two single-
// char tokens; the whole feature is one new parser layer above
// `logic_or` and one new EXPR kind whose codegen reuses the same
// jz/jmp/patch_rel32 machinery if/else already uses (zero new CodeBuf
// encodings). `:` has no other use in this subset.
const TOK_QUESTION: u8 = 48; // "?"
const TOK_COLON: u8 = 49;    // ":"
// MILESTONE 91: `switch` / `case` / `default`. Integer case labels only
// (an INTLIT, not a constant expression), C fall-through, `break` jumps
// to the switch end, optional single `default`. The discriminant is
// evaluated ONCE and spilled to a reserved frame slot.
const TOK_SWITCH: u8 = 50;  // keyword "switch"
const TOK_CASE: u8 = 51;    // keyword "case"
const TOK_DEFAULT: u8 = 52; // keyword "default"
// MILESTONE 92: `goto LABEL ;` and `LABEL :` -- completes this subset's
// C control flow. A label is an ordinary IDENT followed by `:` in
// statement position (disambiguated from an assignment by one token of
// lookahead). Forward and backward gotos both work; an undefined label
// is a real CodeGenError::UndefinedLabel.
const TOK_GOTO: u8 = 53; // keyword "goto"

// MILESTONE 99: `do { ... } while ( cond ) ;` -- the third and last
// loop form. Body runs once before the first condition test; `break`
// and `continue` bind to it exactly like the other two loops
// (`continue` targeting the bottom-of-body condition re-check).
const TOK_DO: u8 = 54; // keyword "do"

// MILESTONE 102: `char` -- a second, genuinely narrower local-variable
// type alongside `int`. Statement level and `for`-init only (same two
// declarator sites Milestone 101's multi-declarator work reaches);
// NOT yet a parameter or return type -- see this milestone's own
// "still genuinely open" note below for why that's a real, disclosed
// scope cut rather than an oversight.
const TOK_CHAR: u8 = 55; // keyword "char"

// MILESTONE 108: `[` and `]` -- the one new lexer surface single-
// dimension fixed-size `int` arrays need. Two ordinary single-char
// tokens, lexed exactly like every other single-char punctuation this
// lexer already recognizes (see the `match b { ... }` single-char arm
// below); no lookahead, no ambiguity with any existing token (this
// subset's lexer has never had any other use for either byte).
const TOK_LBRACKET: u8 = 56; // "["
const TOK_RBRACKET: u8 = 57; // "]"

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
// MILESTONE 93: raised 128 -> 256 alongside MAX_FUNCS 4 -> 8 -- a
// six-function program (CASE 61) lexes past 128 tokens. Same "transient
// malloc()ed scratch, zero effect on cc.elf's linked size" reasoning as
// the 64 -> 128 bump at Milestone 70.
const MAX_TOKENS: usize = 256;

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
        // MILESTONE 94: comments -- skipped in the lexer exactly like
        // whitespace, so the parser/codegen never see them. `//` runs to
        // (but not past) the next newline or end of input; `/* ... */`
        // runs to the first `*/` (an unterminated block comment reaching
        // end of input is a real LexError::UnknownChar at the `/`, the
        // same "point at the offending byte" convention every other lex
        // error uses). No nesting (matches real C).
        if b == b'/' && i + 1 < src_len {
            let c = unsafe { core::ptr::read((src_ptr + i + 1) as *const u8) };
            if c == b'/' {
                i += 2;
                while i < src_len && unsafe { core::ptr::read((src_ptr + i) as *const u8) } != b'\n' {
                    i += 1;
                }
                continue;
            }
            if c == b'*' {
                let comment_start = i;
                i += 2;
                let mut closed = false;
                while i + 1 < src_len {
                    let a = unsafe { core::ptr::read((src_ptr + i) as *const u8) };
                    let d = unsafe { core::ptr::read((src_ptr + i + 1) as *const u8) };
                    if a == b'*' && d == b'/' {
                        i += 2;
                        closed = true;
                        break;
                    }
                    i += 1;
                }
                if !closed {
                    return Err(LexError::UnknownChar(comment_start));
                }
                continue;
            }
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
            } else if unsafe { word_is(src_ptr, start, wlen, b"switch") } {
                TOK_SWITCH // MILESTONE 91
            } else if unsafe { word_is(src_ptr, start, wlen, b"case") } {
                TOK_CASE // MILESTONE 91
            } else if unsafe { word_is(src_ptr, start, wlen, b"default") } {
                TOK_DEFAULT // MILESTONE 91
            } else if unsafe { word_is(src_ptr, start, wlen, b"goto") } {
                TOK_GOTO // MILESTONE 92
            } else if unsafe { word_is(src_ptr, start, wlen, b"do") } {
                TOK_DO // MILESTONE 99
            } else if unsafe { word_is(src_ptr, start, wlen, b"char") } {
                TOK_CHAR // MILESTONE 102
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
            // MILESTONE 95: base prefixes. A leading '0' immediately
            // followed by 'x'/'X' scans hex digits; by 'b'/'B' scans
            // binary digits. Both prefixes are unambiguous -- no
            // pre-existing decimal literal can start "0x"/"0b" -- so the
            // decimal path below is byte-for-byte unchanged and every
            // prior test stays green. Deliberate cuts: no C octal
            // (a leading '0' with more digits stays decimal, e.g. `010`
            // is ten, not eight), no digit separators, no U/L suffixes.
            // "0x"/"0b" with no following digit is a real
            // LexError::UnknownChar at the offset where a digit was due.
            if b == b'0' && i + 1 < src_len {
                let p = unsafe { core::ptr::read((src_ptr + i + 1) as *const u8) };
                if p == b'x' || p == b'X' {
                    i += 2;
                    let digit_start = i;
                    loop {
                        if i >= src_len {
                            break;
                        }
                        let c = unsafe { core::ptr::read((src_ptr + i) as *const u8) };
                        let d = if c.is_ascii_digit() {
                            (c - b'0') as u64
                        } else if c >= b'a' && c <= b'f' {
                            (c - b'a' + 10) as u64
                        } else if c >= b'A' && c <= b'F' {
                            (c - b'A' + 10) as u64
                        } else {
                            break;
                        };
                        val = val * 16 + d;
                        i += 1;
                    }
                    if i == digit_start {
                        return Err(LexError::UnknownChar(digit_start));
                    }
                    unsafe { tok_write(toks_ptr, n, Token { kind: TOK_INTLIT, int_val: val, ident_off: 0, ident_len: 0 }) };
                    n += 1;
                    continue;
                }
                if p == b'b' || p == b'B' {
                    i += 2;
                    let digit_start = i;
                    loop {
                        if i >= src_len {
                            break;
                        }
                        let c = unsafe { core::ptr::read((src_ptr + i) as *const u8) };
                        if c == b'0' || c == b'1' {
                            val = val * 2 + (c - b'0') as u64;
                            i += 1;
                        } else {
                            break;
                        }
                    }
                    if i == digit_start {
                        return Err(LexError::UnknownChar(digit_start));
                    }
                    unsafe { tok_write(toks_ptr, n, Token { kind: TOK_INTLIT, int_val: val, ident_off: 0, ident_len: 0 }) };
                    n += 1;
                    continue;
                }
            }
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
        // MILESTONE 97: a character literal 'c' lexes to an ordinary
        // TOK_INTLIT whose value is the character's byte -- no new token
        // kind, no type system, the parser and codegen treat it exactly
        // like the integer it already is. Supports the common
        // single-character escapes: \n \t \r \0 \\ \' \". Deliberate
        // cuts: no multi-character constants ('ab'), no \xNN or octal
        // \NNN numeric escapes, no wide/unicode literals. An empty '',
        // an unterminated ', an unrecognized \escape, or a literal that
        // is not closed by a quote exactly one character later are all a
        // real LexError::UnknownChar pointing at the opening quote --
        // the same "point at the offending byte" convention every other
        // lex error uses.
        if b == b'\'' {
            if i + 1 >= src_len {
                return Err(LexError::UnknownChar(i));
            }
            let c1 = unsafe { core::ptr::read((src_ptr + i + 1) as *const u8) };
            let (val, consumed): (u64, u64) = if c1 == b'\\' {
                if i + 2 >= src_len {
                    return Err(LexError::UnknownChar(i));
                }
                let e = unsafe { core::ptr::read((src_ptr + i + 2) as *const u8) };
                let v: u64 = match e {
                    b'n' => 10,
                    b't' => 9,
                    b'r' => 13,
                    b'0' => 0,
                    b'\\' => 92,
                    b'\'' => 39,
                    b'"' => 34,
                    _ => return Err(LexError::UnknownChar(i)),
                };
                (v, 3)
            } else if c1 == b'\'' {
                // an empty literal ''
                return Err(LexError::UnknownChar(i));
            } else {
                (c1 as u64, 2)
            };
            if i + consumed >= src_len
                || unsafe { core::ptr::read((src_ptr + i + consumed) as *const u8) } != b'\''
            {
                return Err(LexError::UnknownChar(i));
            }
            if n >= max_toks {
                return Err(LexError::TooManyTokens);
            }
            unsafe { tok_write(toks_ptr, n, Token { kind: TOK_INTLIT, int_val: val, ident_off: 0, ident_len: 0 }) };
            n += 1;
            i += consumed + 1;
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
                else if b == b'%' { Some(TOK_PERCENTEQ) } // MILESTONE 89
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
            b'%' => TOK_PERCENT, // MILESTONE 89 (a lone '%'; "%=" is caught above)
            b'?' => TOK_QUESTION, // MILESTONE 90
            b':' => TOK_COLON,    // MILESTONE 90
            b'~' => TOK_TILDE,
            b'[' => TOK_LBRACKET, // MILESTONE 108
            b']' => TOK_RBRACKET, // MILESTONE 108
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
const EXPR_TERNARY: u8 = 6; // MILESTONE 90: `cond ? then : else` -- `left` = cond, `right` = then, `call_args_ptr` (field-reused) = else. Codegen is if/else's exact jz/jmp/patch shape, into RAX.
// MILESTONE 108: `IDENT "[" cond_expr "]"` -- a real array-element READ.
// `ident_off`/`ident_len` name the array (the same fields EXPR_IDENT
// already uses for a plain variable's name -- same field-reuse
// discipline this AST already uses throughout, e.g. EXPR_CALL reusing
// them for its own callee name); `left` (reused from EXPR_UNARY's own
// "the one operand" role) holds the index sub-expression's own
// ExprNode pointer, itself any real cond_expr (not just an INTLIT or
// IDENT) -- `arr[i+1]`, `arr[f(x)]`, `arr[arr2[0]]` are all legal
// parses. `right`/`call_args_ptr`/`call_argc` stay unused (0), the same
// "unused field stays 0" convention every other EXPR kind already
// follows.
const EXPR_INDEX: u8 = 7;

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
// MILESTONE 109: two more EXPR_UNARY op sentinels -- OP_ADDR ("&x",
// address-of) and OP_DEREF ("*p", dereference), the fourth/fifth
// siblings to OP_NEG/OP_BITNOT/OP_LOGNOT above, the exact extension
// point those comments already named. Unlike the other three, OP_ADDR's
// own operand is deliberately NOT a general recursive-unary parse (see
// parse_unary()'s own Milestone 109 doc comment below for why) -- its
// `left` field always points at a plain EXPR_IDENT node naming the
// variable whose address is taken. OP_DEREF's `left`, by contrast, is
// an ordinary sub-expression (usually an EXPR_IDENT read of a pointer
// variable, but the grammar itself does not require that) exactly like
// every other unary op here.
const OP_ADDR: u8 = 17;
const OP_DEREF: u8 = 18;

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
const STMT_SWITCH: u8 = 8; // MILESTONE 91: expr = discriminant, then_body = chain of STMT_CASE, else_body = the `default` body list (0 if none)
const STMT_CASE: u8 = 9; // MILESTONE 91: one `case INTLIT:` -- expr = an EXPR_INTLIT for the label value, then_body = this case's own stmt list, next = the following STMT_CASE
const STMT_LABEL: u8 = 10; // MILESTONE 92: `LABEL :` -- ident_off/ident_len name the label; no body (the following statement is a separate list node)
const STMT_GOTO: u8 = 11; // MILESTONE 92: `goto LABEL ;` -- ident_off/ident_len name the target
const STMT_DOWHILE: u8 = 12; // MILESTONE 99: `do { body } while (cond);` -- expr = cond, then_body = body list; body first, condition at the bottom
// MILESTONE 108: `IDENT "[" cond_expr "]" "=" cond_expr ";"` -- a real
// array-element WRITE. Deliberately a NEW statement kind rather than a
// flag/variant bolted onto STMT_ASSIGN (the same "new kind, not a
// reused kind with a flag" discipline STMT_DOWHILE above already chose
// over a flag on STMT_WHILE): STMT_ASSIGN's own codegen and every one
// of its existing callers stay completely untouched by this milestone,
// zero regression risk to the plain-scalar-assignment path every prior
// milestone already verified. `ident_off`/`ident_len` name the array
// (same role as STMT_ASSIGN's own target-name fields); `expr` is the
// RHS (same field, same role as STMT_ASSIGN); `then_body` is reused
// (NOT a StmtNode list this time -- an ExprNode pointer, the index
// sub-expression) for the one field this shape needs that STMT_ASSIGN
// doesn't have, the same "reuse an existing dead field for a new
// milestone's own data" discipline `then_body`'s own STMT_DECL role
// (Milestone 102's `is_char` tag) already established; `else_body`
// stays unused (0).
const STMT_ASSIGN_INDEX: u8 = 13;
// MILESTONE 109: `"*" IDENT "=" cond_expr ";"` -- a real pointer
// dereference WRITE, the lvalue form of `*p`. Deliberately its OWN
// statement kind (the same "new kind, not a flag on an existing one"
// discipline STMT_ASSIGN_INDEX above already chose over a flag on
// STMT_ASSIGN) -- STMT_ASSIGN's own codegen and every one of its
// existing callers stay completely untouched. `ident_off`/`ident_len`
// name the POINTER variable (`p`, not the pointee) -- same role
// STMT_ASSIGN's own target-name fields already play, just naming a
// variable whose VALUE is an address rather than one written directly;
// `expr` is the RHS (same field, same role as STMT_ASSIGN). `then_body`/
// `else_body` stay unused (0) -- unlike STMT_ASSIGN_INDEX, this shape
// needs no extra field: this subset has no pointer arithmetic, so `*p`
// is the entire real dereference-target grammar (there is no `*(p+1)`
// this milestone's own narrow scope could even reach).
const STMT_ASSIGN_DEREF: u8 = 14;

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
    // MILESTONE 103: 1 if this parameter was declared `char`, 0 for
    // `int` -- the same `is_char` convention `VarSlot` (Milestone 102)
    // already carries for local variables, now reaching function
    // parameters too. `collect_vars_for_function()` copies this
    // straight into the parameter's own `VarSlot`, so a `char`
    // parameter narrows on read exactly like a `char` local: no new
    // codegen at the call site OR in the prologue's own register-spill
    // (`emit_mov_rbp_off_reg()`, unchanged, still spills the full
    // incoming register) -- only `EXPR_IDENT`'s existing narrowed-read
    // path (Milestone 102) is what makes this real.
    is_char: u8,
    // MILESTONE 113: 1 if this parameter was declared `char *`, 0
    // otherwise -- the real "declared-type bit" `collect_vars_for_
    // function()`'s own Milestone 112 doc comment named as the missing
    // piece for extending `char *` across a function-call boundary (it
    // did not exist on this struct until now). Deliberately a SEPARATE
    // field from `is_char` above, not a reuse: a `char *p` parameter's
    // own resulting `VarSlot` must have `is_char == 0` (its OWN value is
    // an address, never narrowed on read -- the same real, disclosed
    // `is_char`/`is_char_ptr` distinction `VarSlot` itself has carried
    // since Milestone 112) and `is_char_ptr == 1` (a dereference `*p`
    // through it should narrow) -- `parse_params()` sets exactly one of
    // `is_char`/`is_char_ptr`, never both, for any single parameter.
    is_char_ptr: u8,
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
///
/// MILESTONE 107: raised 4 -> 8, the same doubling Milestone 93 already
/// used for MAX_FUNCS (4 -> 8) and Milestone 70 used for MAX_TOKENS
/// (64 -> 128) -- and the real stack-passed-argument codegen this
/// constant's own OLD doc comment (above) explicitly deferred is now
/// built (see MAX_REG_PARAMS below, the EXPR_CALL call-site codegen, and
/// gen_program()'s own prologue parameter-spill loop). Only the first
/// MAX_REG_PARAMS (4) arguments/parameters still use a register --
/// argument/parameter index 4 and up now genuinely lives on the stack,
/// real SysV-style, not a further-widened register set (there is no 5th
/// argument register in this ABI, real or invented). 8, not some larger
/// number, because that is exactly what this milestone's own CASE 86
/// (`mix8`, all 8 slots -- 4 register, 4 stack -- individually
/// distinguished by a base-10-digit-weighted return value so a single
/// scalar result proves every argument landed in its own correct
/// position) needs and exercises; raising it further with no concrete
/// program driving it would be exactly the "maximal generality over a
/// small real headroom" mistake this file's own MAX_PENDING_CALLS doc
/// comment below already names as a discipline this codegen avoids.
const MAX_PARAMS: u64 = 8;

/// MILESTONE 107: the maximum number of arguments/parameters that still
/// travel through a real register -- kept as its own named constant,
/// separate from MAX_PARAMS above, specifically so the boundary between
/// "goes in a register" (index 0..3) and "lives on the stack" (index 4
/// and up) is one obviously-named comparison at both real call sites
/// (the EXPR_CALL codegen below, and gen_program()'s own prologue
/// parameter-spill loop) rather than a bare `4` that could silently
/// drift out of sync between the two if MAX_PARAMS is ever raised again.
/// This is a real, physical fact about the ABI (there are only four
/// argument registers, RDI/RSI/RDX/RCX -- see param_reg()'s own doc
/// comment for why exactly these four, in this order), not a tunable
/// knob; it should never need to change independently of the ISA this
/// codegen targets.
const MAX_REG_PARAMS: u64 = 4;

/// MILESTONE 72: the maximum number of function definitions a single
/// program may contain.
/// MILESTONE 93: raised 4 -> 8. Now that the subset expresses full C
/// control flow (Milestone 92), real multi-function test programs
/// naturally want more than four -- Milestones 76/79/84/90 each
/// explicitly weighed and DEFERRED this bump for lack of "a concrete
/// program driving it"; CASE 61 is that program (six functions, one
/// forward call). `funcs_ptr` is a transient `malloc(MAX_FUNCS * size
/// of FuncSym)` scratch buffer freed at process exit, so raising this
/// has zero effect on cc.elf's own compiled/linked size -- exactly the
/// reasoning MAX_TOKENS' own bump (64 -> 128, Milestone 70) already
/// used. `MAX_PARAMS` stays 4: this codegen's calling convention only
/// has four argument registers (RDI/RSI/RDX/RCX) and stack-passed
/// arguments are genuine new codegen, a separate milestone.
const MAX_FUNCS: u64 = 8;

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
    // MILESTONE 103: 1 if this function's own declared return type is
    // `char`, 0 for `int`. Threaded through gen_stmt_list()'s new
    // `is_char_return` parameter into STMT_RETURN's own codegen, where
    // it's the ONLY thing that makes a `char` return type real rather
    // than purely syntactic: see emit_movsx_rax_al()'s own doc comment
    // for why a return statement needs its own narrowing instruction
    // (unlike a plain variable read, there is no stack slot here to
    // narrow through).
    is_char_return: u8,
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
    /// MILESTONE 108: an array declarator's own `[` ... `]` did not
    /// contain a real positive INTLIT (e.g. `int arr[0];` or
    /// `int arr[];`) -- this subset's arrays are fixed-size, and a
    /// zero-or-absent size has no real stack layout collect_vars_rec()
    /// could allocate. This subset also deliberately does not accept a
    /// general cond_expr here (real C doesn't either, for a plain array
    /// declarator -- only a constant expression is legal), so this path
    /// is reachable ONLY by a literal `0` or a missing/non-INTLIT size,
    /// not by anything more elaborate.
    BadArraySize,
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
                            let e = unsafe { self.parse_ternary() }?;
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
                // MILESTONE 108: IDENT "[" cond_expr "]" -- a real
                // array-element read (EXPR_INDEX). The index itself is
                // any real cond_expr (parse_ternary(), the same
                // widened-through-`?:` production every other bracketed/
                // parenthesized sub-expression in this grammar already
                // recurses through -- see parse_factor()'s own
                // TOK_LPAREN arm below), so `arr[i+1]`, `arr[f(x)]`,
                // `arr[arr2[0]]` are all legal parses, not just a bare
                // INTLIT/IDENT.
                } else if unsafe { self.peek() }.kind == TOK_LBRACKET {
                    unsafe { self.advance() }; // consume '['
                    let idx = unsafe { self.parse_ternary() }?;
                    unsafe { self.expect(TOK_RBRACKET) }?;
                    let node = unsafe { alloc_expr() };
                    if node == 0 {
                        return Err(ParseError::OutOfMemory);
                    }
                    unsafe {
                        core::ptr::write(
                            node as *mut ExprNode,
                            ExprNode {
                                kind: EXPR_INDEX,
                                int_val: 0,
                                ident_off: name.ident_off,
                                ident_len: name.ident_len,
                                op: 0,
                                left: idx,
                                right: 0,
                                call_args_ptr: 0,
                                call_argc: 0,
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
                let e = unsafe { self.parse_ternary() }?;
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
        // MILESTONE 98: unary `+` is a genuine no-op on this subset's
        // only type (`int`) -- consume the token and return the
        // operand's own parse directly, with NO EXPR_UNARY node and NO
        // codegen. Same tightest-precedence slot as unary `-`/`~`/`!`,
        // and it composes with them through the same recursion (`+-x`,
        // `-+x`, `+!x`, `-+-x` all parse). Deliberate: it does not force
        // non-negativity or any conversion -- there are no unsigned or
        // narrower types here for it to convert to.
        if k == TOK_PLUS {
            unsafe { self.advance() };
            return unsafe { self.parse_unary() };
        }
        // MILESTONE 109: real unary `&` (address-of) and `*`
        // (dereference) -- the two new EXPR_UNARY op sentinels this
        // milestone adds (OP_ADDR, OP_DEREF), same tightest-precedence
        // unary slot `-`/`~`/`!` already occupy above. `&` is
        // deliberately NARROWER than every other unary operator here:
        // its operand must be a single plain IDENT (this subset's
        // entire real address-of grammar in this first slice -- no
        // `&arr[i]`, no `&*p`, no `&(expr)`; see this milestone's own
        // README entry for the full scope reasoning), so it does NOT
        // recurse into parse_unary()/parse_factor() the way every other
        // unary operator here does -- it builds the EXPR_IDENT operand
        // node directly and expects nothing else. `*` (dereference), by
        // contrast, DOES recurse into parse_unary() below, exactly like
        // `-`/`~`/`!` above it, so it composes with them and with
        // itself through the same recursive-unary call (`-*p`, `!*p`,
        // `*p` nested inside a ternary/call/index all parse). This `*`
        // here is unambiguous with `*`'s other, pre-existing meaning
        // (binary multiplication, parsed by parse_term()'s own loop
        // just below this function): parse_term() always calls
        // parse_unary() FIRST for its left operand before ever checking
        // for a following `*`/`/`/`%`, so a `*` reached here has, by
        // construction, no left operand yet -- it can only be this
        // prefix form, the exact same "resolved by grammar POSITION,
        // not by the token alone" reasoning this function's own
        // Milestone 76 doc comment above already established for unary
        // vs. binary `-`.
        if k == TOK_AMP {
            unsafe { self.advance() };
            let name = unsafe { self.expect(TOK_IDENT) }?;
            let operand = unsafe { alloc_expr() };
            if operand == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    operand as *mut ExprNode,
                    ExprNode { kind: EXPR_IDENT, int_val: 0, ident_off: name.ident_off, ident_len: name.ident_len, op: 0, left: 0, right: 0, call_args_ptr: 0, call_argc: 0 },
                )
            };
            let node = unsafe { alloc_expr() };
            if node == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    node as *mut ExprNode,
                    ExprNode { kind: EXPR_UNARY, int_val: 0, ident_off: 0, ident_len: 0, op: OP_ADDR, left: operand, right: 0, call_args_ptr: 0, call_argc: 0 },
                )
            };
            return Ok(node);
        }
        let op = if k == TOK_MINUS {
            OP_NEG
        } else if k == TOK_TILDE {
            OP_BITNOT
        } else if k == TOK_BANG {
            OP_LOGNOT
        } else if k == TOK_STAR {
            OP_DEREF
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
            if k != TOK_STAR && k != TOK_SLASH && k != TOK_PERCENT {
                break;
            }
            // MILESTONE 89: `%` joins `*`/`/` at this one precedence level.
            let op = if k == TOK_STAR { b'*' } else if k == TOK_SLASH { b'/' } else { b'%' };
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

    /// MILESTONE 90: the conditional operator --
    /// `ternary := logic_or ("?" ternary ":" ternary)?`. Sits directly
    /// above `logic_or` (looser than `||`, looser than everything except
    /// assignment, exactly real C's ordering). Right-associative:
    /// `a ? b : c ? d : e` parses as `a ? b : (c ? d : e)` because the
    /// else branch recurses through `parse_ternary` itself. The middle
    /// operand is also a full `parse_ternary` (real C treats it as a
    /// parenthesis-free full expression). This is now what every "parse
    /// a whole expression" position calls in place of `parse_logic_or`.
    unsafe fn parse_ternary(&mut self) -> Result<u64, ParseError> {
        let cond = unsafe { self.parse_logic_or() }?;
        if unsafe { self.peek() }.kind != TOK_QUESTION {
            return Ok(cond);
        }
        unsafe { self.advance() }; // consume '?'
        let then_e = unsafe { self.parse_ternary() }?;
        unsafe { self.expect(TOK_COLON) }?;
        let else_e = unsafe { self.parse_ternary() }?;
        let node = unsafe { alloc_expr() };
        if node == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                node as *mut ExprNode,
                // field reuse: left=cond, right=then, call_args_ptr=else.
                ExprNode { kind: EXPR_TERNARY, int_val: 0, ident_off: 0, ident_len: 0, op: 0, left: cond, right: then_e, call_args_ptr: else_e, call_argc: 0 },
            )
        };
        Ok(node)
    }

    /// MILESTONE 88: build a STMT_DECL for `name`, and -- if the next
    /// token is `=` -- a combined initializer, desugared to a
    /// STMT_ASSIGN chained right after the decl (`decl.next = assign`).
    /// `consume_semi` is true at statement level (`int i = e;` ends in
    /// `;`), false for a `for` init clause (the `for` parser consumes
    /// the `;`). Only plain `=` starts an initializer -- a compound op
    /// after `int i` would be reading an uninitialized variable, so it
    /// is left to fall through as a parse error.
    ///
    /// MILESTONE 101: one or more comma-separated declarators after a
    /// single `int`. `int a, b = 2, c;` desugars to the flat statement
    /// chain `DECL(a) -> DECL(b) -> ASSIGN(b = 2) -> DECL(c)`. Each
    /// declarator independently may carry its own `= <ternary>`
    /// initializer (`finish_ident_assign` with `consume_semi = false`,
    /// so it stops before the `,` or `;`), and a later initializer may
    /// reference an earlier declarator in the same list (the chain is
    /// sequential DECL/ASSIGN pairs). Works at statement level and as a
    /// `for` init clause. No new AST kind, no new codegen: `STMT_DECL`
    /// is already a codegen no-op and `collect_vars_rec` already
    /// registers every `STMT_DECL` it walks. A dangling `,` with no
    /// following declarator is a real ParseError.
    ///
    /// The declarator-node creation is inlined per iteration (no helper
    /// call): even with Milestone 100's 32 KiB stack, this is the hot
    /// path for every `int` declaration and there is no reason to spend
    /// a frame on it.
    ///
    /// MILESTONE 102: `is_char` records which type keyword introduced
    /// this whole declarator list (`int` -> false, `char` -> true) --
    /// every declarator in one list shares the one leading type, same
    /// as real C's `int a, b;` (no per-declarator `int a, char b;`
    /// mixing exists in this subset, there being no `*` to hang a
    /// different-type declarator off in the first place). Stored in
    /// each STMT_DECL's own `then_body` field (0/1) -- reusing it
    /// rather than growing StmtNode by a field, the same "0 = absent,
    /// reuse before grow" discipline this struct's own doc comment
    /// already establishes; `then_body` is otherwise dead for
    /// STMT_DECL (unused, always left 0 before this milestone).
    /// `collect_vars_rec` reads it back out into each `VarSlot`.
    unsafe fn make_decl(&mut self, name: Token, consume_semi: bool, is_char: bool) -> Result<u64, ParseError> {
        let type_tag: u64 = if is_char { 1 } else { 0 };
        let head = unsafe { alloc_stmt() };
        if head == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                head as *mut StmtNode,
                StmtNode { kind: STMT_DECL, ident_off: name.ident_off, ident_len: name.ident_len, expr: 0, next: 0, then_body: type_tag, else_body: 0 },
            )
        };
        let mut tail = head;
        if unsafe { self.peek() }.kind == TOK_ASSIGN {
            let assign = unsafe { self.finish_ident_assign(name, false) }?;
            unsafe { (*(tail as *mut StmtNode)).next = assign };
            tail = assign;
        }
        while unsafe { self.peek() }.kind == TOK_COMMA {
            unsafe { self.advance() }; // ','
            let nm = unsafe { self.expect(TOK_IDENT) }?;
            let d = unsafe { alloc_stmt() };
            if d == 0 {
                return Err(ParseError::OutOfMemory);
            }
            unsafe {
                core::ptr::write(
                    d as *mut StmtNode,
                    StmtNode { kind: STMT_DECL, ident_off: nm.ident_off, ident_len: nm.ident_len, expr: 0, next: 0, then_body: type_tag, else_body: 0 },
                )
            };
            unsafe { (*(tail as *mut StmtNode)).next = d };
            tail = d;
            if unsafe { self.peek() }.kind == TOK_ASSIGN {
                let assign = unsafe { self.finish_ident_assign(nm, false) }?;
                unsafe { (*(tail as *mut StmtNode)).next = assign };
                tail = assign;
            }
        }
        if consume_semi {
            unsafe { self.expect(TOK_SEMI) }?;
        }
        Ok(head)
    }

    /// MILESTONE 108: `"int" IDENT "[" INTLIT "]" ";"` -- a real, fixed-
    /// size single-dimension `int` array declaration. `name` and the
    /// leading `int` are already consumed by the caller (TOK_INT's own
    /// arm in parse_stmt(), the same "IDENT already consumed before this
    /// helper runs" shape make_decl() above already uses). Deliberately
    /// its OWN standalone parse path, not folded into make_decl()'s own
    /// comma-separated multi-declarator/initializer chain: this first
    /// slice supports exactly `int arr[N];` as a complete statement --
    /// no `=` initializer (an array initializer list is real, disclosed
    /// future work, not attempted here), no mixing an array declarator
    /// into an `int a, b[3], c;` list alongside plain scalars, no
    /// `char` arrays (`char` stays a scalar-only type, exactly as
    /// Milestone 102 left it) -- the same "narrow, real, honestly
    /// scoped first slice" discipline `char` itself used across
    /// Milestone 102-105 rather than attempting the whole feature at
    /// once. The size must be a real INTLIT (a constant), not any
    /// general cond_expr -- real C requires a constant expression for a
    /// plain array declarator's own bound too, so this is a genuine
    /// subset of real C grammar, not an invented restriction -- and
    /// must be strictly positive (`int arr[0];` is a real, disclosed
    /// ParseError::BadArraySize, not a silently-accepted zero-length
    /// array). The one AST field this whole feature needs beyond an
    /// ordinary scalar STMT_DECL is the element count, stored in
    /// `else_body` (a u64, always 0/unused for every scalar STMT_DECL
    /// before this milestone -- see StmtNode's own doc comment) --
    /// `collect_vars_rec()` below reads it back out to reserve the real
    /// contiguous stack slots this array needs. `is_char` is always
    /// `false` here (only TOK_INT's own arm calls this; TOK_CHAR's arm
    /// is completely untouched by this milestone).
    unsafe fn parse_array_decl(&mut self, name: Token) -> Result<u64, ParseError> {
        unsafe { self.expect(TOK_LBRACKET) }?;
        let size_tok = unsafe { self.peek() };
        if size_tok.kind != TOK_INTLIT || size_tok.int_val == 0 {
            return Err(ParseError::BadArraySize);
        }
        unsafe { self.advance() };
        unsafe { self.expect(TOK_RBRACKET) }?;
        unsafe { self.expect(TOK_SEMI) }?;
        let node = unsafe { alloc_stmt() };
        if node == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                node as *mut StmtNode,
                StmtNode { kind: STMT_DECL, ident_off: name.ident_off, ident_len: name.ident_len, expr: 0, next: 0, then_body: 0, else_body: size_tok.int_val },
            )
        };
        Ok(node)
    }

    /// MILESTONE 109: `"*" IDENT ("=" logic_or)? ";"` -- a real single-
    /// level `int` pointer declarator, `"int"` and the pointer's own
    /// leading `"*"` already consumed by the caller (TOK_INT's own arm
    /// in parse_stmt(), same "already consumed before this helper runs"
    /// shape parse_array_decl() above already uses -- there `name` is
    /// consumed before the `[`; here `name` is consumed AFTER the `*`,
    /// the real grammar difference between the two declarator shapes).
    /// Deliberately narrow: exactly ONE pointer declarator per
    /// statement (no `int *p, *q;` list, no mixing a pointer declarator
    /// into an `int a, *p, b[3];` list) -- the same "own standalone
    /// path, not folded into make_decl()'s comma-chain" discipline
    /// parse_array_decl() itself already established for arrays.
    /// Reuses make_decl()'s own optional `"=" <ternary>` initializer
    /// support directly (finish_ident_assign(), unchanged), so
    /// `int *p = &x;` is a real, legal combined declaration+initializer
    /// in one statement, same as any plain scalar `int i = e;` since
    /// Milestone 88.
    ///
    /// Produces an ORDINARY scalar STMT_DECL (then_body=0, else_body=0)
    /// -- byte-identical in shape to a plain `int p;` declaration. This
    /// is a deliberate design choice, not an oversight: a pointer's
    /// runtime VALUE is just another 8-byte stack slot holding an
    /// address, exactly like an ordinary `int` holds a number --
    /// collect_vars_rec()/find_var() need no new VarSlot field and no
    /// new stack-layout logic, and gen_expr()'s existing EXPR_IDENT read
    /// of `p` (an ordinary 8-byte load) is already exactly correct for
    /// reading a pointer's own value. The only genuinely NEW codegen
    /// this milestone adds lives entirely in EXPR_UNARY's two new op
    /// cases (OP_ADDR for `&x`, OP_DEREF for `*p` as an rvalue) and the
    /// new STMT_ASSIGN_DEREF statement kind (for `*p = expr;` as an
    /// lvalue) -- see their own doc comments for the full derivation,
    /// and this file's own top Milestone 109 doc comment for why zero
    /// new CodeBuf encodings were needed anywhere in this milestone.
    // MILESTONE 112: new `is_char_pointee` parameter -- true for a
    // `char *p ...;` declarator (TOK_CHAR's own arm in parse_stmt(), new),
    // false for the pre-existing `int *p ...;` shape (TOK_INT's own arm,
    // unchanged in every other respect). Threaded into the produced
    // STMT_DECL's own `expr` field (0/1 -- otherwise always 0/dead for a
    // pointer declarator, since a pointer's own initializer is handled by
    // the separate chained STMT_ASSIGN below, exactly as before this
    // milestone) purely so collect_vars_rec() can recover it later and
    // set the new VarSlot `is_char_ptr` flag -- see that struct field's
    // own doc comment for the full reasoning and why this is safe (every
    // OTHER STMT_DECL producer in this file still always writes `expr:
    // 0`, so this new meaning never collides with anything declared
    // before this milestone).
    unsafe fn parse_pointer_decl(&mut self, name: Token, is_char_pointee: bool) -> Result<u64, ParseError> {
        let head = unsafe { alloc_stmt() };
        if head == 0 {
            return Err(ParseError::OutOfMemory);
        }
        unsafe {
            core::ptr::write(
                head as *mut StmtNode,
                StmtNode { kind: STMT_DECL, ident_off: name.ident_off, ident_len: name.ident_len, expr: if is_char_pointee { 1 } else { 0 }, next: 0, then_body: 0, else_body: 0 },
            )
        };
        if unsafe { self.peek() }.kind == TOK_ASSIGN {
            let assign = unsafe { self.finish_ident_assign(name, false) }?;
            unsafe { (*(head as *mut StmtNode)).next = assign };
        }
        unsafe { self.expect(TOK_SEMI) }?;
        Ok(head)
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
            else if opk == TOK_PERCENTEQ { Some(b'%') } // MILESTONE 89
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
        let rhs = unsafe { self.parse_ternary() }?;
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
                // MILESTONE 88: `int IDENT ("=" logic_or)? ";"` -- a
                // combined declaration-with-initializer. `int i = e;` is
                // desugared to `int i;` immediately followed by `i = e;`
                // (a two-node chain -- parse_stmt_list_until_rbrace()
                // already walks to a chain's real tail since Milestone
                // 86). No new codegen, no new AST kind: STMT_DECL is
                // unchanged (still what collect_vars() counts), and the
                // synthesized STMT_ASSIGN uses the exact
                // finish_ident_assign() path a plain `i = e;` already
                // takes.
                unsafe { self.advance() };
                // MILESTONE 109: `int` immediately followed by `*` is a
                // pointer declarator (`int *p ...;`) -- checked BEFORE
                // the IDENT is consumed, since the `*` sits between the
                // type keyword and the name here (unlike the array
                // declarator's own `[`, which comes AFTER the name, or
                // the plain scalar case, which has no extra punctuation
                // at all). One token of lookahead, same "peek before
                // deciding which declarator shape this is" discipline
                // Milestone 108 already used for array-vs-scalar just
                // below.
                if unsafe { self.peek() }.kind == TOK_STAR {
                    unsafe { self.advance() }; // consume '*'
                    let name = unsafe { self.expect(TOK_IDENT) }?;
                    return unsafe { self.parse_pointer_decl(name, false) };
                }
                let name = unsafe { self.expect(TOK_IDENT) }?;
                // MILESTONE 108: one token of lookahead after the IDENT
                // decides between the pre-existing scalar declarator
                // chain (make_decl(), completely unchanged -- every
                // scalar `int` declaration this codegen has ever parsed
                // still takes exactly that path) and a genuine array
                // declarator (parse_array_decl(), new). The same
                // "advance past the name, then peek to disambiguate"
                // shape this file's own TOK_IDENT statement-dispatch
                // arm below already uses for label-vs-assignment.
                if unsafe { self.peek() }.kind == TOK_LBRACKET {
                    unsafe { self.parse_array_decl(name) }
                } else {
                    unsafe { self.make_decl(name, true, false) }
                }
            }
            // MILESTONE 102: `char IDENT ("=" logic_or)? ("," ...)* ";"`
            // -- the exact same grammar TOK_INT's arm just above parses,
            // through the exact same make_decl(), with `is_char = true`
            // the only difference. A `char` declarator list is otherwise
            // indistinguishable to the parser from an `int` one: same
            // multi-declarator chaining (Milestone 101), same optional
            // `=` initializer per declarator.
            // MILESTONE 112: `char` immediately followed by `*` is now
            // ALSO a pointer declarator (`char *p ...;`), the exact same
            // one-token-lookahead check TOK_INT's own arm above already
            // uses for `int *p ...;` -- checked BEFORE the IDENT is
            // consumed, for the identical reason (the `*` sits between
            // the type keyword and the name). Reuses `parse_pointer_decl()`
            // completely, just with `is_char_pointee = true` this time.
            TOK_CHAR => {
                unsafe { self.advance() };
                if unsafe { self.peek() }.kind == TOK_STAR {
                    unsafe { self.advance() }; // consume '*'
                    let name = unsafe { self.expect(TOK_IDENT) }?;
                    return unsafe { self.parse_pointer_decl(name, true) };
                }
                let name = unsafe { self.expect(TOK_IDENT) }?;
                unsafe { self.make_decl(name, true, true) }
            }
            TOK_RETURN => {
                unsafe { self.advance() };
                let e = unsafe { self.parse_ternary() }?;
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
                // MILESTONE 108: `IDENT "[" cond_expr "]" "=" cond_expr
                // ";"` -- a real array-element write, checked BEFORE the
                // label/plain-assignment disambiguation just below (a
                // real array name can never legally be followed by `:`
                // in this subset anyway, so the ordering is safe either
                // way, but checking the newer, narrower case first keeps
                // this dispatch reading top-to-bottom in the order each
                // milestone added it). Only plain `=` is supported here
                // -- `arr[i] += 1;` and friends are real, disclosed
                // future work (Milestone 84's nine compound-assignment
                // operators never reached array targets), not attempted
                // in this first slice.
                if unsafe { self.peek() }.kind == TOK_LBRACKET {
                    unsafe { self.advance() }; // consume '['
                    let idx = unsafe { self.parse_ternary() }?;
                    unsafe { self.expect(TOK_RBRACKET) }?;
                    unsafe { self.expect(TOK_ASSIGN) }?;
                    let rhs = unsafe { self.parse_ternary() }?;
                    unsafe { self.expect(TOK_SEMI) }?;
                    let node = unsafe { alloc_stmt() };
                    if node == 0 {
                        return Err(ParseError::OutOfMemory);
                    }
                    unsafe {
                        core::ptr::write(
                            node as *mut StmtNode,
                            StmtNode { kind: STMT_ASSIGN_INDEX, ident_off: name.ident_off, ident_len: name.ident_len, expr: rhs, next: 0, then_body: idx, else_body: 0 },
                        )
                    };
                    return Ok(node);
                }
                // MILESTONE 92: `IDENT :` in statement position is a
                // label, not an assignment -- one token of lookahead
                // disambiguates. `L:` is a standalone STMT_LABEL node;
                // whatever follows is the next list element.
                if unsafe { self.peek() }.kind == TOK_COLON {
                    unsafe { self.advance() }; // consume ':'
                    let node = unsafe { alloc_stmt() };
                    if node == 0 {
                        return Err(ParseError::OutOfMemory);
                    }
                    unsafe {
                        core::ptr::write(
                            node as *mut StmtNode,
                            StmtNode { kind: STMT_LABEL, ident_off: name.ident_off, ident_len: name.ident_len, expr: 0, next: 0, then_body: 0, else_body: 0 },
                        )
                    };
                    return Ok(node);
                }
                unsafe { self.finish_ident_assign(name, true) }
            }
            // MILESTONE 109: `"*" IDENT "=" cond_expr ";"` -- a real
            // pointer dereference WRITE (the lvalue form of `*p`),
            // dispatched at the very start of a statement exactly like
            // Milestone 108's own `IDENT "["` array-write arm above --
            // a statement STARTING with `*` can only ever be this one
            // shape in this subset's grammar (there is no bare
            // expression-statement here at all -- see TOK_IDENT's own
            // arm above, which requires an `=`/compound-assign or falls
            // through to a real ParseError -- so an expression-only
            // dereference like a bare `*p;` is not legal statement
            // grammar either, consistent with that same restriction).
            // The pointer operand is deliberately restricted to a plain
            // IDENT (not a general unary/factor) -- the same narrow
            // "one real slot, not the whole expression grammar" scope
            // parse_array_decl()'s own array-name restriction already
            // established for STMT_ASSIGN_INDEX; since this subset has
            // no pointer arithmetic, `*p` is the ENTIRE real
            // dereference-target grammar anyway (there is no `*(p+1)`
            // this scope could even reach). This dispatch is
            // unambiguous with parse_unary()'s own `*` (dereference-as-
            // rvalue) handling below: parse_stmt()'s own top-level match
            // only ever sees the FIRST token of a brand-new statement,
            // while parse_unary()'s `*` case only ever runs from
            // somewhere already inside expression parsing (an
            // assignment's RHS, a call argument, a `return` expression,
            // etc.) -- the two can never both fire on the same token.
            TOK_STAR => {
                unsafe { self.advance() }; // consume '*'
                let name = unsafe { self.expect(TOK_IDENT) }?;
                unsafe { self.expect(TOK_ASSIGN) }?;
                let rhs = unsafe { self.parse_ternary() }?;
                unsafe { self.expect(TOK_SEMI) }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode { kind: STMT_ASSIGN_DEREF, ident_off: name.ident_off, ident_len: name.ident_len, expr: rhs, next: 0, then_body: 0, else_body: 0 },
                    )
                };
                Ok(node)
            }
            // MILESTONE 92: `goto LABEL ;`.
            TOK_GOTO => {
                unsafe { self.advance() };
                let target = unsafe { self.expect(TOK_IDENT) }?;
                unsafe { self.expect(TOK_SEMI) }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode { kind: STMT_GOTO, ident_off: target.ident_off, ident_len: target.ident_len, expr: 0, next: 0, then_body: 0, else_body: 0 },
                    )
                };
                Ok(node)
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
                } else if unsafe { self.peek() }.kind == TOK_INT {
                    // MILESTONE 88: `for (int i = 0; ...)` -- a combined
                    // decl-init as the init clause, same desugar
                    // (`int i;` then `i = 0;`) as at statement level.
                    unsafe { self.advance() };
                    let n = unsafe { self.expect(TOK_IDENT) }?;
                    unsafe { self.make_decl(n, false, false) }?
                } else if unsafe { self.peek() }.kind == TOK_CHAR {
                    // MILESTONE 102: `for (char i = 0; ...)`, same
                    // desugar with `is_char = true`.
                    unsafe { self.advance() };
                    let n = unsafe { self.expect(TOK_IDENT) }?;
                    unsafe { self.make_decl(n, false, true) }?
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
                    unsafe { self.parse_ternary() }?
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
                    // MILESTONE 88: `init` may be a two-node chain now
                    // (`int i;` -> `i = 0;`), so link the while after its
                    // real tail, not its head.
                    let mut t = init;
                    while unsafe { (*(t as *const StmtNode)).next } != 0 {
                        t = unsafe { (*(t as *const StmtNode)).next };
                    }
                    unsafe { (*(t as *mut StmtNode)).next = while_node };
                    Ok(init)
                }
            }
            TOK_IF => {
                unsafe { self.advance() };
                unsafe { self.expect(TOK_LPAREN) }?;
                let cond = unsafe { self.parse_ternary() }?;
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
                let cond = unsafe { self.parse_ternary() }?;
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
            // MILESTONE 99: `do "{" stmt* "}" while "(" ternary ")" ";"`.
            // Same `{ ... }` body shape as `while`, but the condition
            // (and its `)` and the trailing `;`) come AFTER the body.
            // Lowered to its own STMT_DOWHILE (expr = cond, then_body =
            // body) -- a distinct kind, not a flag on STMT_WHILE, so
            // STMT_WHILE's own codegen is untouched.
            TOK_DO => {
                unsafe { self.advance() };
                unsafe { self.expect(TOK_LBRACE) }?;
                let body = unsafe { self.parse_stmt_list_until_rbrace() }?;
                unsafe { self.expect(TOK_WHILE) }?;
                unsafe { self.expect(TOK_LPAREN) }?;
                let cond = unsafe { self.parse_ternary() }?;
                unsafe { self.expect(TOK_RPAREN) }?;
                unsafe { self.expect(TOK_SEMI) }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode {
                            kind: STMT_DOWHILE,
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
            // MILESTONE 91: `switch "(" ternary ")" "{"
            //   ("case" INTLIT ":" stmt*)* ("default" ":" stmt*)? "}"`.
            // Builds a STMT_SWITCH whose `then_body` is a chain of
            // STMT_CASE nodes (each carrying an EXPR_INTLIT label value
            // and its own stmt list) and whose `else_body` is the
            // `default` list (0 if absent). Fall-through, `break`, and
            // the "discriminant evaluated once" guarantee are all
            // codegen-side (STMT_SWITCH in gen_stmt_list).
            TOK_SWITCH => {
                unsafe { self.advance() };
                unsafe { self.expect(TOK_LPAREN) }?;
                let disc = unsafe { self.parse_ternary() }?;
                unsafe { self.expect(TOK_RPAREN) }?;
                unsafe { self.expect(TOK_LBRACE) }?;
                let mut cases_head: u64 = 0;
                let mut cases_tail: u64 = 0;
                let mut default_body: u64 = 0;
                loop {
                    let k = unsafe { self.peek() }.kind;
                    if k == TOK_RBRACE {
                        break;
                    }
                    if k == TOK_CASE {
                        unsafe { self.advance() };
                        let val_tok = unsafe { self.expect(TOK_INTLIT) }?;
                        unsafe { self.expect(TOK_COLON) }?;
                        let label = unsafe { alloc_expr() };
                        if label == 0 {
                            return Err(ParseError::OutOfMemory);
                        }
                        unsafe {
                            core::ptr::write(
                                label as *mut ExprNode,
                                ExprNode { kind: EXPR_INTLIT, int_val: val_tok.int_val, ident_off: 0, ident_len: 0, op: 0, left: 0, right: 0, call_args_ptr: 0, call_argc: 0 },
                            )
                        };
                        let body = unsafe { self.parse_case_body() }?;
                        let cnode = unsafe { alloc_stmt() };
                        if cnode == 0 {
                            return Err(ParseError::OutOfMemory);
                        }
                        unsafe {
                            core::ptr::write(
                                cnode as *mut StmtNode,
                                StmtNode { kind: STMT_CASE, ident_off: 0, ident_len: 0, expr: label, next: 0, then_body: body, else_body: 0 },
                            )
                        };
                        if cases_head == 0 {
                            cases_head = cnode;
                        } else {
                            unsafe { (*(cases_tail as *mut StmtNode)).next = cnode };
                        }
                        cases_tail = cnode;
                    } else if k == TOK_DEFAULT {
                        unsafe { self.advance() };
                        unsafe { self.expect(TOK_COLON) }?;
                        default_body = unsafe { self.parse_case_body() }?;
                    } else {
                        return Err(ParseError::UnexpectedToken(self.pos));
                    }
                }
                unsafe { self.expect(TOK_RBRACE) }?;
                let node = unsafe { alloc_stmt() };
                if node == 0 {
                    return Err(ParseError::OutOfMemory);
                }
                unsafe {
                    core::ptr::write(
                        node as *mut StmtNode,
                        StmtNode { kind: STMT_SWITCH, ident_off: 0, ident_len: 0, expr: disc, next: 0, then_body: cases_head, else_body: default_body },
                    )
                };
                Ok(node)
            }
            _ => Err(ParseError::UnexpectedToken(self.pos)),
        }
    }

    /// MILESTONE 91: parse statements until (but not consuming) the next
    /// `case`, `default`, or `}` -- the body of one `case`/`default`
    /// label inside a `switch`. Same chain-tail bookkeeping as
    /// parse_stmt_list_until_rbrace().
    unsafe fn parse_case_body(&mut self) -> Result<u64, ParseError> {
        let mut head: u64 = 0;
        let mut tail: u64 = 0;
        loop {
            let k = unsafe { self.peek() }.kind;
            if k == TOK_RBRACE || k == TOK_CASE || k == TOK_DEFAULT {
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
            tail = stmt;
            while unsafe { (*(tail as *const StmtNode)).next } != 0 {
                tail = unsafe { (*(tail as *const StmtNode)).next };
            }
        }
        Ok(head)
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
            // MILESTONE 103: a parameter's own leading type is now
            // `TOK_INT` OR `TOK_CHAR` (real symmetry with make_decl()'s
            // own `int`/`char` choice at declaration sites) -- `expect()`
            // still real-errors on anything else.
            let is_char = if unsafe { self.peek() }.kind == TOK_CHAR {
                unsafe { self.advance() };
                true
            } else {
                unsafe { self.expect(TOK_INT) }?;
                false
            };
            // MILESTONE 110: an optional `*` right after `int` marks a
            // POINTER parameter -- `int *p`. This needs no new
            // `ParamInfo` field and no codegen change of any kind: a
            // pointer parameter is register/stack-passed exactly like an
            // ordinary scalar `int` (Milestone 109's own "a pointer's
            // runtime value is just another 8-byte value" design point,
            // now reaching one more real call site) --
            // `collect_vars_for_function()`'s own Milestone 110 doc
            // comment has the full reasoning for why its resulting
            // `VarSlot` is `is_array: 0` either way. The token is simply
            // consumed; nothing downstream needs to know it was there.
            //
            // MILESTONE 113: the same optional `*`, now ALSO accepted
            // right after `char` -- `char *p`, the exact grammar position
            // Milestone 112's own closing disclosure named as needing "a
            // real, declared-parameter-type bit through `ParamInfo`,
            // which does not exist today." Unlike the `int *` case above,
            // this one DOES need a new `ParamInfo` field
            // (`is_char_ptr`, just added): a `char *` parameter's own
            // resulting `VarSlot` must end up with `is_char == 0` (its
            // OWN value, the address, is never narrowed on read) and
            // `is_char_ptr == 1` (a dereference `*p` through it should
            // narrow) -- genuinely different `VarSlot` shape than a plain
            // `char` parameter (`is_char == 1`, `is_char_ptr == 0`), so
            // `is_char` is deliberately reset to `false` here the moment
            // a `*` is seen after `char`, rather than left `true` and
            // combined with a second flag.
            let mut is_char = is_char;
            let mut is_char_ptr = false;
            if unsafe { self.peek() }.kind == TOK_STAR {
                unsafe { self.advance() };
                if is_char {
                    is_char = false;
                    is_char_ptr = true;
                }
            }
            let name = unsafe { self.expect(TOK_IDENT) }?;
            if count as u64 >= MAX_PARAMS {
                return Err(ParseError::TooManyParams);
            }
            unsafe { param_write(params_ptr, count as u64, ParamInfo { name_off: name.ident_off, name_len: name.ident_len, is_char: if is_char { 1 } else { 0 }, is_char_ptr: if is_char_ptr { 1 } else { 0 } }) };
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
        // MILESTONE 103: the return-type keyword is now `TOK_INT` OR
        // `TOK_CHAR` -- see ParamInfo/FuncDef's own doc comments above
        // for how `is_char_return` reaches codegen from here.
        let is_char_return = if unsafe { self.peek() }.kind == TOK_CHAR {
            unsafe { self.advance() };
            // MILESTONE 113: an optional `*` right after `char` (before
            // the function's own name) marks a `char *` return type --
            // `char *f(...)`, the exact grammar position Milestone 110
            // already opened for `int *` and Milestone 112's own closing
            // disclosure named as still missing for `char *`. Exactly the
            // same reasoning as the `int *` branch below applies here:
            // `STMT_RETURN`'s codegen only narrows when `is_char_return`
            // is set, and a POINTER return -- whether to `int` or to
            // `char` -- must never be narrowed (RAX already holds the
            // correct raw 8-byte address; narrowing it would corrupt the
            // address itself, not just the value it points to), so
            // leaving `is_char_return` false when a `*` follows `char` is
            // the complete, correct implementation. No new `FuncDef`
            // field needed: `is_char_return` already meant "narrow this
            // function's return value," which was already exactly the
            // right question to ask regardless of whether the return
            // type is a scalar `int`, a scalar `char`, an `int *`, or now
            // a `char *`.
            if unsafe { self.peek() }.kind == TOK_STAR {
                unsafe { self.advance() };
                false
            } else {
                true
            }
        } else {
            unsafe { self.expect(TOK_INT) }?;
            // MILESTONE 110: an optional `*` right after `int` (before
            // the function's own name) marks a POINTER return type --
            // `int *f(...)`. This needs no new `FuncDef` field and no
            // codegen change at all: `STMT_RETURN`'s own codegen already
            // just evaluates the return expression into RAX and, ONLY
            // when `is_char_return` is set, narrows it -- a pointer
            // return needs exactly the same "no narrowing, RAX already
            // holds the right 8-byte value" treatment an ordinary `int`
            // return already gets, so leaving `is_char_return` false
            // here (this branch's existing value) after consuming the
            // token is the complete, correct implementation -- accepting
            // the grammar is the entire feature.
            if unsafe { self.peek() }.kind == TOK_STAR {
                unsafe { self.advance() };
            }
            false
        };
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
                    is_char_return: if is_char_return { 1 } else { 0 },
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
    /// MILESTONE 92: a `goto` whose target label is never defined
    /// anywhere in the same function. Carries the label name's source
    /// byte offset, the same "real Err path, carries a source offset"
    /// shape UndeclaredVariable/UndeclaredFunction already use --
    /// exercised for real by CASE 59.
    UndefinedLabel(u64),
    /// MILESTONE 111: a bare array name used as a plain ASSIGN target
    /// (`arr = expr;`, including a compound-assignment operator like
    /// `arr += expr;` -- parse_assign_stmt() desugars those into an
    /// ordinary STMT_ASSIGN before codegen ever sees them, so this one
    /// check covers both spellings). Carries the array name's own source
    /// byte offset, the same "real Err path, carries a source offset"
    /// shape UndeclaredVariable/UndefinedLabel already use. Closes the
    /// exact read/write asymmetry Milestone 110's own closing disclosure
    /// named ("reading `arr` now decays to its address ... but writing
    /// ... still overwrites element 0's raw VALUE unchanged"): real C
    /// has no answer to "assign to an array" either -- an array is not a
    /// modifiable lvalue there -- so this milestone makes the same call
    /// rather than inventing whole-array-copy semantics this subset's
    /// STMT_ASSIGN codegen was never built to perform.
    ArrayNotAssignable(u64),
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

/// MILESTONE 92: `goto`/label bookkeeping, per function, in module
/// `static mut` arenas (same reasoning as Milestone 87's LOOP_BRK --
/// cc.elf's heap is small and codegen is single-pass, non-reentrant).
/// `LBL_*` records every label seen so far (name -> the buf offset it
/// sits at). `PEND_*` records every `goto` whose target label has not
/// been seen yet (name -> the jmp-placeholder field to patch once it
/// is). `goto_reset()` clears both before each function; an entry left
/// in `PEND_*` after the function body is an undefined label.
const MAX_GOTO: usize = 32;
static mut GOTO_LBL_OFF: [u32; MAX_GOTO] = [0; MAX_GOTO];
static mut GOTO_LBL_LEN: [u8; MAX_GOTO] = [0; MAX_GOTO];
static mut GOTO_LBL_POS: [u64; MAX_GOTO] = [0; MAX_GOTO];
static mut GOTO_LBL_N: u64 = 0;
static mut GOTO_PEND_OFF: [u32; MAX_GOTO] = [0; MAX_GOTO];
static mut GOTO_PEND_LEN: [u8; MAX_GOTO] = [0; MAX_GOTO];
static mut GOTO_PEND_FIELD: [u64; MAX_GOTO] = [0; MAX_GOTO];
static mut GOTO_PEND_N: u64 = 0;

unsafe fn goto_reset() {
    unsafe {
        core::ptr::write(core::ptr::addr_of_mut!(GOTO_LBL_N), 0);
        core::ptr::write(core::ptr::addr_of_mut!(GOTO_PEND_N), 0);
    }
}
unsafe fn goto_lbl_n() -> u64 { unsafe { core::ptr::read(core::ptr::addr_of!(GOTO_LBL_N)) } }
unsafe fn goto_pend_n() -> u64 { unsafe { core::ptr::read(core::ptr::addr_of!(GOTO_PEND_N)) } }

/// Compare two label-name spans, both `(offset, len)` into the same
/// source buffer.
unsafe fn label_eq(src_ptr: u64, a_off: u32, a_len: u8, b_off: u32, b_len: u8) -> bool {
    if a_len != b_len {
        return false;
    }
    let mut i: u64 = 0;
    while i < a_len as u64 {
        let ca = unsafe { core::ptr::read((src_ptr + a_off as u64 + i) as *const u8) };
        let cb = unsafe { core::ptr::read((src_ptr + b_off as u64 + i) as *const u8) };
        if ca != cb {
            return false;
        }
        i += 1;
    }
    true
}

/// A `STMT_LABEL` reached at buffer offset `pos`: record it, then
/// resolve (and drop) every pending `goto` that names it -- a forward
/// jump patched to land exactly here.
unsafe fn goto_define_label(buf: &mut CodeBuf, src_ptr: u64, off: u32, len: u8, pos: u64) -> Result<(), CodeGenError> {
    let n = unsafe { goto_lbl_n() };
    if n >= MAX_GOTO as u64 {
        return Err(CodeGenError::TooManyLoopJumps);
    }
    unsafe {
        core::ptr::write((core::ptr::addr_of_mut!(GOTO_LBL_OFF) as u64 + n * 4) as *mut u32, off);
        core::ptr::write((core::ptr::addr_of_mut!(GOTO_LBL_LEN) as u64 + n) as *mut u8, len);
        core::ptr::write((core::ptr::addr_of_mut!(GOTO_LBL_POS) as u64 + n * 8) as *mut u64, pos);
        core::ptr::write(core::ptr::addr_of_mut!(GOTO_LBL_N), n + 1);
    }
    // resolve pending forward gotos for this name, compacting the list.
    let mut i: u64 = 0;
    let mut keep: u64 = 0;
    let pn = unsafe { goto_pend_n() };
    while i < pn {
        let poff = unsafe { core::ptr::read((core::ptr::addr_of!(GOTO_PEND_OFF) as u64 + i * 4) as *const u32) };
        let plen = unsafe { core::ptr::read((core::ptr::addr_of!(GOTO_PEND_LEN) as u64 + i) as *const u8) };
        let pfield = unsafe { core::ptr::read((core::ptr::addr_of!(GOTO_PEND_FIELD) as u64 + i * 8) as *const u64) };
        if unsafe { label_eq(src_ptr, poff, plen, off, len) } {
            unsafe { buf.patch_rel32(pfield) }; // patches to current buf.len == pos
        } else {
            unsafe {
                core::ptr::write((core::ptr::addr_of_mut!(GOTO_PEND_OFF) as u64 + keep * 4) as *mut u32, poff);
                core::ptr::write((core::ptr::addr_of_mut!(GOTO_PEND_LEN) as u64 + keep) as *mut u8, plen);
                core::ptr::write((core::ptr::addr_of_mut!(GOTO_PEND_FIELD) as u64 + keep * 8) as *mut u64, pfield);
            }
            keep += 1;
        }
        i += 1;
    }
    unsafe { core::ptr::write(core::ptr::addr_of_mut!(GOTO_PEND_N), keep) };
    Ok(())
}

/// A `STMT_GOTO`: if the target label is already defined, a real
/// backward `jmp`; otherwise a placeholder recorded in `PEND_*` for
/// `goto_define_label` (or the end-of-function check) to resolve.
unsafe fn goto_emit(buf: &mut CodeBuf, src_ptr: u64, off: u32, len: u8) -> Result<(), CodeGenError> {
    let ln = unsafe { goto_lbl_n() };
    let mut i: u64 = 0;
    while i < ln {
        let loff = unsafe { core::ptr::read((core::ptr::addr_of!(GOTO_LBL_OFF) as u64 + i * 4) as *const u32) };
        let llen = unsafe { core::ptr::read((core::ptr::addr_of!(GOTO_LBL_LEN) as u64 + i) as *const u8) };
        if unsafe { label_eq(src_ptr, off, len, loff, llen) } {
            let target = unsafe { core::ptr::read((core::ptr::addr_of!(GOTO_LBL_POS) as u64 + i * 8) as *const u64) };
            unsafe { buf.emit_jmp_back(target) };
            return Ok(());
        }
        i += 1;
    }
    let pn = unsafe { goto_pend_n() };
    if pn >= MAX_GOTO as u64 {
        return Err(CodeGenError::TooManyLoopJumps);
    }
    let field = unsafe { buf.emit_jmp_placeholder() };
    unsafe {
        core::ptr::write((core::ptr::addr_of_mut!(GOTO_PEND_OFF) as u64 + pn * 4) as *mut u32, off);
        core::ptr::write((core::ptr::addr_of_mut!(GOTO_PEND_LEN) as u64 + pn) as *mut u8, len);
        core::ptr::write((core::ptr::addr_of_mut!(GOTO_PEND_FIELD) as u64 + pn * 8) as *mut u64, field);
        core::ptr::write(core::ptr::addr_of_mut!(GOTO_PEND_N), pn + 1);
    }
    Ok(())
}

const MAX_VARS: u64 = 8;

#[derive(Clone, Copy)]
#[repr(C)]
struct VarSlot {
    name_off: u32,
    name_len: u8,
    stack_off: i32, // bytes from rbp, always negative
    // MILESTONE 102: 1 if this slot was declared `char`, 0 for `int`
    // (and, unconditionally, for every parameter -- see
    // collect_vars_for_function() below). The slot itself is still a
    // full 8 bytes either way (collect_vars_rec()'s own layout is
    // completely unchanged); only gen_expr()'s EXPR_IDENT read of this
    // slot differs by this flag -- see find_var()'s and
    // emit_movsx_rax_rbp_off_byte()'s own doc comments below for why a
    // real 1-byte-narrower load, not a narrower slot, is the whole
    // feature.
    is_char: u8,
    // MILESTONE 110: 1 for the one real NAMED entry an array declarator
    // produces (collect_vars_rec()'s own Milestone 108 branch, element
    // 0 -- the array's own base slot), 0 for every scalar/pointer
    // variable, every parameter, and every array's own BLANK filler
    // entry (elements 1..N-1, already unmatchable by name so this flag
    // never actually gets read for them). This is what makes real
    // array-to-pointer DECAY possible: gen_expr()'s own EXPR_IDENT arm
    // now checks this flag to decide whether reading a bare name means
    // "load the 8-byte VALUE stored there" (an ordinary scalar or
    // pointer) or "compute the 8-byte ADDRESS of the slot itself" (an
    // array, which decays to a pointer to its own element 0 in every
    // expression context real C allows this subset to reach) -- see
    // that arm's own doc comment for the full derivation. Also closes,
    // as a real side effect rather than a second deliberate change, the
    // "a bare array name silently reads its own element-0 VALUE instead
    // of decaying" simplification Milestone 108's own closing disclosure
    // named honestly at the time.
    is_array: u8,
    // MILESTONE 112: 1 exactly for a `char *p;` pointer DECLARATOR's own
    // named slot (`parse_pointer_decl()`'s new `is_char_pointee`
    // parameter, threaded through `s.expr` -- otherwise dead for
    // STMT_DECL, the same field-reuse-over-new-field discipline
    // `then_body`/`else_body` already established for `is_char`/array
    // element count), 0 for everything else -- an ordinary `int`, a
    // `char` SCALAR, an `int *` pointer, an array, and every parameter
    // (this milestone deliberately does not extend `char *` across a
    // function-call boundary -- see this file's own top Milestone 112
    // doc comment for the scope reasoning). Deliberately a SEPARATE
    // property from `is_char` above: `is_char` means "THIS slot's own
    // stored VALUE narrows to a signed byte on read" (true for a `char`
    // scalar, always false for ANY pointer, since a pointer's own value
    // is an address, never narrowed); `is_char_ptr` means "a dereference
    // THROUGH this pointer should narrow" -- a `char *p;`'s own slot
    // still has `is_char == 0` (reading `p` itself, e.g. `q = p;`, must
    // still load the full 8-byte address unnarrowed) and `is_char_ptr ==
    // 1` (reading `*p` should not). Consulted by exactly one site,
    // `EXPR_UNARY`'s `OP_DEREF` arm -- see that arm's own doc comment for
    // the real, disclosed restriction on which `*expr` shapes this
    // milestone can actually resolve it for.
    is_char_ptr: u8,
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
            // MILESTONE 108: `s.else_body` is 0 for an ordinary scalar
            // declarator (every STMT_DECL before this milestone, and
            // every `char` one still today -- TOK_CHAR's own arm never
            // calls parse_array_decl()) and a real positive element
            // count for an array declarator (parse_array_decl(), new).
            let elem_count: u64 = if s.else_body == 0 { 1 } else { s.else_body };
            if *n + elem_count > MAX_VARS {
                return Err(CodeGenError::TooManyVars);
            }
            let off = -8 * (*n as i32 + 1);
            // MILESTONE 102: `s.then_body` is 0/1, written by make_decl()
            // (see that function's own doc comment for why `then_body`,
            // otherwise dead for STMT_DECL, carries this).
            // MILESTONE 110: `is_array` is real (1) exactly when this
            // declarator reserved more than one element (`elem_count >
            // 1`, i.e. an array declarator, never a scalar one -- a
            // scalar `int`/`char`/pointer declaration always has
            // `elem_count == 1`) -- computed straight from the same
            // `elem_count` this branch already derived above, not a
            // second independent check.
            // MILESTONE 112: `is_char_ptr` is `s.expr` (0/1, otherwise
            // dead for STMT_DECL -- see VarSlot's own `is_char_ptr` doc
            // comment for why this is a genuinely separate property from
            // `is_char` just before it, not a duplicate of it). Only
            // `parse_pointer_decl()`'s own new `is_char_pointee` branch
            // ever writes a nonzero `s.expr` on a STMT_DECL -- every other
            // producer (`make_decl()` for scalars, `parse_array_decl()`
            // for arrays, and `parse_pointer_decl()`'s own pre-existing
            // `int *` path) still writes `expr: 0`, so this cast is safe
            // and this line changes behavior for NOTHING declared before
            // this milestone.
            unsafe { var_write(vars_ptr, *n, VarSlot { name_off: s.ident_off, name_len: s.ident_len, stack_off: off, is_char: s.then_body as u8, is_array: if elem_count > 1 { 1 } else { 0 }, is_char_ptr: s.expr as u8 }) };
            // MILESTONE 108: an array of `elem_count` > 1 reserves
            // `elem_count` contiguous 8-byte stack slots (element `i`
            // lives at `off - 8*i`, i.e. progressively MORE negative as
            // `i` grows -- the exact same "later slot, more negative
            // offset" direction every OTHER slot in this flat allocator
            // already grows in, just walked `elem_count` steps at once
            // instead of one), but only the FIRST of them (element 0,
            // this array's own base address, written just above) is a
            // real NAMED entry find_var() can ever match. Elements
            // 1..elem_count-1 get real, explicitly-written BLANK
            // entries (name_len=0, unmatchable by any real identifier
            // this lexer could ever produce) rather than being left as
            // uninitialized malloc()ed memory -- find_var()'s own linear
            // scan reads every entry up to `nvars` unconditionally, and
            // reading a real uninitialized VarSlot there would be
            // real, live undefined behavior, not merely untidy. Writing
            // real blank filler entries, one per remaining element, is
            // what keeps `*n` (== nvars, this function's own return
            // value) an exactly accurate "how many 8-byte slots does
            // this function's frame need" count -- the same count
            // gen_function()/gen_program()'s own `frame_bytes =
            // (nvars + 1) * 8` already uses UNCHANGED by this milestone
            // to size `sub rsp, imm8` -- so a function that declares an
            // array gets a correctly-sized real stack frame with ZERO
            // changes to either of those two frame-sizing call sites.
            let mut k: u64 = 1;
            while k < elem_count {
                let elem_off = off - 8 * (k as i32);
                unsafe { var_write(vars_ptr, *n + k, VarSlot { name_off: 0, name_len: 0, stack_off: elem_off, is_char: 0, is_array: 0, is_char_ptr: 0 }) };
                k += 1;
            }
            *n += elem_count;
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
        } else if s.kind == STMT_DOWHILE {
            // MILESTONE 99: same recursive-DECL walk into the loop body
            // as STMT_WHILE (the condition is an expression, no decls).
            unsafe { collect_vars_rec(s.then_body, vars_ptr, n) }?;
        } else if s.kind == STMT_SWITCH {
            // MILESTONE 91: `then_body` is a chain of STMT_CASE nodes
            // (handled just below), `else_body` is the `default` list.
            unsafe { collect_vars_rec(s.then_body, vars_ptr, n) }?;
            unsafe { collect_vars_rec(s.else_body, vars_ptr, n) }?;
        } else if s.kind == STMT_CASE {
            // MILESTONE 91: one `case`'s own stmt list.
            unsafe { collect_vars_rec(s.then_body, vars_ptr, n) }?;
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
        // MILESTONE 103: a parameter's own `is_char` (Milestone 102's
        // convention, now reaching parse_params() too) carries straight
        // through into its VarSlot -- a `char` parameter narrows on
        // read exactly like a `char` local. Before this milestone this
        // was unconditionally 0 (char wasn't a parameter type yet).
        // MILESTONE 110: a parameter's own `is_array` is always 0, even
        // for a pointer parameter (`int *p`) -- a POINTER parameter's
        // own slot holds a VALUE (whatever address the caller passed,
        // e.g. a just-decayed array argument), not an in-place array
        // base the way a real local array declarator's own element-0
        // slot does; EXPR_INDEX/STMT_ASSIGN_INDEX's own Milestone 110
        // `is_array`-gated codegen relies on exactly this distinction to
        // correctly subscript THROUGH a pointer parameter (read its
        // VALUE first) rather than treating the parameter's own stack
        // slot as if it WERE the array (which would be wrong -- see
        // those two codegen arms' own doc comments for the full
        // derivation).
        // MILESTONE 112 (superseded by 113 below): `is_char_ptr` was
        // always 0 for a parameter -- Milestone 112 deliberately did not
        // extend `char *` across a function-call boundary (no `char *`
        // parameters/returns), the same real, disclosed scope cut
        // Milestone 109 itself used for `int *` before Milestone 110
        // later closed it.
        // MILESTONE 113: `is_char_ptr` now carries straight through from
        // `p.is_char_ptr` (the real declared-parameter-type bit
        // `parse_params()` now records -- see `ParamInfo::is_char_ptr`'s
        // own doc comment), the exact "thread a real declared-parameter-
        // type bit through `ParamInfo`" step Milestone 112's own closing
        // disclosure (quoted above) named as the missing piece. A
        // `char *p` parameter's own `VarSlot` now correctly ends up with
        // `is_char: 0, is_char_ptr: 1` (mirroring the local-variable
        // shape Milestone 112 already established), so `EXPR_UNARY`'s
        // `OP_DEREF` arm -- which already reads this field generically
        // through `find_var()`, with zero awareness of whether a given
        // slot came from a parameter or a local declaration -- narrows a
        // parameter's own `*p` correctly with NO codegen change at that
        // call site at all, the same "accepting the grammar plus
        // threading the one flag IS the complete feature" shape Milestone
        // 110 already established for `int *` parameters.
        unsafe { var_write(vars_ptr, n, VarSlot { name_off: p.name_off, name_len: p.name_len, stack_off: off, is_char: p.is_char, is_array: 0, is_char_ptr: p.is_char_ptr }) };
        n += 1;
        pi += 1;
    }
    unsafe { collect_vars_rec(f.body, vars_ptr, &mut n) }?;
    Ok(n)
}

/// MILESTONE 102: now returns `(stack_off, is_char)` instead of a bare
/// offset -- gen_expr()'s own EXPR_IDENT read (the only site that needs
/// the type; STMT_ASSIGN's store is always the same full 8 bytes
/// regardless of type, see emit_mov_rbp_off_rax()'s call site) uses the
/// second field to choose a real narrower load. Both existing call
/// sites updated; STMT_ASSIGN's just ignores the new field.
/// MILESTONE 110: now returns `(stack_off, is_char, is_array)` -- the
/// third field is what makes real array-to-pointer decay (EXPR_IDENT)
/// and real pointer-vs-array subscripting (EXPR_INDEX/
/// STMT_ASSIGN_INDEX) possible; every call site updated, the ones that
/// don't need it (STMT_ASSIGN's store, EXPR_UNARY's OP_ADDR) just ignore
/// it, the same "ignore the field you don't need" precedent
/// Milestone 102's own `is_char` addition already established for
/// STMT_ASSIGN.
/// MILESTONE 112: now returns `(stack_off, is_char, is_array,
/// is_char_ptr)` -- the fourth field is what makes a real `*p`
/// dereference-READ narrow to a signed byte when `p` was declared
/// `char *` (see VarSlot's own `is_char_ptr` doc comment for why this is
/// a genuinely separate property from `is_char`, not a duplicate); all
/// six of this function's PRE-EXISTING call sites are updated for the
/// new tuple arity, every one of which just ignores the new field
/// (`EXPR_IDENT`,
/// `EXPR_INDEX`, `EXPR_UNARY`'s own `OP_ADDR` arm, `STMT_ASSIGN`,
/// `STMT_ASSIGN_INDEX`, `STMT_ASSIGN_DEREF` -- none of these needs
/// pointee-narrowing information, the same "ignore the field you don't
/// need" precedent every prior `find_var()` field addition already
/// established. `EXPR_UNARY`'s own `OP_DEREF` arm is a genuinely NEW,
/// seventh call site this milestone adds -- before this milestone that
/// arm never called `find_var()` at all, since `int *` dereference never
/// needed to distinguish anything about the pointee.
unsafe fn find_var(src_ptr: u64, vars_ptr: u64, nvars: u64, ident_off: u64, ident_len: u64) -> Option<(i32, u8, u8, u8)> {
    let mut i: u64 = 0;
    while i < nvars {
        let v = unsafe { var_read(vars_ptr, i) };
        if unsafe { same_span(src_ptr, v.name_off as u64, v.name_len as u64, ident_off, ident_len) } {
            return Some((v.stack_off, v.is_char, v.is_array, v.is_char_ptr));
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
    /// MILESTONE 102: `movsx rax, byte [rbp+disp8]` -- REX.W + `0F BE`
    /// (MOVSX r64, r/m8 per the Intel SDM's own encoding table, the
    /// same opcode family emit_movzx_eax_al() below already uses for
    /// its own 8-bit-source form, `0F B6`, just the sign- instead of
    /// zero-extending sibling and reading from memory instead of AL)
    /// with the exact same ModRM byte (`0x45` -- mod=01/reg=rax(000)/
    /// rm=rbp(101), disp8 follows) emit_mov_rax_rbp_off() above already
    /// uses for its full-width sibling. This is the ONLY place a `char`
    /// variable's real narrowness shows up in codegen: the stack slot
    /// itself stays a full 8 bytes (collect_vars_rec()'s layout is
    /// completely unchanged by this milestone), so a `char` is always
    /// STORED as a full 8-byte value (STMT_ASSIGN's
    /// emit_mov_rbp_off_rax() is untouched) -- reading back only its
    /// low byte, sign-extended, is what makes `char c = 300;` truncate
    /// to `44`, and `char c = 200;` truncate-and-sign-extend to `-56`
    /// (this subset's `char` is signed, matching the default `char`
    /// signedness of a real x86_64 SysV C compiler), on the very next
    /// read -- with no separate "truncate on store" codegen needed at
    /// all, since both operations read/write the identical low byte of
    /// the same little-endian slot.
    unsafe fn emit_movsx_rax_rbp_off_byte(&mut self, off: i32) {
        unsafe { self.push_bytes(&[0x48, 0x0F, 0xBE, 0x45, off as i8 as u8]) };
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
    /// MILESTONE 91: `cmp rax, imm32` -- `48 3D <imm32>` (the special
    /// short-form `cmp RAX/EAX, imm`, no ModRM). The imm32 is
    /// SIGN-EXTENDED to 64 bits, which is exactly right for this
    /// subset's signed-`int` case labels. Used only by STMT_SWITCH's
    /// dispatch to test the spilled discriminant against each `case`
    /// value without needing a scratch register.
    unsafe fn emit_cmp_rax_imm32(&mut self, v: u32) {
        unsafe { self.push_bytes(&[0x48, 0x3D]) };
        unsafe { self.push_bytes(&v.to_le_bytes()) };
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
        unsafe { self.push_bytes(&[0x48, 0xF7, 0xF9]) }; // idiv rcx (quotient -> rax, remainder -> rdx)
    }
    /// MILESTONE 89: `mov rax, rdx` -- the one new encoding `%` needs.
    /// `idiv rcx` already leaves the remainder in RDX; `%` just moves
    /// it into RAX to satisfy gen_expr()'s "every node leaves its value
    /// in RAX" postcondition. Same REX.W + 0x89 (`mov r/m64, r64`) +
    /// ModRM 11 010 000 (reg = RDX, rm = RAX) shape as
    /// emit_mov_rcx_rax()/emit_mov_rbp_rsp() above.
    unsafe fn emit_mov_rax_rdx(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x89, 0xD0]) }; // mov rax, rdx
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
    /// MILESTONE 103: `movsx rax, al` -- REX.W + `0F BE` (the SAME
    /// MOVSX r64, r/m8 opcode `emit_movsx_rax_rbp_off_byte()` (Milestone
    /// 102) already uses for its own memory-operand form), ModRM 0xC0 =
    /// 11 000 000 (mod=11 register-direct, reg=RAX, rm=AL) -- the exact
    /// same register-direct ModRM shape `emit_movzx_eax_al()` just above
    /// already uses for ITS sibling opcode, this one just sign- instead
    /// of zero-extending, and needing REX.W since the destination is a
    /// full 64-bit RAX rather than the implicitly-64-bit-zero-extending
    /// EAX. This is the ONLY codegen `char` as a function RETURN type
    /// needs: unlike a `char` local/parameter (Milestone 102/103, which
    /// narrow via a real STACK SLOT read), a `return` statement's value
    /// lives in RAX with no slot to narrow through, so a `char`-
    /// returning function's own STMT_RETURN emits this real
    /// register-to-register narrow BEFORE the epilogue, truncating and
    /// sign-extending whatever gen_expr() computed down to its own
    /// declared 8-bit width -- e.g. `char f() { return 300; }` genuinely
    /// returns `44` (`300 & 0xFF`), not `300`.
    unsafe fn emit_movsx_rax_al(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x0F, 0xBE, 0xC0]) };
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

    /// MILESTONE 107: `add rsp, imm8` (REX.W 83 /0 ib) -- the real
    /// caller-side stack cleanup a call with more than MAX_REG_PARAMS
    /// arguments needs after it returns, to reclaim the bytes its own
    /// EXPR_CALL codegen pushed for the callee's stack-passed argument
    /// tail and deliberately never popped back off. Same ModRM SHAPE as
    /// emit_prologue()'s own `sub rsp, imm8` (mod=11, rm=100=rsp, disp8-
    /// sized immediate follows) -- only the middle 3 ModRM bits differ:
    /// opcode-extension digit 0 (ADD) instead of digit 5 (SUB), the same
    /// "middle ModRM bits pick which 83-family sub-opcode" shape
    /// emit_neg_rax() above already relies on for the unrelated F7
    /// opcode's own digit field. `0xC4` = mod=11/reg=000/rm=100,
    /// individually hand-verified against the Intel SDM's own encoding
    /// table before being written here, the same discipline every other
    /// CodeBuf method above already used. `bytes` is always a small
    /// multiple of 8 in practice (`stack_argc * 8`, and MAX_PARAMS -
    /// MAX_REG_PARAMS is currently only 4 -- max 32), always comfortably
    /// inside imm8's real signed range, so this never needs a wider
    /// immediate encoding.
    unsafe fn emit_add_rsp_imm8(&mut self, bytes: u8) {
        unsafe { self.push_bytes(&[0x48, 0x83, 0xC4, bytes]) };
    }

    // MILESTONE 108: the four real new encodings a single-dimension
    // fixed-size `int` array's own indexed read/write needs, on top of
    // everything Milestone 68-107 already built -- each individually
    // hand-verified against the Intel SDM's own encoding tables before
    // being written here, the same discipline every other CodeBuf
    // method above already used. Every array-element access this
    // codegen ever emits (EXPR_INDEX in gen_expr(), STMT_ASSIGN_INDEX in
    // gen_stmt_list()) uses exactly these four in the exact same real
    // stack-machine idiom every other node in this file already uses
    // (evaluate into RAX, PUSH to save across a nested evaluation, POP
    // into RCX, combine) -- no new persistent/callee-saved register is
    // ever introduced, so this milestone needs no prologue/epilogue
    // save-restore of anything beyond what already existed.

    /// `LEA r64, [rbp+disp8]` (REX.W `8D` /r) -- computes the real
    /// EFFECTIVE ADDRESS `rbp+off` into RAX, WITHOUT dereferencing it
    /// (unlike emit_mov_rax_rbp_off()'s `8B` /r sibling just above,
    /// which loads the 8 bytes AT that address). Exact same ModRM byte
    /// (`0x45` = mod=01/reg=RAX(000)/rm=RBP(101), disp8 follows) as
    /// emit_mov_rax_rbp_off() -- only the opcode differs (`8D` LEA vs.
    /// `8B` MOV) -- because LEA and this MOV form share the identical
    /// real x86 "compute memory operand" addressing-mode encoding; LEA
    /// simply never issues the load MOV does. This is how an array
    /// variable's own `find_var()`-returned `stack_off` (element 0's
    /// offset -- see collect_vars_rec()'s own Milestone 108 doc comment)
    /// becomes a real runtime BASE ADDRESS an index can be added to or
    /// subtracted from, the one thing no encoding before this milestone
    /// could do (every prior slot access was always a compile-time-
    /// constant disp8 straight off `rbp`, since every prior variable was
    /// exactly one slot).
    unsafe fn emit_lea_rax_rbp_off(&mut self, off: i32) {
        unsafe { self.push_bytes(&[0x48, 0x8D, 0x45, off as i8 as u8]) };
    }
    /// `SHL r/m64, imm8` (REX.W `C1` /4 ib) applied to RCX specifically
    /// -- the SAME real "Shift Group 2" opcode family
    /// emit_shl_rax_cl()/emit_sar_rax_cl() above already document in
    /// full (opcode-extension digit 4 = SHL/SAL), just the IMMEDIATE-
    /// count sibling (`C1`) of that pair's shift-by-CL form (`D3`)
    /// instead, because this milestone's own shift count is always the
    /// same compile-time constant 3 (every array element is a full
    /// 8-byte slot, `x << 3 == x * 8`, the same real "scale by element
    /// size" arithmetic a real SysV-ABI-targeting compiler emits for any
    /// scaled-index addressing it cannot fold into a single SIB byte's
    /// own 1/2/4/8 scale field -- this codegen never builds a SIB byte
    /// at all, so the scale is done explicitly here instead), not a
    /// runtime value the way `<<`'s own operator codegen needs CL for.
    /// ModRM `0xE1` = mod=11 (register-direct) / reg=100 (SHL's own
    /// opcode-extension digit) / rm=001 (RCX) -- the same "middle ModRM
    /// bits pick which C1/D3/F7-family sub-opcode" shape
    /// emit_neg_rax()/emit_shl_rax_cl() above both already rely on,
    /// generalized past RAX-only (used everywhere else in this file) to
    /// RCX specifically, since RCX is where this milestone's own index-
    /// address codegen (see EXPR_INDEX/STMT_ASSIGN_INDEX below) already
    /// leaves the popped-back-off index value.
    unsafe fn emit_shl_rcx_imm8(&mut self, imm: u8) {
        unsafe { self.push_bytes(&[0x48, 0xC1, 0xE1, imm]) };
    }
    /// `MOV RAX, [RAX]` (REX.W `8B` /r) -- a real array-element LOAD
    /// through a runtime-computed address already sitting in RAX itself
    /// (the exact address emit_lea_rax_rbp_off()+emit_shl_rcx_imm8()+
    /// emit_sub_rax_rcx() -- the latter two both ALREADY EXISTING
    /// encodings, reused unchanged -- leave there; see EXPR_INDEX's own
    /// codegen below for the full sequence). ModRM `0x00` = mod=00
    /// (no displacement) / reg=000 (RAX, the destination) / rm=000
    /// (RAX, the address register) -- mod=00 with rm=000 is real plain
    /// `[RAX]` register-indirect addressing, needing neither a SIB byte
    /// (only rm=100/RSP does) nor a disp32 (only rm=101/RBP with mod=00
    /// does, the real RIP-relative special case) -- individually
    /// checked against the Intel SDM's own ModRM/SIB addressing-mode
    /// table before picking RAX/RCX (not, say, RBX/RBP) as this
    /// milestone's own two scratch registers specifically so this and
    /// emit_mov_mem_rcx_rax() below both stay in this simplest real
    /// addressing-mode class, no SIB byte ever needed anywhere in this
    /// milestone's own new codegen.
    unsafe fn emit_mov_rax_mem_rax(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x8B, 0x00]) };
    }
    /// MILESTONE 112: `MOVSX RAX, BYTE PTR [RAX]` (REX.W `0F BE` /r) --
    /// this milestone's one genuinely NEW real x86_64 encoding, a real
    /// `char *` dereference-READ. Combines two halves each ALREADY
    /// individually hand-verified against the Intel SDM by an earlier
    /// milestone: the opcode bytes (`0F BE`, MOVSX r64, r/m8, REX.W for
    /// the 64-bit destination) are IDENTICAL to
    /// emit_movsx_rax_rbp_off_byte()'s own (Milestone 102); the ModRM
    /// byte (`0x00` = mod=00/reg=000 (RAX, destination)/rm=000 (RAX, the
    /// address register), real plain `[RAX]` register-indirect
    /// addressing, no SIB, no displacement) is IDENTICAL to
    /// emit_mov_rax_mem_rax()'s own, just above (Milestone 108). Neither
    /// half is new; what IS genuinely new here is the claim that
    /// COMBINING them -- a byte-sized, sign-extending source, addressed
    /// through a register-indirect operand instead of an rbp-relative
    /// one -- is itself a real, valid, distinct instruction encoding,
    /// checked directly against the Intel SDM's own MOVSX entry rather
    /// than assumed correct by analogy. This is the one place this
    /// milestone's own `*p` dereference (EXPR_UNARY's OP_DEREF arm,
    /// gen_expr(), gated on `find_var()`'s new `is_char_ptr` field)
    /// narrows to a real signed byte, exactly mirroring what an ordinary
    /// `char` variable's own read already does for its rbp-relative slot
    /// -- the pointee, not the pointer's own value (an address, never
    /// narrowed), is what narrows.
    unsafe fn emit_movsx_rax_mem_rax_byte(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x0F, 0xBE, 0x00]) };
    }
    /// `MOV [RCX], RAX` (REX.W `89` /r) -- the store-side sibling of
    /// emit_mov_rax_mem_rax() above: writes RAX's own value to the real
    /// runtime address already sitting in RCX (STMT_ASSIGN_INDEX's own
    /// codegen below computes the target address into RAX, PUSHes it,
    /// evaluates the RHS into RAX -- necessarily clobbering the address
    /// register -- then POPs the saved address back into RCX
    /// specifically so this final store has a value register (RAX) and
    /// an address register (RCX) that are genuinely different
    /// registers). ModRM `0x01` = mod=00 (no displacement) / reg=000
    /// (RAX, the source) / rm=001 (RCX, the address register) -- same
    /// real "mod=00, rm not 100/101" plain register-indirect addressing
    /// class emit_mov_rax_mem_rax() above already uses, source and
    /// destination roles swapped between the two (MOV's `89` opcode is
    /// r/m,r -- register TO memory -- versus `8B`'s r,r/m -- memory TO
    /// register -- the exact same opcode-direction pairing this file's
    /// own emit_mov_rax_rbp_off()/emit_mov_rbp_off_rax() pair already
    /// established for the rbp-relative case).
    unsafe fn emit_mov_mem_rcx_rax(&mut self) {
        unsafe { self.push_bytes(&[0x48, 0x89, 0x01]) };
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
/// evaluated and pushed, popped back off into these) and in a callee's own
/// prologue (spilling each incoming register to its own stack slot) -- ONE
/// real mapping, used both directions, rather than two independently-
/// written tables that could silently drift apart.
///
/// MILESTONE 107: callers must only ever pass `idx < MAX_REG_PARAMS` (4)
/// now that MAX_PARAMS is 8 -- both real call sites (EXPR_CALL's codegen
/// and gen_program()'s own prologue parameter-spill loop) already gate
/// every call to this function behind exactly that check, so the `_ =>
/// REG_RCX` arm below is unreachable in practice, kept only because it
/// was already the correct, harmless behavior for `idx == 3` before this
/// milestone and narrowing the match to a hard `panic!`/abort for `idx >=
/// 4` would pull in the same `core::fmt` machinery this file's own top
/// doc comment already explains this `#![no_std]`, `panic=abort` binary
/// cannot afford to link.
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
            let slot = unsafe { find_var(src_ptr, vars_ptr, nvars, e.ident_off as u64, e.ident_len as u64) };
            match slot {
                // MILESTONE 102: a `char` variable's whole real
                // narrowness -- read only its low byte, sign-extended,
                // instead of the plain full-width load every `int`
                // variable (and every variable before this milestone)
                // still uses. See emit_movsx_rax_rbp_off_byte()'s own
                // doc comment for the full reasoning.
                // MILESTONE 110: real array-to-pointer DECAY. If this
                // name is an array (`is_array != 0` -- collect_vars_rec()'s
                // own Milestone 110 tag), reading it as a bare expression
                // no longer loads the 8-byte VALUE sitting in its own
                // element-0 slot (Milestone 108's own disclosed
                // simplification); it computes that slot's own ADDRESS
                // instead, via `emit_lea_rax_rbp_off()` -- Milestone
                // 108's own array-base-address encoding, reused
                // unchanged, the exact same instruction OP_ADDR
                // (Milestone 109) already uses for `&x`. This is real C's
                // own decay rule (an array used as an ordinary
                // expression yields a pointer to its first element) and
                // is what makes `foo(arr)` at a call site pass a real
                // address -- no special-casing anywhere in EXPR_CALL's
                // own codegen is needed, since EXPR_CALL evaluates each
                // argument through this same gen_expr()/EXPR_IDENT path
                // like any other expression.
                Some((o, is_char, is_array, _is_char_ptr)) => {
                    if is_array != 0 {
                        unsafe { buf.emit_lea_rax_rbp_off(o) };
                    } else if is_char != 0 {
                        unsafe { buf.emit_movsx_rax_rbp_off_byte(o) };
                    } else {
                        unsafe { buf.emit_mov_rax_rbp_off(o) };
                    }
                }
                None => return Err(CodeGenError::UndeclaredVariable(e.ident_off as u64)),
            }
        }
        // MILESTONE 108: `arr[idx]` -- a real array-element READ. `find_var()`
        // resolves the array's own NAME to its element-0 base offset
        // (`o`, the same base_off collect_vars_rec()'s own Milestone 108
        // doc comment derives; the `is_char` field it also returns is
        // ignored here -- this first slice's arrays are `int`-only, see
        // parse_array_decl()'s own doc comment) exactly like EXPR_IDENT
        // above, then this arm does the one genuinely new thing no
        // scalar variable ever needed: computes a REAL RUNTIME address
        // instead of using `o` as an immediate disp8 directly.
        //   <idx>                        (gen_expr, index into RAX)
        //   push rax                     (save the index across the LEA)
        //   lea rax, [rbp+o]             (RAX = &arr[0], element 0's real address)
        //   pop rcx                      (RCX = the index)
        //   shl rcx, 3                   (RCX = index * 8 -- 8 bytes/element)
        //   sub rax, rcx                 (RAX = &arr[0] - index*8 = &arr[index] --
        //                                 correct because collect_vars_rec()'s own
        //                                 element layout grows MORE negative as the
        //                                 index grows, the same direction subtracting
        //                                 a larger index*8 already moves RAX)
        //   mov rax, [rax]               (RAX = arr[index], the real load)
        // Only RAX/RCX are ever touched -- see emit_lea_rax_rbp_off()'s
        // own doc comment above for why no callee-saved register needed
        // saving/restoring for this milestone. No bounds check is
        // emitted (a real, disclosed simplification -- see this
        // milestone's own README entry's "still genuinely open" note):
        // an out-of-range `idx` reads whatever real stack memory sits at
        // that computed address, exactly the same real undefined
        // behavior an unchecked C array access already has on genuine
        // hardware, not a new gap this codegen introduces.
        // MILESTONE 110: `p[idx]` now works for a POINTER variable too
        // (a pointer parameter, or a plain local `int *p`), not only a
        // real declared array -- real C itself defines `p[idx]` as
        // `*(p + idx)` regardless of whether `p` is an array or a
        // pointer, and this codegen now genuinely matches that: the base
        // address computation branches on `is_array` (find_var()'s own
        // Milestone 110 third field). For an ARRAY (`is_array != 0`,
        // unchanged from Milestone 108), the base is the slot's own
        // ADDRESS (`emit_lea_rax_rbp_off()`, computed in place -- the
        // array's own storage IS the element data). For a POINTER
        // (`is_array == 0`), the base is the slot's own VALUE
        // (`emit_mov_rax_rbp_off()`, an ordinary scalar read, reused
        // unchanged from every plain EXPR_IDENT read this codegen has
        // ever emitted) -- the pointer variable does not itself hold
        // any element data, it holds the ADDRESS of element 0 somewhere
        // else (typically a caller's own decayed array argument). Once
        // the correct base is in RAX, every remaining step (index-scale-
        // subtract-load) is IDENTICAL to Milestone 108's own original
        // sequence, unchanged -- INCLUDING the subtract direction.
        // This deserves its own note: Milestone 108's own array layout
        // grows toward LOWER real addresses as the index grows (element
        // `k` sits at `off - 8*k` relative to `rbp`), a direct,
        // already-disclosed consequence of this codegen's own downward-
        // growing stack frame, not real C's usual "addresses increase
        // with index" convention. Since the ONLY real source of a
        // multi-element address this subset can ever produce is exactly
        // such a stack array (there is no heap/malloc-backed array
        // anywhere in this language), a pointer's own VALUE -- whenever
        // it was obtained by decaying a real array argument (this
        // milestone's own EXPR_IDENT arm, above) -- points at memory
        // that ALREADY obeys that same "index grows, address shrinks"
        // layout. Subscripting through the pointer with the SAME
        // subtract-based arithmetic the array case already uses is
        // therefore not an arbitrary reuse of convenient code -- it is
        // what makes `p[idx]` inside the callee land on the EXACT same
        // bytes `arr[idx]` would inside the caller, the two conventions
        // made to agree by construction.
        EXPR_INDEX => {
            let slot = unsafe { find_var(src_ptr, vars_ptr, nvars, e.ident_off as u64, e.ident_len as u64) };
            let (base_off, is_array) = match slot {
                Some((o, _is_char, is_array, _is_char_ptr)) => (o, is_array),
                None => return Err(CodeGenError::UndeclaredVariable(e.ident_off as u64)),
            };
            unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.left) }?;
            unsafe { buf.emit_push_rax() };
            if is_array != 0 {
                unsafe { buf.emit_lea_rax_rbp_off(base_off) };
            } else {
                unsafe { buf.emit_mov_rax_rbp_off(base_off) };
            }
            unsafe { buf.emit_pop_reg(REG_RCX) };
            unsafe { buf.emit_shl_rcx_imm8(3) };
            unsafe { buf.emit_sub_rax_rcx() };
            unsafe { buf.emit_mov_rax_mem_rax() };
        }
        // MILESTONE 72: a call expression's own real codegen -- evaluate
        // every argument into RAX, PUSHing each one immediately after
        // (the exact same "stack machine" technique EXPR_BINARY already
        // uses for its own two operands just below, generalized past two
        // operands to however many this call has), THEN pop the first
        // MAX_REG_PARAMS of them back off into the real SysV integer
        // argument registers (RDI, RSI, RDX, RCX, in that order -- see
        // param_reg()'s own doc comment above for why this exact ordering
        // was chosen), finally a real `call rel32` to the callee --
        // MILESTONE 74: no longer necessarily to an "already-known" code
        // offset. gen_program() below now runs a real first pass that
        // registers every function's own name/param_count before any body
        // is compiled, so find_func() finds a real match here regardless
        // of source order; what it returns as `code_off` may still be the
        // real UNRESOLVED_CODE_OFF sentinel (a genuine forward reference --
        // the callee hasn't been compiled yet). If so, this emits a real
        // placeholder instead (emit_call_placeholder()) and records a
        // pending-patch entry (this field's own offset, the callee's own
        // table index) for gen_program()'s own final backpatch pass to
        // resolve once every function's own code_off is final --
        // otherwise (the callee is already compiled, backward reference or
        // self), the exact original single-instruction emit_call() path
        // runs, unchanged. On return, RAX already holds the callee's own
        // result (its own RETURN statement's gen_expr() call already left
        // it there before that function's epilogue ran) -- the same
        // "gen_expr()'s own postcondition is always value-in-RAX"
        // invariant this whole function already guarantees for every
        // OTHER node kind, so a call expression composes with the rest of
        // this codegen (as an operand of arithmetic, as another call's
        // own argument, etc.) with zero special-casing anywhere else.
        //
        // MILESTONE 107: the push loop below now runs argument index
        // DESCENDING (`call_argc-1` downto `0`) instead of ascending --
        // the real, deliberate fix for the exact landmine a prior
        // session's own groundwork disclosed (see this file's own top
        // Milestone 107 doc comment): with MAX_PARAMS now 8 but only
        // MAX_REG_PARAMS (4) real argument registers, argument indices 4
        // and up must end up sitting on the stack, in ascending order,
        // for the callee's own prologue to read (see gen_program()'s own
        // updated prologue comment for the receiving side) -- and the OLD
        // ascending-push-then-descending-pop scheme buries them under the
        // register-destined arguments instead of leaving them accessible.
        // Pushing DESCENDING (argc-1 first, ... arg0 last) means arg0 ends
        // up on TOP of the stack once every argument is pushed, with
        // arg1, arg2, ... below it in ascending order -- so popping
        // ASCENDING (0..min(call_argc, MAX_REG_PARAMS)) now correctly
        // drains exactly the register-destined arguments off the top,
        // lands each in ITS OWN matching register (pop order now equals
        // argument index, not its reverse), and leaves any REMAINING
        // arguments (index MAX_REG_PARAMS and up, if this call has more
        // than 4) already sitting on the stack in exactly the ascending
        // order the callee's own prologue expects at `[rbp+16]`,
        // `[rbp+24]`, ... -- zero extra push/pop work needed for the
        // stack-passed tail, it falls out of the SAME reversed push loop
        // for free. This flips argument EVALUATION order from the
        // original left-to-right to right-to-left, which would be a real
        // observable behavior change in a language with expression side
        // effects (C itself famously leaves this unspecified for exactly
        // this reason) -- but this subset-C has neither assignment
        // EXPRESSIONS (only STMT_ASSIGN, a statement -- see this file's
        // own grammar; there is no EXPR_ASSIGN at all) nor `++`/`--` (no
        // such tokens exist in this lexer) nor any global/static variable
        // a nested call could mutate for a sibling argument to observe
        // (every variable is a function-local stack slot -- see
        // collect_vars_for_function()'s own doc comment) nor any I/O a
        // compiled program can perform (no builtin print/write is exposed
        // to compiled subset-C source; a program's only externally
        // visible effect is its own single sys_exit(result) at the very
        // end) -- so no subset-C argument sub-expression can have an
        // effect observable by a sibling argument or by anything else,
        // and this reordering is genuinely, verifiably unobservable, not
        // merely assumed safe. Because the caller (not the callee) pushed
        // the stack-passed tail, the caller must also clean it back off
        // after the call returns -- see the real `emit_add_rsp_imm8()`
        // call below.
        EXPR_CALL => {
            let target = unsafe { find_func(src_ptr, funcs_ptr, nfuncs, e.ident_off as u64, e.ident_len as u64) };
            let (code_off, callee_param_count, callee_idx) = match target {
                Some(t) => t,
                None => return Err(CodeGenError::UndeclaredFunction(e.ident_off as u64)),
            };
            if e.call_argc != callee_param_count {
                return Err(CodeGenError::ArgCountMismatch(e.ident_off as u64));
            }
            let argc = e.call_argc as u64;
            let mut ai_push: i64 = argc as i64 - 1;
            while ai_push >= 0 {
                let arg_node = unsafe { call_arg_read(e.call_args_ptr, ai_push as u64) };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, arg_node) }?;
                unsafe { buf.emit_push_rax() };
                ai_push -= 1;
            }
            let reg_argc = if argc > MAX_REG_PARAMS { MAX_REG_PARAMS } else { argc };
            let mut ai_pop: u64 = 0;
            while ai_pop < reg_argc {
                let reg = unsafe { param_reg(ai_pop) };
                unsafe { buf.emit_pop_reg(reg) };
                ai_pop += 1;
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
            // MILESTONE 107: caller-cleans-stack -- any argument past
            // MAX_REG_PARAMS was left on the stack (deliberately never
            // popped above) for the callee's own prologue to read
            // directly; the CALLER is the one who pushed those bytes, so
            // the caller is the one who must reclaim them now that the
            // callee has returned (RAX already holds the result; this
            // adjustment only moves RSP, it never touches RAX). A call
            // with <= MAX_REG_PARAMS arguments (every call before this
            // milestone, and the overwhelming majority of calls after it)
            // takes stack_argc == 0 here and this is a real, cheap no-op.
            let stack_argc = argc - reg_argc;
            if stack_argc > 0 {
                unsafe { buf.emit_add_rsp_imm8((stack_argc * 8) as u8) };
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
        // MILESTONE 109: `&x` -- real address-of codegen. `e.left`
        // always points at a plain EXPR_IDENT node (parse_unary()'s own
        // `&` arm builds nothing else), naming the variable whose
        // ADDRESS is wanted, not its value -- so this arm deliberately
        // does NOT recurse into gen_expr(e.left) the way every other
        // EXPR_UNARY case does (that would emit an ordinary VALUE load,
        // the wrong thing for `&`). Instead it reads that EXPR_IDENT
        // node directly, resolves its name via find_var() exactly like
        // an ordinary EXPR_IDENT read would, and emits ONE instruction
        // -- `emit_lea_rax_rbp_off()`, Milestone 108's own array-base-
        // address encoding, reused completely unchanged (an ordinary
        // scalar's own `rbp`-relative slot is exactly as valid an LEA
        // operand as an array's element-0 slot already was) -- to
        // compute `rbp+off` into RAX WITHOUT dereferencing it. Zero new
        // CodeBuf encodings.
        EXPR_UNARY if e.op == OP_ADDR => {
            let operand = unsafe { core::ptr::read(e.left as *const ExprNode) };
            let slot = unsafe { find_var(src_ptr, vars_ptr, nvars, operand.ident_off as u64, operand.ident_len as u64) };
            match slot {
                // MILESTONE 110: `_is_array` is ignored here exactly like
                // `_is_char` already was -- `&arr` (an array's own bare
                // name) and `&arr[0]` compute the SAME real numeric
                // address either way (this arm's own `emit_lea_rax_
                // rbp_off()` call already matches real C's own `&arr`
                // rule for that reason, needing no is_array branch of
                // its own, unlike EXPR_IDENT's plain read above it).
                Some((o, _is_char, _is_array, _is_char_ptr)) => unsafe { buf.emit_lea_rax_rbp_off(o) },
                None => return Err(CodeGenError::UndeclaredVariable(operand.ident_off as u64)),
            }
        }
        // MILESTONE 109: `*p` as an RVALUE -- real dereference-read
        // codegen. `e.left` is evaluated completely ordinarily (the
        // same gen_expr() recursion every other EXPR_UNARY case already
        // uses), leaving the pointer's own VALUE (an address) in RAX;
        // one more instruction -- `emit_mov_rax_mem_rax()`, Milestone
        // 108's own array-element-load encoding ("MOV RAX,[RAX]"),
        // reused completely unchanged -- loads the 8 bytes AT that
        // address. Zero new CodeBuf encodings: this milestone's entire
        // new codegen surface (this arm, OP_ADDR above, and
        // STMT_ASSIGN_DEREF below in gen_stmt_list()) is built entirely
        // out of `emit_lea_rax_rbp_off()`/`emit_mov_rax_mem_rax()`/
        // `emit_mov_mem_rcx_rax()`, all three already hand-verified
        // against the Intel SDM by Milestone 108 -- a real, deliberate
        // "single-level pointer read/write is exactly array-element
        // read/write with the index folded away" reuse, not a
        // coincidence: an array-element access already computes
        // "a runtime address, then load/store through it"; a pointer
        // dereference is the same operation with the address coming
        // from a variable's own VALUE instead of a LEA+index
        // computation.
        // MILESTONE 112: real `char *` dereference-READ narrowing. `e.left`
        // is evaluated completely ordinarily first (unchanged from
        // Milestone 109 -- the pointer's own VALUE ends up in RAX exactly
        // as before), but THEN, separately, this arm checks whether
        // `e.left` is a plain `EXPR_IDENT` naming a `char *` variable
        // (`find_var()`'s new fourth field, `is_char_ptr`) and, if so,
        // emits the new `emit_movsx_rax_mem_rax_byte()` (sign-extending
        // BYTE load through the address already sitting in RAX -- the
        // real char-pointee-narrowing counterpart to
        // `emit_movsx_rax_rbp_off_byte()`'s own rbp-relative form) instead
        // of the ordinary full-width `emit_mov_rax_mem_rax()`. A real,
        // deliberate, DISCLOSED scope restriction: `e.left` is checked
        // for the EXACT shape `parse_unary()`'s own `*` grammar always
        // produces when the pointer is held directly in a local variable
        // (every reachable `char *` value in THIS milestone's own scope
        // -- no `char *` function parameters/returns, no array decay to a
        // `char *`, so a local pointer variable is the only way this
        // milestone can ever construct one) -- any OTHER shape (a nested
        // dereference `**q`, a call result `*f()`, etc.) falls through to
        // the ordinary full-width load unconditionally, the same
        // "resolved by operand SHAPE, not attempted for every expression"
        // precedent Milestone 109's own `&` restriction already
        // established for this exact operator family.
        EXPR_UNARY if e.op == OP_DEREF => {
            unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.left) }?;
            let operand = unsafe { core::ptr::read(e.left as *const ExprNode) };
            let is_char_pointee = if operand.kind == EXPR_IDENT {
                match unsafe { find_var(src_ptr, vars_ptr, nvars, operand.ident_off as u64, operand.ident_len as u64) } {
                    Some((_, _, _, is_char_ptr)) => is_char_ptr != 0,
                    None => false,
                }
            } else {
                false
            };
            if is_char_pointee {
                unsafe { buf.emit_movsx_rax_mem_rax_byte() };
            } else {
                unsafe { buf.emit_mov_rax_mem_rax() };
            }
        }
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
        // MILESTONE 90: `cond ? then : else`. The exact if/else lowering
        // gen_stmt_list's STMT_IF arm already uses, but as an EXPRESSION
        // (result in RAX) rather than statements:
        //   <cond>            (into RAX)
        //   test rax, rax
        //   jz else_label     (forward-patched)
        //   <then>            (into RAX)
        //   jmp end_label     (forward-patched)
        // else_label:
        //   <else>            (into RAX)
        // end_label:
        // Exactly one side runs; both leave their value in RAX, so the
        // whole expression's postcondition holds either way. Zero new
        // CodeBuf encodings -- emit_jz_placeholder/emit_jmp_placeholder/
        // patch_rel32 are all Milestone 70's. `left`=cond, `right`=then,
        // `call_args_ptr`=else (field reuse, see EXPR_TERNARY's const).
        EXPR_TERNARY => {
            unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.left) }?;
            unsafe { buf.emit_test_rax_rax() };
            let jz_field = unsafe { buf.emit_jz_placeholder() };
            unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.right) }?;
            let jmp_field = unsafe { buf.emit_jmp_placeholder() };
            unsafe { buf.patch_rel32(jz_field) };
            unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, e.call_args_ptr) }?;
            unsafe { buf.patch_rel32(jmp_field) };
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
                    // MILESTONE 89: `%` -- same `cqo; idiv rcx` as `/`
                    // (idiv leaves the remainder in RDX alongside the
                    // quotient in RAX), then move the remainder into RAX.
                    b'%' => {
                        unsafe { buf.emit_cqo() };
                        unsafe { buf.emit_idiv_rcx() };
                        unsafe { buf.emit_mov_rax_rdx() };
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
    // MILESTONE 91: 1 if a `break` is legal in this list -- true inside
    // a loop body (as before) OR inside a `switch`'s case bodies.
    // `continue` still checks `loop_top` (loops only). Threaded through
    // STMT_IF like the loop fields; set by STMT_WHILE and STMT_SWITCH.
    break_ok: u64,
    // MILESTONE 103: 1 if the ENCLOSING FUNCTION's own declared return
    // type is `char` -- a real per-function constant, not a per-block
    // one, so (unlike loop_top/break_ok) every recursive call below
    // passes it through completely UNCHANGED, the same "threaded but
    // never modified" shape vars_ptr/funcs_ptr/mode already have.
    // STMT_RETURN is the only arm that reads it (see its own doc
    // comment). gen_function() passes `f.is_char_return` for the
    // original single-function path; gen_program()'s per-function loop
    // passes each FuncDef's own field.
    is_char_return: u64,
) -> Result<(), CodeGenError> {
    let mut cur = head;
    while cur != 0 {
        let s = unsafe { core::ptr::read(cur as *const StmtNode) };
        match s.kind {
            STMT_DECL => {}
            // MILESTONE 111: the slot lookup (and its new `is_array`
            // check) now happens BEFORE the RHS is compiled -- the same
            // find-slot-first shape STMT_ASSIGN_INDEX already uses just
            // below -- so a rejected `arr = expr;` costs zero wasted
            // CodeBuf bytes for the RHS rather than emitting it and then
            // discarding the whole buffer on Err.
            STMT_ASSIGN => {
                let slot = unsafe { find_var(src_ptr, vars_ptr, nvars, s.ident_off as u64, s.ident_len as u64) };
                // MILESTONE 111: reject a bare array name as an ASSIGN
                // target at compile time. Before this milestone,
                // `arr = expr;` silently overwrote element 0's raw
                // 8-byte VALUE (this store side has always ignored
                // `is_array`, back to before it even existed) even
                // though Milestone 110 made READING `arr` decay to its
                // own ADDRESS -- a real, disclosed asymmetry named in
                // that milestone's own closing "Scope" paragraph. Real C
                // has no answer to "assign to an array" either -- an
                // array is not a modifiable lvalue -- so this is the
                // same call, not new invented semantics.
                let store_off = match slot {
                    Some((o, _is_char, is_array, _is_char_ptr)) => {
                        if is_array != 0 {
                            return Err(CodeGenError::ArrayNotAssignable(s.ident_off as u64));
                        }
                        o
                    }
                    None => return Err(CodeGenError::UndeclaredVariable(s.ident_off as u64)),
                };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                // MILESTONE 102: the store side is UNCHANGED by `char`
                // -- always the same full 8-byte write regardless of
                // the slot's type (the second `find_var()` field is
                // ignored here). See emit_movsx_rax_rbp_off_byte()'s
                // own doc comment for why storing full-width and
                // narrowing only on read is correct, not a shortcut.
                unsafe { buf.emit_mov_rbp_off_rax(store_off) };
            }
            // MILESTONE 108: `arr[idx] = expr;` -- a real array-element
            // WRITE, `s.then_body` (field-reused, see STMT_ASSIGN_INDEX's
            // own const doc comment) holding the index sub-expression's
            // ExprNode pointer. The address-computation half is exactly
            // EXPR_INDEX's own real `lea`+`shl`+`sub` sequence above
            // (gen_expr()'s own doc comment there has the full
            // derivation) -- the one real difference is what happens
            // once the address is known: EXPR_INDEX immediately
            // dereferences it to READ; this arm instead PUSHes the
            // computed address, evaluates the RHS expression into RAX
            // (necessarily clobbering RAX/RCX again -- the RHS can be
            // arbitrarily complex, e.g. another array read), then POPs
            // the saved address back into RCX and stores.
            //   <idx>                        (gen_expr, index into RAX)
            //   push rax                     (save index)
            //   lea rax, [rbp+o]             (RAX = &arr[0])
            //   pop rcx                      (RCX = index)
            //   shl rcx, 3                   (RCX = index * 8)
            //   sub rax, rcx                 (RAX = &arr[index])
            //   push rax                     (save the computed address)
            //   <rhs>                        (gen_expr, value into RAX --
            //                                 may itself use RAX/RCX freely)
            //   pop rcx                      (RCX = &arr[index])
            //   mov [rcx], rax               (arr[index] = value, the real store)
            // MILESTONE 110: exactly like EXPR_INDEX's own read side, the
            // base-address computation now branches on `is_array` so
            // `p[idx] = expr;` works correctly through a POINTER
            // variable too (base = the pointer's own VALUE,
            // `emit_mov_rax_rbp_off()`), not only through a real
            // declared array (base = the slot's own address,
            // `emit_lea_rax_rbp_off()`, unchanged) -- see EXPR_INDEX's
            // own doc comment above for the full derivation, including
            // why reusing the same subtract-based index arithmetic for
            // both cases is what makes a callee's `p[idx] = ...` write
            // land on the exact same bytes a caller's own `arr[idx]`
            // occupies.
            STMT_ASSIGN_INDEX => {
                let slot = unsafe { find_var(src_ptr, vars_ptr, nvars, s.ident_off as u64, s.ident_len as u64) };
                let (base_off, is_array) = match slot {
                    Some((o, _is_char, is_array, _is_char_ptr)) => (o, is_array),
                    None => return Err(CodeGenError::UndeclaredVariable(s.ident_off as u64)),
                };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.then_body) }?;
                unsafe { buf.emit_push_rax() };
                if is_array != 0 {
                    unsafe { buf.emit_lea_rax_rbp_off(base_off) };
                } else {
                    unsafe { buf.emit_mov_rax_rbp_off(base_off) };
                }
                unsafe { buf.emit_pop_reg(REG_RCX) };
                unsafe { buf.emit_shl_rcx_imm8(3) };
                unsafe { buf.emit_sub_rax_rcx() };
                unsafe { buf.emit_push_rax() };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                unsafe { buf.emit_pop_reg(REG_RCX) };
                unsafe { buf.emit_mov_mem_rcx_rax() };
            }
            // MILESTONE 109: `*p = expr;` -- a real pointer dereference
            // WRITE. `s.ident_off`/`s.ident_len` name the POINTER
            // variable `p` (not the pointee) -- its own VALUE (an
            // address) is read first via the ordinary
            // `emit_mov_rax_rbp_off()` load every plain EXPR_IDENT read
            // already uses, then PUSHed to save it across the RHS
            // evaluation (which may itself use RAX/RCX freely -- e.g.
            // another dereference, or a call), then the RHS is
            // evaluated into RAX and stored at the saved address --
            // the exact same "push the address, evaluate the RHS,
            // pop the address back into RCX, store" shape
            // STMT_ASSIGN_INDEX just above already established for
            // `arr[idx] = expr;`, minus the index-address-computation
            // steps (a pointer's own value already IS the target
            // address -- no LEA/SHL/SUB needed at all).
            //   mov rax, [rbp+o]     (RAX = p's own value, the target address)
            //   push rax             (save the target address)
            //   <rhs>                (gen_expr, value into RAX)
            //   pop rcx              (RCX = the saved target address)
            //   mov [rcx], rax       (*p = value, the real store)
            // MILESTONE 112: this arm is UNCHANGED -- `*p = expr;` always
            // does the ordinary full 8-byte store regardless of whether
            // `p` is `int *` or `char *`, matching the exact convention
            // an ordinary `char` SCALAR's own store already uses (`char
            // c = expr;` also always writes the full 8 bytes,
            // `emit_mov_rbp_off_rax()`, narrowing only ever happening on
            // READ -- see that instruction's own Milestone 102 doc
            // comment). This is a real, deliberate design choice, not an
            // oversight: since the pointee, in every case this subset can
            // reach, is always another VarSlot's own full 8-byte stack
            // slot (no packed multi-char buffer exists anywhere in this
            // language), writing all 8 bytes through the pointer lands on
            // the identical bytes an ordinary write to that same variable
            // already would.
            STMT_ASSIGN_DEREF => {
                let slot = unsafe { find_var(src_ptr, vars_ptr, nvars, s.ident_off as u64, s.ident_len as u64) };
                match slot {
                    Some((o, _is_char, _is_array, _is_char_ptr)) => unsafe { buf.emit_mov_rax_rbp_off(o) },
                    None => return Err(CodeGenError::UndeclaredVariable(s.ident_off as u64)),
                }
                unsafe { buf.emit_push_rax() };
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                unsafe { buf.emit_pop_reg(REG_RCX) };
                unsafe { buf.emit_mov_mem_rcx_rax() };
            }
            STMT_RETURN => {
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                // MILESTONE 103: a real narrowing instruction, ONLY when
                // the enclosing function's own declared return type is
                // `char` -- see emit_movsx_rax_al()'s own doc comment
                // for why this is the one case that needs a register-
                // to-register narrow rather than the slot-read narrow
                // every other `char` use (Milestone 102 locals,
                // Milestone 103 parameters) already gets for free.
                if is_char_return != 0 {
                    unsafe { buf.emit_movsx_rax_al() };
                }
                unsafe { emit_epilogue(buf, mode) };
            }
            // MILESTONE 87: `break` -- an unconditional forward jump to
            // the innermost loop's exit, recorded as a placeholder in
            // that loop's own patch list (resolved by STMT_WHILE once the
            // exit address is known). Outside any loop -> a real semantic
            // error, the same shape as UndeclaredVariable.
            STMT_BREAK => {
                if break_ok == 0 {
                    return Err(CodeGenError::BreakOutsideLoop);
                }
                let field = unsafe { buf.emit_jmp_placeholder() };
                unsafe { loop_push_brk(field) }?;
            }
            // MILESTONE 92: a label just marks the current buf position
            // and resolves any pending forward `goto`s to it.
            STMT_LABEL => {
                let pos = buf.len;
                unsafe { goto_define_label(buf, src_ptr, s.ident_off, s.ident_len, pos) }?;
            }
            // MILESTONE 92: `goto` -- a backward jmp if the label is
            // already defined, otherwise a placeholder for later.
            STMT_GOTO => {
                unsafe { goto_emit(buf, src_ptr, s.ident_off, s.ident_len) }?;
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
                unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.then_body, loop_top, loop_is_for, break_ok, is_char_return) }?;
                if s.else_body != 0 {
                    let jmp_field = unsafe { buf.emit_jmp_placeholder() };
                    unsafe { buf.patch_rel32(jz_field) };
                    unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.else_body, loop_top, loop_is_for, break_ok, is_char_return) }?;
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

                unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.then_body, my_top, my_is_for, 1, is_char_return) }?;

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
                    unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.else_body, NOT_IN_LOOP, 0, 0, is_char_return) }?;
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
            // MILESTONE 99: `do { body } while (cond);` -- body FIRST,
            // condition at the BOTTOM:
            //   body_top:  <body>                (gen_stmt_list)
            //   cont_tgt:  <condition>           (gen_expr, into RAX)
            //              test rax, rax
            //              jz exit               (forward-patched)
            //              jmp body_top          (BACKWARD -- repeat)
            //   exit:
            // The body runs unconditionally once before the first test.
            // `continue` inside the body must reach `cont_tgt` (the
            // condition), which is not emitted until after the body --
            // exactly the "forward jump into the loop's cont patch list"
            // situation a `for` loop's `continue` already handles, so the
            // body is recursed with loop_is_for = 1 and every pending
            // continue is patched to `cont_tgt` here. `break` uses the
            // same LOOP_BRK arena + save/restore as STMT_WHILE. Zero new
            // encodings.
            STMT_DOWHILE => {
                let body_top = buf.len;
                let brk_base = unsafe { loop_brk_n() };
                let cont_base = unsafe { loop_cont_n() };

                unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.then_body, body_top, 1, 1, is_char_return) }?;

                // `continue` target: the condition re-check, right here.
                let cn = unsafe { loop_cont_n() };
                let mut i = cont_base;
                while i < cn {
                    unsafe { buf.patch_rel32(loop_cont_at(i)) };
                    i += 1;
                }
                unsafe { loop_set_cont_n(cont_base) };

                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                unsafe { buf.emit_test_rax_rax() };
                let jz_out = unsafe { buf.emit_jz_placeholder() };
                unsafe { buf.emit_jmp_back(body_top) };
                unsafe { buf.patch_rel32(jz_out) };

                let bn = unsafe { loop_brk_n() };
                let mut j = brk_base;
                while j < bn {
                    unsafe { buf.patch_rel32(loop_brk_at(j)) };
                    j += 1;
                }
                unsafe { loop_set_brk_n(brk_base) };
            }
            // MILESTONE 91: `switch`. Real C lowering, discriminant
            // evaluated ONCE:
            //   <discriminant>              (into RAX)
            //   mov [rbp - (nvars+1)*8], rax     (spill to the reserved
            //                                     switch-scratch slot)
            //   -- dispatch: for each case Ci --
            //   mov rax, [scratch]
            //   cmp rax, Ci
            //   jz body_i                   (forward-patched)
            //   ...
            //   jmp default  (or jmp end, if no default)   (fwd-patched)
            //   -- bodies, in source order, FALL THROUGH --
            // body_0:  <case 0 stmts>       (no jmp at the end: falls into body_1)
            // body_1:  <case 1 stmts>
            //   ...
            // default: <default stmts>      (only if present)
            // end:                          <- every `break` jumps here
            // `break` inside a case body reaches `end` through the same
            // LOOP_BRK arena + save/restore STMT_WHILE uses; `continue`
            // is untouched (still targets the enclosing loop via
            // `loop_top`, passed straight through).
            STMT_SWITCH => {
                let scratch_off: i32 = -((nvars + 1) as i32) * 8;
                unsafe { gen_expr(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, s.expr) }?;
                unsafe { buf.emit_mov_rbp_off_rax(scratch_off) };

                let brk_base = unsafe { loop_brk_n() };

                // First pass over the STMT_CASE chain: emit the compares,
                // collecting one jz-placeholder field per case (kept in
                // the LOOP_CONT arena, which is otherwise unused here --
                // save/restore its count too so a `for`'s pending
                // continues, if this switch sits in a for body, are
                // untouched).
                let cont_base = unsafe { loop_cont_n() };
                let mut c = s.then_body;
                while c != 0 {
                    let cn = unsafe { core::ptr::read(c as *const StmtNode) };
                    let label = unsafe { core::ptr::read(cn.expr as *const ExprNode) };
                    unsafe { buf.emit_mov_rax_rbp_off(scratch_off) };
                    unsafe { buf.emit_cmp_rax_imm32(label.int_val as u32) };
                    let jz = unsafe { buf.emit_jz_placeholder() };
                    unsafe { loop_push_cont(jz) }?;
                    c = cn.next;
                }
                // No case matched -> jump to default (or end).
                let miss_jmp = unsafe { buf.emit_jmp_placeholder() };

                // Second pass: emit each case body, patching its own jz
                // to the body's start. Bodies fall through (no jmp
                // between them) -- real C switch semantics.
                let mut idx = cont_base;
                let mut c2 = s.then_body;
                while c2 != 0 {
                    let cn = unsafe { core::ptr::read(c2 as *const StmtNode) };
                    unsafe { buf.patch_rel32(loop_cont_at(idx)) };
                    idx += 1;
                    unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, cn.then_body, loop_top, loop_is_for, 1, is_char_return) }?;
                    c2 = cn.next;
                }
                unsafe { loop_set_cont_n(cont_base) };

                // Default body (if any) -- the no-match jump lands at
                // its start; otherwise it lands at `end` below.
                if s.else_body != 0 {
                    unsafe { buf.patch_rel32(miss_jmp) };
                    unsafe { gen_stmt_list(buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs, pending_ptr, pending_count_ptr, mode, s.else_body, loop_top, loop_is_for, 1, is_char_return) }?;
                } else {
                    unsafe { buf.patch_rel32(miss_jmp) };
                }

                // `end`: patch every `break` this switch's bodies recorded.
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
    // MILESTONE 91: +1 slot, always, for STMT_SWITCH to spill its
    // discriminant into (rbp - (nvars+1)*8). 8 wasted bytes for a
    // switch-free function -- negligible, and avoids an extra AST walk.
    let raw_frame = (nvars + 1) * 8;
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
    unsafe { goto_reset() };  // MILESTONE 92: fresh label/goto arena per function
    unsafe { gen_stmt_list(&mut buf, src_ptr, vars_ptr, nvars, 0, 0, 0, 0, mode, f.body, NOT_IN_LOOP, 0, 0, f.is_char_return as u64) }?;
    if unsafe { goto_pend_n() } > 0 {
        let off = unsafe { core::ptr::read(core::ptr::addr_of!(GOTO_PEND_OFF) as *const u32) };
        return Err(CodeGenError::UndefinedLabel(off as u64));
    }

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
        // MILESTONE 91: +1 slot, always, for STMT_SWITCH to spill its
    // discriminant into (rbp - (nvars+1)*8). 8 wasted bytes for a
    // switch-free function -- negligible, and avoids an extra AST walk.
    let raw_frame = (nvars + 1) * 8;
        let frame_bytes = if raw_frame == 0 { 0u8 } else { (((raw_frame + 15) / 16) * 16) as u8 };

        unsafe { buf.emit_prologue(frame_bytes) };

        // Real prologue-time argument spill -- see emit_mov_rbp_off_reg()
        // and param_reg()'s own doc comments above for the full real
        // encoding/convention reasoning. Each parameter's own stack slot
        // was already assigned by collect_vars_for_function() above, in
        // the SAME declaration order param_reg() itself indexes by.
        //
        // MILESTONE 107: parameter index MAX_REG_PARAMS (4) and up is no
        // longer spillable from a register at all -- there is no 5th
        // argument register in this ABI -- so this loop now branches on
        // exactly the same MAX_REG_PARAMS boundary the EXPR_CALL call-site
        // codegen above uses. For pi < MAX_REG_PARAMS, completely
        // UNCHANGED: spill the incoming register to this parameter's own
        // local slot. For pi >= MAX_REG_PARAMS, this parameter arrives on
        // the STACK instead, at `[rbp + 16 + 8*(pi - MAX_REG_PARAMS)]` --
        // `+16` clears the two real 8-byte values this function's own
        // `emit_prologue()` just pushed below the caller's stack-argument
        // tail (the `call` instruction's own return address, then this
        // prologue's `push rbp`), and each further stack-passed parameter
        // sits 8 bytes higher, in ascending index order -- the exact
        // layout the EXPR_CALL codegen's own reversed push loop was built
        // to leave behind (see that codegen's own Milestone 107 doc
        // comment for the caller-side derivation). Reads that caller-
        // supplied value into RAX via emit_mov_rax_rbp_off() (an EXISTING
        // encoding, completely unmodified -- it already supports a
        // positive disp8, this is simply the first call site that ever
        // passes it one) and then stores it into this parameter's own
        // local slot via emit_mov_rbp_off_rax() (also existing, also
        // unmodified) -- zero new x86_64 encodings needed for the
        // callee's receiving side, only emit_add_rsp_imm8() (new, see its
        // own doc comment) on the caller side. RAX is safe to clobber
        // here: this loop runs once per function, entirely before
        // gen_stmt_list() emits the function's own body, so no live value
        // survives across one parameter's copy into the next.
        let mut pi: u64 = 0;
        while pi < f.param_count as u64 {
            let off = -8 * (pi as i32 + 1);
            if pi < MAX_REG_PARAMS {
                let reg = unsafe { param_reg(pi) };
                unsafe { buf.emit_mov_rbp_off_reg(off, reg) };
            } else {
                let caller_off = 16 + 8 * (pi as i32 - MAX_REG_PARAMS as i32);
                unsafe { buf.emit_mov_rax_rbp_off(caller_off) };
                unsafe { buf.emit_mov_rbp_off_rax(off) };
            }
            pi += 1;
        }

        unsafe { loop_reset() }; // MILESTONE 87: fresh break/continue arena per function
        unsafe { goto_reset() };  // MILESTONE 92: fresh label/goto arena per function
        unsafe { gen_stmt_list(&mut buf, src_ptr, vars_ptr, nvars, funcs_ptr, nfuncs_total, pending_ptr, pending_count_ptr, this_fn_mode, f.body, NOT_IN_LOOP, 0, 0, f.is_char_return as u64) }?;
        if unsafe { goto_pend_n() } > 0 {
            let off = unsafe { core::ptr::read(core::ptr::addr_of!(GOTO_PEND_OFF) as *const u32) };
            return Err(CodeGenError::UndefinedLabel(off as u64));
        }

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
        // MILESTONE 96: re-parse CASE 1's exact source RIGHT HERE rather
        // than reusing CASE 1's `func1` pointer. Since Milestone 87 the
        // compile helpers call `heap_reset()` on entry (to bound the
        // ~60-compile self-test's per-compile leak inside the fixed
        // per-process heap), which rewinds the bump allocator between
        // CASE 1 and this point -- by now CASE 4/5/6/7's intervening
        // compiles have overwritten `func1`'s malloc'd AST nodes, so
        // `gen_function(func1, ...)` was codegen'ing a stale/empty
        // function and the ELF exited 0 instead of 42 (a real
        // regression from M87 through M95; `OVERALL_M69=FAIL`). CASE 8's
        // intent is unchanged -- CASE 1's exact source, compiled through
        // the Standalone (ELF64 + real `exec()`) backend -- it just no
        // longer leans on heap state that M87 legitimately rewinds. A
        // fresh `heap_reset()` first keeps CASE 8 on the same
        // "every compile entry resets" invariant as CASE 9/10 and every
        // later ELF-exec case. `src1_ptr` is a `&[u8]` static, never on
        // the heap, so it survives the reset.
        unsafe { heap_reset() };
        let case8_ok = match lex_and_parse(src1_ptr, src1_len) {
            Ok(func8) => match gen_function(src1_ptr, func8, CodegenMode::Standalone) {
                Ok(buf8) => {
                    let (elf8_ptr, elf8_len) = build_elf64_standalone(buf8.ptr, buf8.len, 0);
                    write_exec_and_check!(PATH8, elf8_ptr, elf8_len, 42)
                }
                Err(_) => {
                    w(b"  case8 gen_function(Standalone) returned Err unexpectedly\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case8 re-parse of CASE 1's source returned Err unexpectedly\n");
                false
            }
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
        // MILESTONE 96: regression fix -- `case8` above no longer reuses
        // CASE 1's `func1` AST pointer (dead since Milestone 87's
        // `heap_reset()` started rewinding the bump allocator between
        // compiles); it re-parses CASE 1's exact source in place. This
        // milestone's own check is that the fix actually took: `case8`
        // PASSES and `OVERALL_M69` is back to PASS. `OVERALL_M69` was
        // FAIL on every boot from Milestone 87 through Milestone 95
        // (committed boot logs `m87_*`..`m95_*` all show it); this is the
        // milestone that flips it back. No new grammar, no new codegen,
        // no new `cc.elf` feature -- a self-test correctness fix, in the
        // same spirit as Milestone 85 (SAR verification) closing an
        // on-the-books hole rather than adding surface.
        // ===============================================================
        let overall_m96 = case8_ok && overall_m69;
        write_check(b"case8_reparse_no_longer_leans_on_stale_func1_heap=", case8_ok);
        write_check(b"m96_overall_m69_regression_cleared=", overall_m96);
        w(b"OVERALL_M96=");
        w(if overall_m96 { b"PASS" } else { b"FAIL" });
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

        // -------------------------------------------------------------
        // MILESTONE 88: combined declaration-with-initializer,
        // `int i = e;` (desugared to `int i;` then `i = e;`).
        //
        // CASE 50 (in-process Callable path): two initializers, the
        // second referencing the first, plus a real expression in each.
        //   int main() {
        //       int x = 40 + 2;
        //       int y = x * 2;
        //       return y - 4;
        //   }
        // Hand-computed: x = 42, y = 84, return 80.
        // -------------------------------------------------------------
        const SRC50: &[u8] = b"int main() { int x = 40 + 2; int y = x * 2; return y - 4; }";
        let case50_result = compile_and_run_program_callable(SRC50.as_ptr() as u64, SRC50.len() as u64);
        w(b"  case50 (decl-with-initializer, in-process) returned=");
        if let Some(r) = case50_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 80)\n");
        let case50_ok = case50_result == Some(80);
        write_check(b"case50_decl_with_initializer_returns_80=", case50_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 51 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier): a combined decl-init as the `for`
        // loop's OWN init clause -- `for (int i = 1; ...)`, the shape
        // Milestone 86's `for` could not express (its init was an
        // assignment to an already-declared variable).
        //   int main() {
        //       int s = 0;
        //       for (int i = 1; i <= 5; i += 1) { s += i; }
        //       return s;
        //   }
        // Hand-computed: s = 1+2+3+4+5 = 15. Exercises the `for`-init
        // decl-init desugar chaining correctly ahead of the while.
        // -------------------------------------------------------------
        const SRC51: &[u8] =
            b"int main() { int s = 0; for (int i = 1; i <= 5; i += 1) { s += i; } return s; }";
        const PATH51: &[u8] = PATH8;
        let (elf51_ptr, elf51_len) = compile_program_standalone_elf(SRC51.as_ptr() as u64, SRC51.len() as u64);
        let case51_ok = if elf51_ptr == 0 {
            w(b"  case51 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH51, elf51_ptr, elf51_len, 15)
        };
        write_check(b"case51_real_elf_exec_for_int_i_init_returns_15=", case51_ok);

        w(b"\n");

        let overall_m88 = case50_ok && case51_ok;
        w(b"OVERALL_M88=");
        w(if overall_m88 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 89: the `%` (modulo) operator and its `%=` compound.
        //
        // CASE 52 (in-process Callable path): `%`, `/` and `%=` over
        // real variables, plus a decl-init.
        //   int main() {
        //       int a = 17;
        //       int b = 5;
        //       int r = a % b;      // 17 % 5 = 2
        //       r += a / b;         // + (17 / 5 = 3)  -> 5
        //       r %= 7;             // 5 % 7 = 5
        //       return r;
        //   }
        // Hand-computed: 5.
        // -------------------------------------------------------------
        const SRC52: &[u8] = b"int main() { int a = 17; int b = 5; int r = a % b; r += a / b; r %= 7; return r; }";
        let case52_result = compile_and_run_program_callable(SRC52.as_ptr() as u64, SRC52.len() as u64);
        w(b"  case52 (%, / and %= combined, in-process) returned=");
        if let Some(r) = case52_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 5)\n");
        let case52_ok = case52_result == Some(5);
        write_check(b"case52_modulo_and_compound_returns_5=", case52_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 53 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier): a precedence-regression test.
        // `%` is one level with `*` and `/`, left-associative.
        //   int main() { int r = 20 % 7 * 2; return r; }
        // Hand-computed (CORRECT: left-assoc, `%` and `*` same level):
        // (20 % 7) * 2 = 6 * 2 = 12. If `%` bound LOOSER than `*`, it
        // would be 20 % (7 * 2) = 20 % 14 = 6 -- a different,
        // distinguishable result.
        // -------------------------------------------------------------
        const SRC53: &[u8] = b"int main() { int r = 20 % 7 * 2; return r; }";
        const PATH53: &[u8] = PATH8;
        let (elf53_ptr, elf53_len) = compile_program_standalone_elf(SRC53.as_ptr() as u64, SRC53.len() as u64);
        let case53_ok = if elf53_ptr == 0 {
            w(b"  case53 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH53, elf53_ptr, elf53_len, 12)
        };
        write_check(b"case53_real_elf_exec_modulo_precedence_returns_12=", case53_ok);

        w(b"\n");

        let overall_m89 = case52_ok && case53_ok;
        w(b"OVERALL_M89=");
        w(if overall_m89 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 90: the conditional (ternary) operator `? :`.
        //
        // CASE 54 (in-process Callable path): the operator both ways,
        // over real variables, its result stored and reused.
        //   int main() {
        //       int a = 7;
        //       int b = 3;
        //       int m = a > b ? a : b;   // true  -> 7
        //       int n = a < b ? a : b;   // false -> 3
        //       return m * 10 + n;
        //   }
        // Hand-computed: 73.
        // -------------------------------------------------------------
        const SRC54: &[u8] = b"int main() { int a = 7; int b = 3; int m = a > b ? a : b; int n = a < b ? a : b; return m * 10 + n; }";
        let case54_result = compile_and_run_program_callable(SRC54.as_ptr() as u64, SRC54.len() as u64);
        w(b"  case54 (ternary both ways, in-process) returned=");
        if let Some(r) = case54_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 73)\n");
        let case54_ok = case54_result == Some(73);
        write_check(b"case54_ternary_both_ways_returns_73=", case54_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 55 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier): ternary as a SUB-EXPRESSION
        // inside real arithmetic, with a `*` in one branch and a `%` in
        // another (Milestone 89), and two ternaries added together.
        //   int main() {
        //       int a = 5;
        //       int r = (a > 0 ? a * 2 : 0 - a) + (a > 10 ? 999 : a % 3);
        //       return r;
        //   }
        // Hand-computed: a>0 -> a*2 = 10; a>10 false -> a%3 = 5%3 = 2;
        // 10 + 2 = 12. Only the taken branch of each `?:` contributes;
        // a wrong branch pick gives -5+2, 10+999, etc. -- all different.
        // -------------------------------------------------------------
        const SRC55: &[u8] =
            b"int main() { int a = 5; int r = (a > 0 ? a * 2 : 0 - a) + (a > 10 ? 999 : a % 3); return r; }";
        const PATH55: &[u8] = PATH8;
        let (elf55_ptr, elf55_len) = compile_program_standalone_elf(SRC55.as_ptr() as u64, SRC55.len() as u64);
        let case55_ok = if elf55_ptr == 0 {
            w(b"  case55 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH55, elf55_ptr, elf55_len, 12)
        };
        write_check(b"case55_real_elf_exec_ternary_subexpression_returns_12=", case55_ok);

        w(b"\n");

        let overall_m90 = case54_ok && case55_ok;
        w(b"OVERALL_M90=");
        w(if overall_m90 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 91: `switch` / `case` / `default`.
        //
        // CASE 56 (in-process Callable path): a match that FALLS THROUGH
        // into the next case, a `break`, a `+=` in a case body, and a
        // `default` that is not taken.
        //   int main() {
        //       int x = 2;
        //       int r = 0;
        //       switch (x) {
        //           case 1: r = 10; break;
        //           case 2: r = 20;           // no break -> falls through
        //           case 3: r += 5; break;
        //           default: r = 99;
        //       }
        //       return r;
        //   }
        // Hand-computed: x == 2 -> r = 20, fall into case 3 -> r += 5 ->
        // 25, break. return 25.
        // -------------------------------------------------------------
        const SRC56: &[u8] = b"int main() { int x = 2; int r = 0; switch (x) { case 1: r = 10; break; case 2: r = 20; case 3: r += 5; break; default: r = 99; } return r; }";
        let case56_result = compile_and_run_program_callable(SRC56.as_ptr() as u64, SRC56.len() as u64);
        w(b"  case56 (switch: match, fall-through, break, in-process) returned=");
        if let Some(r) = case56_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 25)\n");
        let case56_ok = case56_result == Some(25);
        write_check(b"case56_switch_fallthrough_and_break_returns_25=", case56_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 57 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier): a `switch` INSIDE a `for` loop.
        // Proves the switch's own `break` jumps to the switch end and
        // does NOT escape the enclosing loop, that `default` runs on a
        // no-match, and that the loop keeps iterating after each switch.
        //   int main() {
        //       int total = 0;
        //       int i = 0;
        //       for (i = 0; i < 6; i += 1) {
        //           switch (i) {
        //               case 0: total += 1; break;
        //               case 3: total += 10; break;
        //               default: total += 30;
        //           }
        //       }
        //       return total;
        //   }
        // Hand-computed: i=0 -> +1 (=1); i=1 -> default +30 (=31); i=2
        // -> +30 (=61); i=3 -> +10 (=71); i=4 -> +30 (=101); i=5 -> +30
        // (=131). return 131. If the switch's `break` broke the loop,
        // total would be 1; if `default` never ran, it would be 11.
        // -------------------------------------------------------------
        const SRC57: &[u8] =
            b"int main() { int total = 0; int i = 0; for (i = 0; i < 6; i += 1) { switch (i) { case 0: total += 1; break; case 3: total += 10; break; default: total += 30; } } return total; }";
        const PATH57: &[u8] = PATH8;
        let (elf57_ptr, elf57_len) = compile_program_standalone_elf(SRC57.as_ptr() as u64, SRC57.len() as u64);
        let case57_ok = if elf57_ptr == 0 {
            w(b"  case57 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH57, elf57_ptr, elf57_len, 131)
        };
        write_check(b"case57_real_elf_exec_switch_in_for_returns_131=", case57_ok);

        w(b"\n");

        let overall_m91 = case56_ok && case57_ok;
        w(b"OVERALL_M91=");
        w(if overall_m91 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 92: `goto` and labels.
        //
        // CASE 58 (in-process Callable path): a BACKWARD goto forming a
        // loop, and a FORWARD goto skipping a statement.
        //   int main() {
        //       int i = 0;
        //       int s = 0;
        //   loop:
        //       s += i;
        //       i += 1;
        //       if (i < 5) { goto loop; }
        //       goto done;
        //       s = 999;          // unreachable -- skipped by `goto done`
        //   done:
        //       return s;
        //   }
        // Hand-computed: s = 0+1+2+3+4 = 10; the `goto done` skips
        // `s = 999`. return 10.
        // -------------------------------------------------------------
        const SRC58: &[u8] = b"int main() { int i = 0; int s = 0; loop: s += i; i += 1; if (i < 5) { goto loop; } goto done; s = 999; done: return s; }";
        let case58_result = compile_and_run_program_callable(SRC58.as_ptr() as u64, SRC58.len() as u64);
        w(b"  case58 (backward + forward goto, in-process) returned=");
        if let Some(r) = case58_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 10)\n");
        let case58_ok = case58_result == Some(10);
        write_check(b"case58_goto_backward_and_forward_returns_10=", case58_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 59: a deliberate SEMANTIC error -- `goto` to a label that
        // is never defined must be Err(CodeGenError::UndefinedLabel),
        // the same "prove the new Err path is real" discipline CASE 7
        // (UndeclaredVariable) and CASE 49 (BreakOutsideLoop) established.
        //   int main() { goto nowhere; return 1; }
        // -------------------------------------------------------------
        const SRC59: &[u8] = b"int main() { goto nowhere; return 1; }";
        let src59_ptr = SRC59.as_ptr() as u64;
        let src59_len = SRC59.len() as u64;
        let case59_ok = match lex_and_parse(src59_ptr, src59_len) {
            Ok(func59) => match gen_function(src59_ptr, func59, CodegenMode::Callable) {
                Err(CodeGenError::UndefinedLabel(_)) => {
                    w(b"  case59 real codegen error -- goto to an undefined label\n");
                    true
                }
                _ => {
                    w(b"  case59 expected Err(UndefinedLabel), got something else\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case59 lex_and_parse() returned Err unexpectedly (source should lex+parse fine)\n");
                false
            }
        };
        write_check(b"case59_goto_undefined_label_is_UndefinedLabel=", case59_ok);

        w(b"\n");

        // -------------------------------------------------------------
        // CASE 60 (real on-disk-ELF + kernel exec()+wait() path -- this
        // milestone's strongest tier): `goto` as an early-exit that
        // jumps clear OUT of a `for` loop (something `break` alone
        // cannot do -- it also skips the post-loop statement).
        //   int main() {
        //       int r = 0;
        //       int i = 0;
        //       for (i = 0; i < 10; i += 1) {
        //           if (i == 7) { goto found; }
        //           r += 1;
        //       }
        //       r = 100;         // skipped once the goto fires
        //   found:
        //       return r;
        //   }
        // Hand-computed: i=0..6 -> r += 1 (=7); i == 7 -> goto found,
        // skipping `r = 100`. return 7. If the goto did nothing the
        // loop would finish and r would be 100.
        // -------------------------------------------------------------
        const SRC60: &[u8] =
            b"int main() { int r = 0; int i = 0; for (i = 0; i < 10; i += 1) { if (i == 7) { goto found; } r += 1; } r = 100; found: return r; }";
        const PATH60: &[u8] = PATH8;
        let (elf60_ptr, elf60_len) = compile_program_standalone_elf(SRC60.as_ptr() as u64, SRC60.len() as u64);
        let case60_ok = if elf60_ptr == 0 {
            w(b"  case60 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH60, elf60_ptr, elf60_len, 7)
        };
        write_check(b"case60_real_elf_exec_goto_out_of_for_returns_7=", case60_ok);

        w(b"\n");

        let overall_m92 = case58_ok && case59_ok && case60_ok;
        w(b"OVERALL_M92=");
        w(if overall_m92 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 93: MAX_FUNCS 4 -> 8, MAX_TOKENS 128 -> 256.
        //
        // CASE 61 (in-process Callable path): SIX functions in one
        // program -- impossible to compile under the old cap -- with a
        // real forward call (`poly` calls `lin`, defined after it).
        //   int add(int a, int b) { return a + b; }
        //   int mul(int a, int b) { return a * b; }
        //   int neg(int x) { return 0 - x; }
        //   int poly(int x) { return add(mul(x, x), lin(x)); }
        //   int lin(int x) { return add(mul(3, x), 7); }        // x^2 + 3x + 7, split
        //   int main() { return poly(4) + neg(5); }
        // Hand-computed: lin(4) = 3*4 + 7 = 19; poly(4) = 4*4 + 19 = 35;
        // neg(5) = -5; main = 35 + (-5) = 30. Any function-table or
        // forward-patch bug at 6 > 4 functions blows this.
        // -------------------------------------------------------------
        const SRC61: &[u8] = b"int add(int a, int b) { return a + b; } int mul(int a, int b) { return a * b; } int neg(int x) { return 0 - x; } int poly(int x) { return add(mul(x, x), lin(x)); } int lin(int x) { return add(mul(3, x), 7); } int main() { return poly(4) + neg(5); }";
        let case61_result = compile_and_run_program_callable(SRC61.as_ptr() as u64, SRC61.len() as u64);
        w(b"  case61 (six functions + a forward call, in-process) returned=");
        if let Some(r) = case61_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 30)\n");
        let case61_ok = case61_result == Some(30);
        write_check(b"case61_six_functions_forward_call_returns_30=", case61_ok);

        w(b"\n");

        let overall_m93 = case61_ok;
        w(b"OVERALL_M93=");
        w(if overall_m93 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 94: `//` line and `/* */` block comments (lexer
        // only). CASE 62 (in-process Callable path): a program peppered
        // with both comment styles, including a block comment BETWEEN
        // two tokens, a line comment ending at a real '\n' followed by
        // more code, and a `/=` right after a `//` (proving the comment
        // check does not eat the `/=` operator).
        //   int main() {
        //       int a = 100; /* set */ a /= 4; // now 25
        //       int b = a; /* keep */ return b + 5;
        //   }
        // Hand-computed: a = 100 / 4 = 25; b = 25; return 30. If any
        // comment leaked into the token stream the program would fail to
        // compile or return a different value.
        // -------------------------------------------------------------
        const SRC62: &[u8] = b"int main() { int a = 100; /* set */ a /= 4; // now 25\n int b = a; /* keep */ return b + 5; }";
        let case62_result = compile_and_run_program_callable(SRC62.as_ptr() as u64, SRC62.len() as u64);
        w(b"  case62 (// and /* */ comments, in-process) returned=");
        if let Some(r) = case62_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 30)\n");
        let case62_ok = case62_result == Some(30);
        write_check(b"case62_line_and_block_comments_returns_30=", case62_ok);

        w(b"\n");

        // CASE 63: an UNTERMINATED block comment is a real
        // LexError -- the same "prove the Err path is real" discipline
        // CASE 3 (a bad lex byte) already established.
        //   int main() { return 1; /* never closed
        const SRC63: &[u8] = b"int main() { return 1; /* never closed";
        let case63_ok = match lex_and_parse(SRC63.as_ptr() as u64, SRC63.len() as u64) {
            Err(_) => {
                w(b"  case63 unterminated block comment rejected, as expected\n");
                true
            }
            Ok(_) => {
                w(b"  case63 expected a LexError for the unterminated /* comment, got Ok\n");
                false
            }
        };
        write_check(b"case63_unterminated_block_comment_is_a_lex_error=", case63_ok);

        w(b"\n");

        let overall_m94 = case62_ok && case63_ok;
        w(b"OVERALL_M94=");
        w(if overall_m94 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 95: 0x/0X hex and 0b/0B binary integer literals
        // (lexer only). CASE 64 (in-process Callable path): a program
        // that mixes all three bases in one expression.
        //   int main() {
        //       int a = 0xFF;    // 255
        //       int b = 0b1010;  //  10
        //       int c = 0x10;    //  16
        //       return a + b + c;
        //   }
        // Hand-computed: 255 + 10 + 16 = 281. A wrong base (e.g. hex
        // parsed as decimal, or the 'b' eaten as an identifier) would
        // fail to compile or return something other than 281.
        // -------------------------------------------------------------
        const SRC64: &[u8] = b"int main() { int a = 0xFF; int b = 0b1010; int c = 0x10; return a + b + c; }";
        let case64_result = compile_and_run_program_callable(SRC64.as_ptr() as u64, SRC64.len() as u64);
        w(b"  case64 (0x hex + 0b binary literals, in-process) returned=");
        if let Some(r) = case64_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 281)\n");
        let case64_ok = case64_result == Some(281);
        write_check(b"case64_hex_and_binary_literals_return_281=", case64_ok);

        w(b"\n");

        // CASE 65: a bare "0x" with no hex digit is a real LexError,
        // the same Err-path discipline CASE 3 / CASE 63 established.
        //   int main() { return 0x; }
        const SRC65: &[u8] = b"int main() { return 0x; }";
        let case65_ok = match lex_and_parse(SRC65.as_ptr() as u64, SRC65.len() as u64) {
            Err(_) => {
                w(b"  case65 empty hex literal \"0x\" rejected, as expected\n");
                true
            }
            Ok(_) => {
                w(b"  case65 expected a LexError for \"0x\" with no digits, got Ok\n");
                false
            }
        };
        write_check(b"case65_empty_hex_literal_is_a_lex_error=", case65_ok);

        w(b"\n");

        let overall_m95 = case64_ok && case65_ok;
        w(b"OVERALL_M95=");
        w(if overall_m95 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 97: character literals (lexer only -- a `'c'` lexes
        // to an ordinary TOK_INTLIT). CASE 66 (in-process Callable
        // path): plain chars, an arithmetic difference of two chars, and
        // two escapes.
        //   int main() {
        //       int a = 'A';         // 65
        //       int b = 'a' - 'A';   // 32
        //       int nl = '\n';       // 10
        //       int z = '\0';        //  0
        //       return a + b + nl + z;
        //   }
        // Hand-computed: 65 + 32 + 10 + 0 = 107. A char literal parsed
        // as anything other than its byte value would miss 107.
        // -------------------------------------------------------------
        const SRC66: &[u8] = b"int main() { int a = 'A'; int b = 'a' - 'A'; int nl = '\\n'; int z = '\\0'; return a + b + nl + z; }";
        let case66_result = compile_and_run_program_callable(SRC66.as_ptr() as u64, SRC66.len() as u64);
        w(b"  case66 (char literals + escapes, in-process) returned=");
        if let Some(r) = case66_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 107)\n");
        let case66_ok = case66_result == Some(107);
        write_check(b"case66_char_literals_and_escapes_return_107=", case66_ok);

        w(b"\n");

        // CASE 67: a multi-character constant 'ab' is a real LexError
        // (the byte after the first char is not the closing quote) --
        // same Err-path discipline as CASE 3 / 63 / 65.
        //   int main() { return 'ab'; }
        const SRC67: &[u8] = b"int main() { return 'ab'; }";
        let case67_ok = match lex_and_parse(SRC67.as_ptr() as u64, SRC67.len() as u64) {
            Err(_) => {
                w(b"  case67 multi-character constant 'ab' rejected, as expected\n");
                true
            }
            Ok(_) => {
                w(b"  case67 expected a LexError for the multi-char constant 'ab', got Ok\n");
                false
            }
        };
        write_check(b"case67_multichar_constant_is_a_lex_error=", case67_ok);

        w(b"\n");

        let overall_m97 = case66_ok && case67_ok;
        w(b"OVERALL_M97=");
        w(if overall_m97 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 98: unary `+` (a parse-time no-op -- no AST node, no
        // codegen). CASE 68 (in-process Callable path): `+` on a
        // literal, on a variable, and next to `*`.
        //   int main() {
        //       int x = +5;
        //       return +x + +3 * +2;
        //   }
        // Hand-computed: x = 5; +3 * +2 = 6; 5 + 6 = 11. If unary `+`
        // were mis-parsed as a binary `+` (missing left operand) this
        // would fail to compile.
        // -------------------------------------------------------------
        const SRC68: &[u8] = b"int main() { int x = +5; return +x + +3 * +2; }";
        let case68_result = compile_and_run_program_callable(SRC68.as_ptr() as u64, SRC68.len() as u64);
        w(b"  case68 (unary + on literal/var/next-to-*) returned=");
        if let Some(r) = case68_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 11)\n");
        let case68_ok = case68_result == Some(11);
        write_check(b"case68_unary_plus_is_a_noop_returns_11=", case68_ok);

        w(b"\n");

        // CASE 69: unary `+` composes with unary `-` through the same
        // recursion, and is a true no-op (does not force sign).
        //   int main() { int x = 10; return -+-x; }
        // Hand-computed: -x = -10; +(-10) = -10; -(-10) = 10.
        const SRC69: &[u8] = b"int main() { int x = 10; return -+-x; }";
        let case69_result = compile_and_run_program_callable(SRC69.as_ptr() as u64, SRC69.len() as u64);
        w(b"  case69 (-+-x, unary + composes with unary -) returned=");
        if let Some(r) = case69_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 10)\n");
        let case69_ok = case69_result == Some(10);
        write_check(b"case69_unary_plus_composes_with_unary_minus_returns_10=", case69_ok);

        w(b"\n");

        let overall_m98 = case68_ok && case69_ok;
        w(b"OVERALL_M98=");
        w(if overall_m98 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // ===============================================================
        // MILESTONE 99: `do { body } while (cond);` -- the third loop
        // form. Real codegen (STMT_DOWHILE, body-first, condition at the
        // bottom, backward jump), reusing STMT_WHILE's LOOP_BRK/LOOP_CONT
        // arena save/restore and the `for`-style forward-patched
        // `continue`. Three cases: 70 in-process (loops N times, sums),
        // 71 in-process (`break` + `continue` bind to the do-while, and
        // `continue` targets the bottom condition), 72 on-disk ELF +
        // real exec()/wait() (factorial).
        // ===============================================================

        // CASE 70: a do-while that iterates and accumulates.
        //   int main() {
        //       int i = 0; int sum = 0;
        //       do { sum = sum + i; i = i + 1; } while (i < 5);
        //       return sum;
        //   }
        // Hand-computed: i runs 0,1,2,3,4 (the guard `i < 5` fails at
        // i == 5, AFTER the body that made it 5). sum = 0+1+2+3+4 = 10.
        const SRC70: &[u8] = b"int main() { int i = 0; int sum = 0; do { sum = sum + i; i = i + 1; } while (i < 5); return sum; }";
        let case70_result = compile_and_run_program_callable(SRC70.as_ptr() as u64, SRC70.len() as u64);
        w(b"  case70 (do-while sum 0..4, in-process) returned=");
        if let Some(r) = case70_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 10)\n");
        let case70_ok = case70_result == Some(10);
        write_check(b"case70_do_while_sum_returns_10=", case70_ok);

        w(b"\n");

        // CASE 71: `break` and `continue` inside a do-while.
        //   int main() {
        //       int i = 0; int sum = 0;
        //       do {
        //           i = i + 1;
        //           if (i == 3) { continue; }
        //           if (i == 6) { break; }
        //           sum = sum + i;
        //       } while (i < 100);
        //       return sum;
        //   }
        // Hand-computed: i=1 -> sum 1; i=2 -> sum 3; i=3 -> continue
        // (condition still checked, 3 < 100 true); i=4 -> sum 7; i=5 ->
        // sum 12; i=6 -> break. return 12. A `continue` that jumped to
        // the top (skipping the guard) would still work here, but one
        // that skipped the increment would loop forever; a `continue`
        // that skipped the guard AND a `break` that fell through would
        // both give the wrong sum.
        const SRC71: &[u8] = b"int main() { int i = 0; int sum = 0; do { i = i + 1; if (i == 3) { continue; } if (i == 6) { break; } sum = sum + i; } while (i < 100); return sum; }";
        let case71_result = compile_and_run_program_callable(SRC71.as_ptr() as u64, SRC71.len() as u64);
        w(b"  case71 (do-while with break + continue, in-process) returned=");
        if let Some(r) = case71_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 12)\n");
        let case71_ok = case71_result == Some(12);
        write_check(b"case71_do_while_break_and_continue_returns_12=", case71_ok);

        w(b"\n");

        // CASE 72: a do-while factorial through the on-disk ELF + real
        // exec()/wait() path.
        //   int main() {
        //       int n = 5; int f = 1;
        //       do { f = f * n; n = n - 1; } while (n > 0);
        //       return f;
        //   }
        // Hand-computed: 5*4*3*2*1 = 120; the guard `n > 0` fails at
        // n == 0, AFTER the body multiplied by 1. Exit code 120.
        const SRC72: &[u8] = b"int main() { int n = 5; int f = 1; do { f = f * n; n = n - 1; } while (n > 0); return f; }";
        const PATH72: &[u8] = PATH8;
        let (elf72_ptr, elf72_len) = compile_program_standalone_elf(SRC72.as_ptr() as u64, SRC72.len() as u64);
        let case72_ok = if elf72_ptr == 0 {
            w(b"  case72 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH72, elf72_ptr, elf72_len, 120)
        };
        write_check(b"case72_real_elf_exec_do_while_factorial_returns_120=", case72_ok);

        w(b"\n");

        let overall_m99 = case70_ok && case71_ok && case72_ok;
        w(b"OVERALL_M99=");
        w(if overall_m99 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 101: multiple comma-separated declarators after one
        // `int` (parser-only). This is also the end-to-end proof that
        // Milestone 100's extra stack pages are real and usable -- this
        // exact change stack-overflowed cc.elf's single 4 KiB page on
        // its first attempt (the reverted "M100"); it now compiles and
        // runs inside the 32 KiB stack with room to spare.
        // CASE 73 (in-process): a mix of initialized and uninitialized
        // declarators.
        //   int main() { int a = 1, b, c = 3; b = a + c; return a + b + c; }
        // Hand-computed: a = 1, c = 3, b = 4; 1 + 4 + 3 = 8.
        // -------------------------------------------------------------
        const SRC73: &[u8] = b"int main() { int a = 1, b, c = 3; b = a + c; return a + b + c; }";
        let case73_result = compile_and_run_program_callable(SRC73.as_ptr() as u64, SRC73.len() as u64);
        w(b"  case73 (mixed multi-declarator, in-process) returned=");
        if let Some(r) = case73_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 8)\n");
        let case73_ok = case73_result == Some(8);
        write_check(b"case73_multi_declarator_mixed_init_returns_8=", case73_ok);

        w(b"\n");

        // CASE 74: a later declarator's initializer refers to an earlier
        // declarator in the SAME list.
        //   int main() { int x = 5, y = x * 2, z = y + 1; return z; }
        // Hand-computed: x = 5, y = 10, z = 11.
        const SRC74: &[u8] = b"int main() { int x = 5, y = x * 2, z = y + 1; return z; }";
        let case74_result = compile_and_run_program_callable(SRC74.as_ptr() as u64, SRC74.len() as u64);
        w(b"  case74 (declarator initializer uses earlier declarator) returned=");
        if let Some(r) = case74_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 11)\n");
        let case74_ok = case74_result == Some(11);
        write_check(b"case74_multi_declarator_sequential_dependency_returns_11=", case74_ok);

        w(b"\n");

        // CASE 75: a trailing comma with no declarator after it is a
        // real ParseError (the `expect(TOK_IDENT)` after the `,` fails).
        //   int main() { int a, ; return a; }
        const SRC75: &[u8] = b"int main() { int a, ; return a; }";
        let case75_ok = match lex_and_parse(SRC75.as_ptr() as u64, SRC75.len() as u64) {
            Err(_) => {
                w(b"  case75 dangling comma in declarator list rejected, as expected\n");
                true
            }
            Ok(_) => {
                w(b"  case75 expected a ParseError for the dangling `,` in `int a, ;`, got Ok\n");
                false
            }
        };
        write_check(b"case75_dangling_declarator_comma_is_a_parse_error=", case75_ok);

        w(b"\n");

        let overall_m101 = case73_ok && case74_ok && case75_ok;
        w(b"OVERALL_M101=");
        w(if overall_m101 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 102: a real second type, `char`, alongside `int` --
        // this subset's first genuine width distinction. Every stack
        // slot stays a full 8 bytes (unchanged layout); only a `char`
        // variable's READ narrows, via a real `movsx rax, byte
        // [rbp+off]` (emit_movsx_rax_rbp_off_byte()) instead of the
        // full-width load -- see that function's own doc comment for
        // why storing full-width and narrowing only on read is
        // correct, not a shortcut, and gives real 8-bit truncation AND
        // real signed-char sign-extension for free, with zero
        // store-side codegen changes.
        // CASE 76 (in-process): basic `char` declaration, read, and
        // arithmetic.
        //   int main() { char c = 65; int i = c + 1; return i; }
        // Hand-computed: c = 65 ('A'), i = 66.
        // -------------------------------------------------------------
        const SRC76: &[u8] = b"int main() { char c = 65; int i = c + 1; return i; }";
        let case76_result = compile_and_run_program_callable(SRC76.as_ptr() as u64, SRC76.len() as u64);
        w(b"  case76 (basic char decl + arithmetic, in-process) returned=");
        if let Some(r) = case76_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 66)\n");
        let case76_ok = case76_result == Some(66);
        write_check(b"case76_char_decl_and_arithmetic_returns_66=", case76_ok);

        w(b"\n");

        // CASE 77: an out-of-range initializer truncates to a real
        // 8-bit value on the very next READ (no separate truncate-on-
        // store codegen exists -- see this milestone's own top doc
        // comment). 321 = 0x141; low byte 0x41 = 65, and 65 < 128 so
        // sign-extension doesn't matter here (see CASE 78 for the
        // negative case).
        //   int main() { char c = 321; return c; }
        // Hand-computed: 321 & 0xFF = 65.
        const SRC77: &[u8] = b"int main() { char c = 321; return c; }";
        let case77_result = compile_and_run_program_callable(SRC77.as_ptr() as u64, SRC77.len() as u64);
        w(b"  case77 (char truncates an out-of-range initializer, in-process) returned=");
        if let Some(r) = case77_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 65)\n");
        let case77_ok = case77_result == Some(65);
        write_check(b"case77_char_truncates_out_of_range_initializer_returns_65=", case77_ok);

        w(b"\n");

        // CASE 78: real signed-char sign-extension, proven via a
        // comparison rather than by returning a negative value
        // directly (this subset's `int main()` return-code tests are
        // all non-negative by convention -- a comparison's own 0/1
        // result stays in that convention while still exercising the
        // real negative value). 200 = 0xC8; as a signed byte that's
        // -56 (200 - 256), matching a real x86_64 SysV C compiler's
        // default (signed) `char`.
        //   int main() { char c = 200; return c == -56; }
        // Hand-computed: c reads back as -56, so c == -56 is 1.
        const SRC78: &[u8] = b"int main() { char c = 200; return c == -56; }";
        let case78_result = compile_and_run_program_callable(SRC78.as_ptr() as u64, SRC78.len() as u64);
        w(b"  case78 (char is signed -- 200 sign-extends to -56, in-process) returned=");
        if let Some(r) = case78_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        let case78_ok = case78_result == Some(1);
        write_check(b"case78_char_is_signed_200_sign_extends_to_neg56=", case78_ok);

        w(b"\n");

        // CASE 79: `char` used as an ordinary `while`-loop counter --
        // real on-disk-ELF + fork()/exec()/wait() path, the same
        // strongest verification tier every milestone since 69 reaches
        // for. This case ORIGINALLY shipped on the in-process Callable
        // path instead: while developing Milestone 102, adding a
        // FOURTH real fork()/exec()/wait() self-test cycle here (on top
        // of this suite's already-large existing count) reproducibly
        // made the KERNEL itself panic later in the same boot --
        // `tasks::run_preemption_demo()`'s first 16 KiB worker-stack
        // allocation failing against the kernel's own fixed 100 KiB
        // heap. Milestone 102 disclosed that bug honestly rather than
        // paper over it, and kept this case on the in-process path as a
        // real, disclosed scope decision. MILESTONE 105 closes that gap
        // now that Milestone 104 actually root-caused and fixed it
        // (real evidence-gated `heap_guard::admit()` + `HEAP_SIZE`
        // raised 100 KiB -> 256 KiB, see that milestone's own README
        // entry) -- this case now runs through
        // `compile_program_standalone_elf()` + `write_exec_and_check!`
        // exactly like CASE 72 above, a real fifth exec() cycle added to
        // this suite on top of the four Milestone 102 already found
        // right at the old budget's edge, verified against a real boot
        // rather than assumed safe from the fix's own numbers alone.
        //   int main() { char c = 10; int sum = 0;
        //                while (c > 0) { sum = sum + c; c = c - 1; }
        //                return sum; }
        // Hand-computed: 10+9+...+1 = 55 (fits in a real exit code's
        // 0-255 range, so truncating the in-process return value down to
        // wait()'s exit-code convention changes nothing here).
        const SRC79: &[u8] = b"int main() { char c = 10; int sum = 0; while (c > 0) { sum = sum + c; c = c - 1; } return sum; }";
        const PATH79: &[u8] = PATH8;
        let (elf79_ptr, elf79_len) = compile_program_standalone_elf(SRC79.as_ptr() as u64, SRC79.len() as u64);
        let case79_ok = if elf79_ptr == 0 {
            w(b"  case79 compile_program_standalone_elf failed\n");
            false
        } else {
            write_exec_and_check!(PATH79, elf79_ptr, elf79_len, 55)
        };
        write_check(b"case79_char_while_loop_counter_returns_55=", case79_ok);

        w(b"\n");

        let overall_m102 = case76_ok && case77_ok && case78_ok && case79_ok;
        w(b"OVERALL_M102=");
        w(if overall_m102 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 103: `char` as a function PARAMETER type and a
        // function RETURN type -- closing the gap Milestone 102's own
        // "still genuinely open" note named. `char` parameters reuse
        // Milestone 102's narrowed-read mechanism with ZERO new codegen
        // (a parameter's own `is_char`, now recorded by parse_params(),
        // flows straight into its VarSlot via collect_vars_for_function()
        // -- the prologue's own register-spill, emit_mov_rbp_off_reg(),
        // is completely untouched, still spilling the full incoming
        // register). A `char` RETURN type is the one genuinely new piece
        // of codegen: emit_movsx_rax_al(), new, narrows RAX in place
        // right before the epilogue -- see that method's own doc
        // comment for why a return value (which lives in a register, not
        // a stack slot) needs this and a local/parameter doesn't.
        // -------------------------------------------------------------
        // CASE 80 (in-process): a basic `char` parameter, read and used
        // in arithmetic.
        //   int use_char_param(char c) { return c + 1; }
        //   int main() { return use_char_param(65); }
        // Hand-computed: 65 + 1 = 66.
        const SRC80: &[u8] = b"int use_char_param(char c) { return c + 1; } int main() { return use_char_param(65); }";
        let case80_result = compile_and_run_program_callable(SRC80.as_ptr() as u64, SRC80.len() as u64);
        w(b"  case80 (char parameter, basic read + arithmetic, in-process) returned=");
        if let Some(r) = case80_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 66)\n");
        let case80_ok = case80_result == Some(66);
        write_check(b"case80_char_parameter_basic_returns_66=", case80_ok);

        w(b"\n");

        // CASE 81: an out-of-range ARGUMENT truncates to a real 8-bit
        // value on the callee's own first read of its `char` parameter
        // -- the exact same free truncation CASE 77 already proved for
        // a local, now proven across the real calling-convention
        // boundary (the caller passes the full, untruncated 321 in a
        // register; the callee's own prologue spills that full value
        // unchanged; only the parameter's own narrowed READ inside the
        // callee's body truncates it).
        //   int trunc_param(char c) { return c; }
        //   int main() { return trunc_param(321); }
        // Hand-computed: 321 & 0xFF = 65.
        const SRC81: &[u8] = b"int trunc_param(char c) { return c; } int main() { return trunc_param(321); }";
        let case81_result = compile_and_run_program_callable(SRC81.as_ptr() as u64, SRC81.len() as u64);
        w(b"  case81 (char parameter truncates an out-of-range argument, in-process) returned=");
        if let Some(r) = case81_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 65)\n");
        let case81_ok = case81_result == Some(65);
        write_check(b"case81_char_parameter_truncates_out_of_range_argument_returns_65=", case81_ok);

        w(b"\n");

        // CASE 82: a real `char` RETURN type actually narrows the
        // returned value -- the one genuinely new codegen this
        // milestone adds (emit_movsx_rax_al()). Without it this case
        // would return the untruncated `300`; with it, `300 & 0xFF`.
        //   char get_truncated() { return 300; }
        //   int main() { return get_truncated(); }
        // Hand-computed: 300 & 0xFF = 44.
        const SRC82: &[u8] = b"char get_truncated() { return 300; } int main() { return get_truncated(); }";
        let case82_result = compile_and_run_program_callable(SRC82.as_ptr() as u64, SRC82.len() as u64);
        w(b"  case82 (char return type truncates an out-of-range value, in-process) returned=");
        if let Some(r) = case82_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 44)\n");
        let case82_ok = case82_result == Some(44);
        write_check(b"case82_char_return_type_truncates_returns_44=", case82_ok);

        w(b"\n");

        // CASE 83: `char` parameter and `char` return type CHAINED --
        // a real multi-function integration proof, not just two
        // isolated single-function cases.
        //   char add_one(char c) { return c + 1; }
        //   int main() { char x = 40; char y = add_one(x);
        //                return y + 2; }
        // Hand-computed: x=40, add_one(40)=41, y=41, y+2=43.
        const SRC83: &[u8] = b"char add_one(char c) { return c + 1; } int main() { char x = 40; char y = add_one(x); return y + 2; }";
        let case83_result = compile_and_run_program_callable(SRC83.as_ptr() as u64, SRC83.len() as u64);
        w(b"  case83 (char parameter + char return chained, in-process) returned=");
        if let Some(r) = case83_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 43)\n");
        let case83_ok = case83_result == Some(43);
        write_check(b"case83_char_param_and_return_chained_returns_43=", case83_ok);

        w(b"\n");

        // CASE 84: real SIGNED sign-extension across the parameter
        // boundary specifically -- the direct parameter-side analogue
        // of CASE 78 (which proved this for a local). Proven via a
        // comparison, same convention as CASE 78, since this suite's
        // own `int main()` return-code convention stays non-negative.
        //   int check_param_sign(char c) { return c == -56; }
        //   int main() { return check_param_sign(200); }
        // Hand-computed: 200 passed in full; the callee's own read of
        // its `char` parameter sign-extends 200's low byte to -56.
        const SRC84: &[u8] = b"int check_param_sign(char c) { return c == -56; } int main() { return check_param_sign(200); }";
        let case84_result = compile_and_run_program_callable(SRC84.as_ptr() as u64, SRC84.len() as u64);
        w(b"  case84 (char parameter is signed -- 200 sign-extends to -56, in-process) returned=");
        if let Some(r) = case84_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        let case84_ok = case84_result == Some(1);
        write_check(b"case84_char_parameter_is_signed_returns_1=", case84_ok);

        w(b"\n");

        let overall_m103 = case80_ok && case81_ok && case82_ok && case83_ok && case84_ok;
        w(b"OVERALL_M103=");
        w(if overall_m103 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 107: raises MAX_PARAMS 4 -> 8 via real stack-passed
        // argument codegen -- the first time this codegen has ever
        // needed to place an argument/parameter anywhere other than a
        // register. EXPR_CALL's own call-site codegen now pushes
        // arguments in REVERSE index order (so ascending pop order lands
        // the first MAX_REG_PARAMS (4) of them in RDI/RSI/RDX/RCX,
        // leaving any remainder already correctly positioned on the
        // stack) and cleans that remainder back off via the new
        // emit_add_rsp_imm8() after the call returns; gen_program()'s own
        // prologue now reads a stack-passed parameter from
        // `[rbp+16+8*(idx-4)]` into its own local slot via two EXISTING
        // encodings (emit_mov_rax_rbp_off()/emit_mov_rbp_off_rax()), zero
        // new receiving-side opcodes. See this file's own top Milestone
        // 107 doc comment for the full derivation, including the real,
        // checked (not assumed) argument-evaluation-order-is-unobservable
        // proof this reversal relies on.
        // -------------------------------------------------------------
        // CASE 85: five parameters -- the first call this whole codegen
        // has ever made with a genuinely stack-passed argument (`e`,
        // index 4). A base-10-digit-weighted return value, the same
        // technique CASE 26's own `mix4` established for proving all 4
        // registers land correctly, extended one slot past the register
        // boundary: any mixup between the four register-passed arguments
        // and the one stack-passed argument -- wrong value, wrong slot,
        // register argument accidentally clobbered by the stack-cleanup
        // add rsp -- changes this single scalar result.
        //   int mix5(int a,int b,int c,int d,int e){
        //       return a*10000+b*1000+c*100+d*10+e; }
        //   int main() { return mix5(1, 2, 3, 4, 5); }
        // Hand-computed: 1*10000+2*1000+3*100+4*10+5 = 12345.
        const SRC85: &[u8] = b"int mix5(int a, int b, int c, int d, int e) { return a * 10000 + b * 1000 + c * 100 + d * 10 + e; } int main() { return mix5(1, 2, 3, 4, 5); }";
        let case85_result = compile_and_run_program_callable(SRC85.as_ptr() as u64, SRC85.len() as u64);
        w(b"  case85 (mix5(1,2,3,4,5), real 5-param call -- first stack-passed argument, in-process) returned=");
        if let Some(r) = case85_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 12345)\n");
        let case85_ok = case85_result == Some(12345);
        write_check(b"case85_five_param_call_returns_12345=", case85_ok);

        w(b"\n");

        // CASE 86: all 8 of MAX_PARAMS -- 4 register-passed, 4 stack-
        // passed -- the full-width stress case. Same digit-weighted
        // technique, now across the whole parameter list, so a single
        // scalar return value proves every one of the 8 arguments landed
        // in its own correct position, register- and stack-passed alike.
        //   int mix8(int a,int b,int c,int d,int e,int f,int g,int h){
        //       return a*10000000+b*1000000+c*100000+d*10000
        //            + e*1000+f*100+g*10+h; }
        //   int main() { return mix8(1,2,3,4,5,6,7,8); }
        // Hand-computed: 12345678.
        const SRC86: &[u8] = b"int mix8(int a, int b, int c, int d, int e, int f, int g, int h) { return a * 10000000 + b * 1000000 + c * 100000 + d * 10000 + e * 1000 + f * 100 + g * 10 + h; } int main() { return mix8(1, 2, 3, 4, 5, 6, 7, 8); }";
        let case86_result = compile_and_run_program_callable(SRC86.as_ptr() as u64, SRC86.len() as u64);
        w(b"  case86 (mix8(1..8), real 8-param call -- full MAX_PARAMS width, 4 register + 4 stack, in-process) returned=");
        if let Some(r) = case86_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 12345678)\n");
        let case86_ok = case86_result == Some(12345678);
        write_check(b"case86_eight_param_call_returns_12345678=", case86_ok);

        w(b"\n");

        // CASE 87: the cap itself -- a 9-parameter function definition
        // (MAX_PARAMS + 1) is still a real, disclosed
        // ParseError::TooManyParams. Proves the cap MOVED (4 -> 8), not
        // that it was quietly removed.
        //   int nine(int a,int b,int c,int d,int e,int f,int g,int h,int i)
        //   { return a; }
        const SRC87: &[u8] = b"int nine(int a, int b, int c, int d, int e, int f, int g, int h, int i) { return a; }";
        let case87_ok = match lex_and_parse_program(SRC87.as_ptr() as u64, SRC87.len() as u64) {
            Err(_) => {
                w(b"  case87 9-parameter function definition rejected (TooManyParams), as expected\n");
                true
            }
            Ok(_) => {
                w(b"  case87 expected a ParseError::TooManyParams for a 9th parameter, got Ok\n");
                false
            }
        };
        write_check(b"case87_ninth_parameter_is_too_many_params=", case87_ok);

        w(b"\n");

        let overall_m107 = case85_ok && case86_ok && case87_ok;
        w(b"OVERALL_M107=");
        w(if overall_m107 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 108: single-dimension fixed-size `int` array
        // declarations, indexed reads, and indexed writes -- see this
        // file's own top Milestone 108 doc comment for the full grammar
        // delta and codegen derivation.
        //
        // CASE 88: exactly MAX_VARS (8) elements, no other locals in the
        // function -- both the real position-correctness proof (the
        // same base-10-digit-weighted-return technique CASE 26's own
        // `mix4` and CASE 86's own `mix8` already established, so a
        // single scalar result proves EVERY element landed in its own
        // correct slot, not just that some commutative combination came
        // out right) AND the real MAX_VARS boundary-SUCCESS proof (one
        // array using the entire real 8-slot vars-table budget by
        // itself, the same cap collect_vars_rec()'s own new Milestone
        // 108 branch enforces for every array, not a smaller,
        // unrepresentative array size).
        //   int main() {
        //       int arr[8];
        //       arr[0]=1; arr[1]=2; arr[2]=3; arr[3]=4;
        //       arr[4]=5; arr[5]=6; arr[6]=7; arr[7]=8;
        //       return arr[0]*10000000 + arr[1]*1000000 + arr[2]*100000
        //            + arr[3]*10000 + arr[4]*1000 + arr[5]*100 + arr[6]*10 + arr[7];
        //   }
        // Hand-computed: 12345678.
        // -------------------------------------------------------------
        const SRC88: &[u8] = b"int main() { int arr[8]; arr[0] = 1; arr[1] = 2; arr[2] = 3; arr[3] = 4; arr[4] = 5; arr[5] = 6; arr[6] = 7; arr[7] = 8; return arr[0] * 10000000 + arr[1] * 1000000 + arr[2] * 100000 + arr[3] * 10000 + arr[4] * 1000 + arr[5] * 100 + arr[6] * 10 + arr[7]; }";
        let case88_result = compile_and_run_program_callable(SRC88.as_ptr() as u64, SRC88.len() as u64);
        w(b"  case88 (int arr[8], every element written and read back at its own position, in-process) returned=");
        if let Some(r) = case88_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 12345678)\n");
        let case88_ok = case88_result == Some(12345678);
        write_check(b"case88_eight_element_array_returns_12345678=", case88_ok);

        w(b"\n");

        // CASE 89: a REAL runtime variable index -- a `while`-loop
        // counter, not a literal -- proving this codegen genuinely
        // computes `&arr[0] - idx*8` at runtime (emit_lea_rax_rbp_off()/
        // emit_shl_rcx_imm8()/emit_sub_rax_rcx()/emit_mov_rax_mem_rax())
        // rather than folding a constant index at compile time; a
        // literal-index-only implementation could pass CASE 88 above by
        // accident (e.g. by mis-treating each `arr[K]` for constant K as
        // an independent scalar slot named "arr_K") without ever
        // exercising the real address arithmetic this milestone's own
        // codegen adds. Also realistic array usage -- sum every element
        // via a loop, the actual point of an indexable array over a
        // named scalar.
        //   int main() {
        //       int arr[5]; int i; int sum;
        //       arr[0]=10; arr[1]=20; arr[2]=30; arr[3]=40; arr[4]=50;
        //       i = 0; sum = 0;
        //       while (i < 5) { sum = sum + arr[i]; i = i + 1; }
        //       return sum;
        //   }
        // Hand-computed: 10+20+30+40+50 = 150.
        // -------------------------------------------------------------
        const SRC89: &[u8] = b"int main() { int arr[5]; int i; int sum; arr[0] = 10; arr[1] = 20; arr[2] = 30; arr[3] = 40; arr[4] = 50; i = 0; sum = 0; while (i < 5) { sum = sum + arr[i]; i = i + 1; } return sum; }";
        let case89_result = compile_and_run_program_callable(SRC89.as_ptr() as u64, SRC89.len() as u64);
        w(b"  case89 (int arr[5] summed via a real while-loop counter as the index, in-process) returned=");
        if let Some(r) = case89_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 150)\n");
        let case89_ok = case89_result == Some(150);
        write_check(b"case89_runtime_indexed_loop_sum_returns_150=", case89_ok);

        w(b"\n");

        // CASE 90: the real cap itself -- MAX_VARS (8) + 1 elements in a
        // single array, and NOTHING else declared in the function (so
        // this is genuinely the array alone exceeding the budget, not an
        // array competing with other locals for it). The same "prove the
        // cap moved/exists for real, don't just exercise the happy path"
        // discipline CASE 87 already established for MAX_PARAMS, applied
        // here to MAX_VARS -- CodeGenError::TooManyVars existed since
        // Milestone 67/72 but (checked directly against this file's own
        // history: grepping this file for "TooManyVars" before this
        // milestone finds only its own definition and its two real throw
        // sites, never a single self-test call site) had never once been
        // exercised by any self-test case in this file until this one.
        //   int main() { int arr[9]; return arr[0]; }
        // -------------------------------------------------------------
        const SRC90: &[u8] = b"int main() { int arr[9]; return arr[0]; }";
        let case90_ok = match lex_and_parse_program(SRC90.as_ptr() as u64, SRC90.len() as u64) {
            Ok(prog90) => match gen_program(SRC90.as_ptr() as u64, prog90, CodegenMode::Callable) {
                Err(CodeGenError::TooManyVars) => {
                    w(b"  case90 9-element array (MAX_VARS + 1) rejected (TooManyVars), as expected\n");
                    true
                }
                _ => {
                    w(b"  case90 expected Err(CodeGenError::TooManyVars) for a 9-element array, got something else\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case90 lex_and_parse_program() returned Err unexpectedly (source should lex+parse fine)\n");
                false
            }
        };
        write_check(b"case90_nine_element_array_is_too_many_vars=", case90_ok);

        w(b"\n");

        let overall_m108 = case88_ok && case89_ok && case90_ok;
        w(b"OVERALL_M108=");
        w(if overall_m108 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 109: single-level `int` pointer declaration,
        // address-of, and dereference (read AND write) -- see this
        // file's own top Milestone 109 doc comment for the full grammar
        // delta and codegen derivation. All four cases run in-process
        // (CodegenMode::Callable), the same verification tier Milestone
        // 107/108 both already used for their own new cases -- no new
        // real fork()/exec()/wait() cycle is added by this milestone.
        //
        // CASE 91: the combined declaration+initializer form
        // (`int *p = &x;`, parse_pointer_decl()'s own optional
        // initializer branch) exercising address-of AND
        // dereference-read together in one program.
        //   int main() {
        //       int x;
        //       x = 10;
        //       int *p = &x;
        //       return *p;
        //   }
        // Hand-computed: 10.
        // -------------------------------------------------------------
        const SRC91: &[u8] = b"int main() { int x; x = 10; int *p = &x; return *p; }";
        let case91_result = compile_and_run_program_callable(SRC91.as_ptr() as u64, SRC91.len() as u64);
        w(b"  case91 (int *p = &x; return *p;, combined pointer decl+init, address-of + dereference-read, in-process) returned=");
        if let Some(r) = case91_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 10)\n");
        let case91_ok = case91_result == Some(10);
        write_check(b"case91_pointer_addr_and_deref_read_returns_10=", case91_ok);

        w(b"\n");

        // CASE 92: `*p = expr;` -- the real, distinguishing proof that a
        // dereference WRITE mutates the pointee's own real stack slot
        // (genuine aliasing), not some independent copy `p` happens to
        // hold -- a shallow/buggy implementation that treated `p` as an
        // ordinary variable disconnected from `x`'s own storage would
        // still pass CASE 91 above (a fresh read right back through the
        // same pointer) but would fail THIS case, since `x` itself would
        // stay 5 if the write did not really land at `x`'s own address.
        //   int main() {
        //       int x; int *p;
        //       x = 5; p = &x; *p = 99;
        //       return x;
        //   }
        // Hand-computed: 99.
        // -------------------------------------------------------------
        const SRC92: &[u8] = b"int main() { int x; int *p; x = 5; p = &x; *p = 99; return x; }";
        let case92_result = compile_and_run_program_callable(SRC92.as_ptr() as u64, SRC92.len() as u64);
        w(b"  case92 (*p = 99; return x; -- real aliasing proof, dereference-write mutates the pointee, in-process) returned=");
        if let Some(r) = case92_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 99)\n");
        let case92_ok = case92_result == Some(99);
        write_check(b"case92_deref_write_mutates_pointee_returns_99=", case92_ok);

        w(b"\n");

        // CASE 93: a real "swap via pointers" idiom -- two SEPARATE
        // pointer variables (`pa`, `pb`) must each hold their own
        // distinct address (a bug that aliased both to the same slot
        // would show up here as the swap silently doing nothing, or
        // corrupting one of the two values), combined with both
        // dereference-read and dereference-write in the same program --
        // the strongest realistic composition proof this milestone's
        // own self-test offers, the same "distinguishing, not just
        // commutative-safe" discipline CASE 88/CASE 26 already
        // established for arrays/register arguments.
        //   int main() {
        //       int a; int b; int *pa; int *pb; int tmp;
        //       a = 3; b = 7;
        //       pa = &a; pb = &b;
        //       tmp = *pa; *pa = *pb; *pb = tmp;
        //       return a * 100 + b;
        //   }
        // Hand-computed: a becomes 7, b becomes 3 -> 7*100+3 = 703.
        // -------------------------------------------------------------
        const SRC93: &[u8] = b"int main() { int a; int b; int *pa; int *pb; int tmp; a = 3; b = 7; pa = &a; pb = &b; tmp = *pa; *pa = *pb; *pb = tmp; return a * 100 + b; }";
        let case93_result = compile_and_run_program_callable(SRC93.as_ptr() as u64, SRC93.len() as u64);
        w(b"  case93 (swap a/b via two distinct pointers pa/pb, in-process) returned=");
        if let Some(r) = case93_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 703)\n");
        let case93_ok = case93_result == Some(703);
        write_check(b"case93_pointer_swap_returns_703=", case93_ok);

        w(b"\n");

        // CASE 94: the real, disclosed narrow-scope proof -- `&`
        // applied to a non-IDENT operand (a bare INTLIT here) is a real
        // ParseError, not silently accepted, since parse_unary()'s own
        // `&` arm calls `self.expect(TOK_IDENT)` directly rather than
        // recursing into the general expression grammar (see that
        // function's own Milestone 109 doc comment for the full
        // reasoning). The same "prove the scope restriction is real,
        // don't just document it" discipline CASE 87/90 already
        // established for MAX_PARAMS/MAX_VARS.
        //   int main() { return &5; }
        // -------------------------------------------------------------
        const SRC94: &[u8] = b"int main() { return &5; }";
        let case94_ok = match lex_and_parse_program(SRC94.as_ptr() as u64, SRC94.len() as u64) {
            Err(_) => {
                w(b"  case94 `&5` (address-of a non-IDENT operand) rejected at parse time, as expected\n");
                true
            }
            Ok(_) => {
                w(b"  case94 expected a ParseError for `&5` (address-of a non-IDENT operand), got Ok\n");
                false
            }
        };
        write_check(b"case94_address_of_non_ident_is_parse_error=", case94_ok);

        w(b"\n");

        let overall_m109 = case91_ok && case92_ok && case93_ok && case94_ok;
        w(b"OVERALL_M109=");
        w(if overall_m109 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 110: real array-to-pointer decay, and pointers (and,
        // through decay, arrays) crossing a real function-call boundary
        // -- see this file's own top Milestone 110 doc comment for the
        // full grammar delta and codegen derivation. All three cases run
        // in-process (CodegenMode::Callable), the same verification tier
        // Milestone 107/108/109 all already used for their own new
        // cases -- no new real fork()/exec()/wait() cycle is added by
        // this milestone.
        //
        // CASE 95: a declared array decays to a real address when passed
        // as a call argument, and the callee subscripts THROUGH the
        // resulting pointer parameter with ordinary `p[idx]` syntax --
        // proves both halves at once (decay at the call site, and the
        // new is_array-aware EXPR_INDEX codegen correctly reading a
        // pointer's own VALUE as the base instead of the slot's own
        // address).
        //   int sum3(int *p) { return p[0] + p[1] + p[2]; }
        //   int main() {
        //       int arr[3];
        //       arr[0] = 10; arr[1] = 20; arr[2] = 30;
        //       return sum3(arr);
        //   }
        // Hand-computed: 10+20+30 = 60.
        // -------------------------------------------------------------
        const SRC95: &[u8] = b"int sum3(int *p) { return p[0] + p[1] + p[2]; } int main() { int arr[3]; arr[0] = 10; arr[1] = 20; arr[2] = 30; return sum3(arr); }";
        let case95_result = compile_and_run_program_callable(SRC95.as_ptr() as u64, SRC95.len() as u64);
        w(b"  case95 (sum3(arr) -- array decays to pointer argument, callee subscripts via p[idx], in-process) returned=");
        if let Some(r) = case95_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 60)\n");
        let case95_ok = case95_result == Some(60);
        write_check(b"case95_array_decay_call_arg_returns_60=", case95_ok);

        w(b"\n");

        // CASE 96: the real, distinguishing proof that decay hands the
        // callee a genuine address into the CALLER's own stack frame --
        // real aliasing across a function-call boundary, not a copy. A
        // shallow/buggy implementation (e.g. one that accidentally
        // passed each array element by value, or decayed to a fresh
        // scratch copy) could still pass CASE 95 above (a read-only
        // sum) but would fail this one, since the caller's own `arr`
        // would stay `5, 7` if the callee's writes did not really land
        // at `arr`'s own real addresses. `zero_out(arr)`'s own result is
        // captured into `r` rather than called as a bare statement -- a
        // real, pre-existing, already-disclosed limitation this
        // milestone's own first draft ran straight into (not a new
        // finding): this subset-C has never had an "expression
        // statement" production at all (see this file's own top doc
        // comment, "no function may be called as a bare expression-
        // statement ... an existing gap since Milestone 67, unwidened
        // here"), so a call can only appear where an EXPRESSION is
        // legal -- an assignment RHS here, exactly like every prior
        // milestone's own call-expression self-test cases already do.
        //   int zero_out(int *p) { p[0] = 0; p[1] = 0; return 0; }
        //   int main() {
        //       int arr[2]; int r;
        //       arr[0] = 5; arr[1] = 7;
        //       r = zero_out(arr);
        //       return arr[0] * 100 + arr[1];
        //   }
        // Hand-computed: 0.
        // -------------------------------------------------------------
        const SRC96: &[u8] = b"int zero_out(int *p) { p[0] = 0; p[1] = 0; return 0; } int main() { int arr[2]; int r; arr[0] = 5; arr[1] = 7; r = zero_out(arr); return arr[0] * 100 + arr[1]; }";
        let case96_result = compile_and_run_program_callable(SRC96.as_ptr() as u64, SRC96.len() as u64);
        w(b"  case96 (zero_out(arr) writes through p[idx] -- real cross-call aliasing proof, in-process) returned=");
        if let Some(r) = case96_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 0)\n");
        let case96_ok = case96_result == Some(0);
        write_check(b"case96_pointer_param_write_aliases_caller_array_returns_0=", case96_ok);

        w(b"\n");

        // CASE 97: a real pointer RETURN type (`int *identity(int *p)`),
        // proving both new grammar positions at once -- a pointer
        // parameter passed straight through and handed back as a
        // pointer return value (RAX pass-through, zero narrowing, the
        // exact same codegen an ordinary `int` return already used) --
        // and that the CALLER can dereference a call expression's own
        // result directly (`*identity(&x)`, OP_DEREF's own operand
        // recursing straight into an EXPR_CALL, exactly as Milestone
        // 109's own parse_unary() doc comment already described as
        // legal).
        //   int *identity(int *p) { return p; }
        //   int main() {
        //       int x;
        //       x = 42;
        //       return *identity(&x);
        //   }
        // Hand-computed: 42.
        // -------------------------------------------------------------
        const SRC97: &[u8] = b"int *identity(int *p) { return p; } int main() { int x; x = 42; return *identity(&x); }";
        let case97_result = compile_and_run_program_callable(SRC97.as_ptr() as u64, SRC97.len() as u64);
        w(b"  case97 (*identity(&x) -- pointer parameter AND pointer return type, in-process) returned=");
        if let Some(r) = case97_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 42)\n");
        let case97_ok = case97_result == Some(42);
        write_check(b"case97_pointer_param_and_return_type_returns_42=", case97_ok);

        w(b"\n");

        let overall_m110 = case95_ok && case96_ok && case97_ok;
        w(b"OVERALL_M110=");
        w(if overall_m110 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 111: `arr = expr;` (and, via the pre-existing
        // compound-assignment desugaring, `arr op= expr;`) is now a
        // real, deliberate `CodeGenError::ArrayNotAssignable` compile
        // error instead of silently overwriting element 0's raw value --
        // see this file's own top Milestone 111 doc comment and
        // `CodeGenError::ArrayNotAssignable`'s own doc comment for the
        // full reasoning. Closes the exact read/write asymmetry
        // Milestone 110's own closing disclosure named as still open
        // ("reading `arr` now decays to its address ... but writing ...
        // still overwrites element 0's raw VALUE unchanged"). Both cases
        // run at the `lex_and_parse()` + `gen_function()` tier
        // (`CodegenMode::Callable`), the same tier CASE 59
        // (`UndefinedLabel`) already used for its own new error variant
        // -- no new real `fork()`/`exec()`/`wait()` cycle is added by
        // this milestone.
        //
        // CASE 98: a direct `arr = 5;` must be
        // `Err(CodeGenError::ArrayNotAssignable)`, the same "prove the
        // new Err path is real, don't just document it" discipline CASE
        // 7 (`UndeclaredVariable`), CASE 49 (`BreakOutsideLoop`), and
        // CASE 59 (`UndefinedLabel`) already established for their own
        // new error variants.
        //   int main() { int arr[3]; arr[0] = 1; arr = 5; return arr[0]; }
        // -------------------------------------------------------------
        const SRC98: &[u8] = b"int main() { int arr[3]; arr[0] = 1; arr = 5; return arr[0]; }";
        let src98_ptr = SRC98.as_ptr() as u64;
        let src98_len = SRC98.len() as u64;
        let case98_ok = match lex_and_parse(src98_ptr, src98_len) {
            Ok(func98) => match gen_function(src98_ptr, func98, CodegenMode::Callable) {
                Err(CodeGenError::ArrayNotAssignable(_)) => {
                    w(b"  case98 real codegen error -- `arr = 5;` rejected, an array is not a modifiable lvalue\n");
                    true
                }
                _ => {
                    w(b"  case98 expected Err(ArrayNotAssignable), got something else\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case98 lex_and_parse() returned Err unexpectedly (source should lex+parse fine)\n");
                false
            }
        };
        write_check(b"case98_bare_array_assign_is_ArrayNotAssignable=", case98_ok);

        w(b"\n");

        // CASE 99: the same rejection reached through the COMPOUND-
        // assignment desugaring path instead (`arr += 5;`) --
        // `parse_assign_stmt()` rewrites every `op=` spelling into an
        // ordinary `STMT_ASSIGN` before codegen ever runs (Milestone 84's
        // own desugaring, unchanged here), so this is a real,
        // distinguishing proof that the fix lives in `STMT_ASSIGN`'s one
        // shared codegen arm rather than a hypothetical direct-
        // assignment-only check that this case alone would expose as
        // incomplete: a fix bolted onto some separate compound-assignment
        // code path (there isn't one) could still pass CASE 98 above but
        // would fail this one.
        //   int main() { int arr[3]; arr[0] = 1; arr += 5; return arr[0]; }
        // -------------------------------------------------------------
        const SRC99: &[u8] = b"int main() { int arr[3]; arr[0] = 1; arr += 5; return arr[0]; }";
        let src99_ptr = SRC99.as_ptr() as u64;
        let src99_len = SRC99.len() as u64;
        let case99_ok = match lex_and_parse(src99_ptr, src99_len) {
            Ok(func99) => match gen_function(src99_ptr, func99, CodegenMode::Callable) {
                Err(CodeGenError::ArrayNotAssignable(_)) => {
                    w(b"  case99 real codegen error -- `arr += 5;` rejected too, reached via the same shared STMT_ASSIGN codegen arm through compound-assign desugaring\n");
                    true
                }
                _ => {
                    w(b"  case99 expected Err(ArrayNotAssignable), got something else\n");
                    false
                }
            },
            Err(_) => {
                w(b"  case99 lex_and_parse() returned Err unexpectedly (source should lex+parse fine)\n");
                false
            }
        };
        write_check(b"case99_compound_array_assign_is_ArrayNotAssignable=", case99_ok);

        w(b"\n");

        let overall_m111 = case98_ok && case99_ok;
        w(b"OVERALL_M111=");
        w(if overall_m111 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 112: real `char *` pointers -- declaration and a real
        // byte-narrowing dereference-READ, the narrow first slice
        // Milestone 110's own closing disclosure named as the OTHER
        // candidate alongside the `arr = expr;` asymmetry Milestone 111
        // just resolved. See this file's own top Milestone 112 doc
        // comment and `VarSlot::is_char_ptr`'s own doc comment for the
        // full grammar/codegen derivation. All three cases run in-process
        // (`CodegenMode::Callable`), the same tier Milestone 107 through
        // 111 all already used for their own new cases -- no new real
        // `fork()`/`exec()`/`wait()` cycle is added by this milestone.
        //
        // CASE 100: the real, distinguishing proof that `*p` genuinely
        // narrows to a signed byte when `p` is `char *`, not merely that
        // it reads SOME value back correctly. `c`'s own full 8-byte slot
        // stores 200 (`char`'s own store side is always full-width,
        // unchanged since Milestone 102); a buggy/shallow implementation
        // that reused `int *`'s own unconditional full-width
        // `emit_mov_rax_mem_rax()` for `*p` too would read back 200 (the
        // raw stored value, not narrowed), making `*p == -56` false; the
        // real byte-narrowing load only correctly reads back the
        // sign-extended low byte, -56 (200 - 256, the exact `char`
        // signedness precedent CASE 78/84 already established for a
        // PLAIN `char` variable's own read, now proven true through a
        // pointer for the first time).
        //   int main() {
        //       char c; c = 200;
        //       char *p = &c;
        //       return *p == -56;
        //   }
        // Hand-computed: 1.
        // -------------------------------------------------------------
        const SRC100: &[u8] = b"int main() { char c; c = 200; char *p = &c; return *p == -56; }";
        let case100_result = compile_and_run_program_callable(SRC100.as_ptr() as u64, SRC100.len() as u64);
        w(b"  case100 (*p == -56 -- char* dereference-read narrows to a real signed byte, in-process) returned=");
        if let Some(r) = case100_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        let case100_ok = case100_result == Some(1);
        write_check(b"case100_char_ptr_deref_read_narrows_returns_1=", case100_ok);

        w(b"\n");

        // CASE 101: the real, distinguishing proof that a WRITE through a
        // `char *` is genuine aliasing into the pointee's own real stack
        // slot -- not a disconnected copy -- combined with proof that the
        // write side deliberately does NOT narrow (this milestone's own
        // disclosed design choice: `*p = expr;` always stores the full 8
        // bytes, exactly like an ordinary `char` scalar's own store
        // already does, narrowing only ever happening on READ). `*p =
        // 200;` writes the full raw value 200 into `c`'s own slot;
        // reading `c` DIRECTLY afterward (NOT through `p`) narrows it via
        // the pre-existing, unchanged `char`-scalar read path to -56 --
        // the same "read back through a DIFFERENT access path than the
        // one that wrote it" distinguishing shape Milestone 109's own
        // CASE 92 established for `int *` aliasing, now proven for
        // `char *` too.
        //   int main() {
        //       char c; c = 5;
        //       char *p = &c;
        //       *p = 200;
        //       return c == -56;
        //   }
        // Hand-computed: 1.
        // -------------------------------------------------------------
        const SRC101: &[u8] = b"int main() { char c; c = 5; char *p = &c; *p = 200; return c == -56; }";
        let case101_result = compile_and_run_program_callable(SRC101.as_ptr() as u64, SRC101.len() as u64);
        w(b"  case101 (*p = 200; return c == -56; -- real aliasing through char*, write side stays full-width, in-process) returned=");
        if let Some(r) = case101_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        let case101_ok = case101_result == Some(1);
        write_check(b"case101_char_ptr_write_aliases_pointee_returns_1=", case101_ok);

        w(b"\n");

        // CASE 102: the real, distinguishing proof that `int *` and
        // `char *` dereference-reads coexist correctly in the SAME
        // program with zero cross-contamination -- `find_var()`'s new
        // `is_char_ptr` field is looked up PER POINTER VARIABLE, so a bug
        // that accidentally narrowed EVERY dereference (or narrowed
        // NEITHER) would still pass CASE 100/101 in isolation but fails
        // here: `*ip == 1000` is true only if `int *` dereference stayed
        // full-width (1000's own low byte alone, sign-extended, is NOT
        // 1000); `*cp == -56` is true only if `char *` dereference
        // genuinely narrowed. The combined arithmetic result is only 11
        // if BOTH are independently correct at once.
        //   int main() {
        //       int x; x = 1000;
        //       int *ip = &x;
        //       char c; c = 200;
        //       char *cp = &c;
        //       return (*ip == 1000) + (*cp == -56) * 10;
        //   }
        // Hand-computed: (1) + (1)*10 = 11.
        // -------------------------------------------------------------
        const SRC102: &[u8] = b"int main() { int x; x = 1000; int *ip = &x; char c; c = 200; char *cp = &c; return (*ip == 1000) + (*cp == -56) * 10; }";
        let case102_result = compile_and_run_program_callable(SRC102.as_ptr() as u64, SRC102.len() as u64);
        w(b"  case102 (int* and char* dereference-reads coexist with zero cross-contamination, in-process) returned=");
        if let Some(r) = case102_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 11)\n");
        let case102_ok = case102_result == Some(11);
        write_check(b"case102_int_ptr_and_char_ptr_coexist_returns_11=", case102_ok);

        w(b"\n");

        let overall_m112 = case100_ok && case101_ok && case102_ok;
        w(b"OVERALL_M112=");
        w(if overall_m112 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        w(b"\n");

        // -------------------------------------------------------------
        // MILESTONE 113: `char *` crossing a real function-call boundary
        // -- parameters AND return values -- the exact candidate
        // Milestone 112's own closing disclosure named ("no `char *`
        // function parameters or return values ... a future milestone
        // ... would need to thread a real declared-parameter-type bit
        // through `ParamInfo`, which does not exist today"), the same
        // step Milestone 110 already took for `int *` after Milestone
        // 109. See this file's own top Milestone 113 doc comment,
        // `ParamInfo::is_char_ptr`'s own doc comment, and
        // `collect_vars_for_function()`'s own Milestone 113 comment for
        // the full grammar/codegen derivation. All four cases run
        // in-process (`CodegenMode::Callable`), the same tier Milestone
        // 107 through 112 all already used for their own new cases -- no
        // new real `fork()`/`exec()`/`wait()` cycle is added by this
        // milestone.
        //
        // CASE 103: the real, distinguishing proof that a `char *`
        // PARAMETER's own dereference-read still genuinely narrows --
        // not merely that a `char *` local does (Milestone 112's own
        // CASE 100 already proved that). `p` is passed as a real
        // argument, received as a real parameter, and `*p` is
        // dereferenced entirely inside the CALLEE -- a buggy
        // implementation that left `ParamInfo::is_char_ptr` unthreaded
        // (the exact bug this milestone's own `collect_vars_for_
        // function()` fix closes) would produce a parameter `VarSlot`
        // with `is_char_ptr == 0`, silently falling back to the
        // un-narrowed `int *`-style load and reading back the raw 200
        // instead of -56.
        //   int read_narrow(char *p) { return *p == -56; }
        //   int main() {
        //       char c; c = 200;
        //       char *p = &c;
        //       return read_narrow(p);
        //   }
        // Hand-computed: 1.
        // -------------------------------------------------------------
        const SRC103: &[u8] = b"int read_narrow(char *p) { return *p == -56; } int main() { char c; c = 200; char *p = &c; return read_narrow(p); }";
        let case103_result = compile_and_run_program_callable(SRC103.as_ptr() as u64, SRC103.len() as u64);
        w(b"  case103 (char* parameter dereference-read narrows across a real call boundary, in-process) returned=");
        if let Some(r) = case103_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        let case103_ok = case103_result == Some(1);
        write_check(b"case103_char_ptr_param_deref_read_narrows_returns_1=", case103_ok);

        w(b"\n");

        // CASE 104: the real, distinguishing proof that a WRITE through a
        // `char *` PARAMETER is genuine aliasing into the CALLER's own
        // real stack slot, crossing a function-call boundary -- the same
        // "read back through a DIFFERENT access path than the one that
        // wrote it" distinguishing shape Milestone 112's own CASE 101
        // already established for a local `char *`, and Milestone 110's
        // own CASE 96 already established for `int *`, now proven for a
        // `char *` PARAMETER specifically: `write_it()`'s own `*p = 200;`
        // must land on the exact same bytes `main()`'s own `c` occupies,
        // not a disconnected copy. The write side stays deliberately
        // un-narrowed (Milestone 112's own design choice, unaffected by
        // this milestone): `c` is read back DIRECTLY (not through `p`),
        // narrowing 200 to -56 via the ordinary, unchanged `char`-scalar
        // read path.
        //   int write_it(char *p) { *p = 200; return 0; }
        //   int main() {
        //       char c; c = 5;
        //       char *p = &c;
        //       int r; r = write_it(p);
        //       return c == -56;
        //   }
        // Hand-computed: 1.
        // -------------------------------------------------------------
        const SRC104: &[u8] = b"int write_it(char *p) { *p = 200; return 0; } int main() { char c; c = 5; char *p = &c; int r; r = write_it(p); return c == -56; }";
        let case104_result = compile_and_run_program_callable(SRC104.as_ptr() as u64, SRC104.len() as u64);
        w(b"  case104 (char* parameter write aliases the CALLER's real stack slot across a call boundary, in-process) returned=");
        if let Some(r) = case104_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        let case104_ok = case104_result == Some(1);
        write_check(b"case104_char_ptr_param_write_aliases_caller_returns_1=", case104_ok);

        w(b"\n");

        // CASE 105: the real, distinguishing proof that a `char *`
        // RETURN type works correctly -- specifically, that
        // `parse_function()`'s own new star-check leaves `is_char_return`
        // FALSE for a `char *` return (mirroring `int *`'s own Milestone
        // 110 return-type treatment), so the returned ADDRESS reaches the
        // caller un-corrupted. A buggy implementation that left
        // `is_char_return` TRUE for `char *identity_char(...)` (treating
        // the leading `char` keyword alone as the whole signal, ignoring
        // the following `*`) would apply `emit_movsx_rax_al()` to the
        // returned ADDRESS itself, not a char VALUE -- sign-extending its
        // own low byte into a completely different, essentially random
        // address -- so `q` would hold a corrupted pointer and `*q` would
        // almost certainly read the wrong memory entirely (not merely the
        // wrong VALUE), the same "this would be caught, not silently
        // wrong" property Milestone 110's own CASE 97 doc comment already
        // reasoned about for the analogous `int *` case. The result is
        // assigned to a local `char *q` first (an ordinary STMT_ASSIGN,
        // unaffected by this milestone) and dereferenced through THAT
        // plain IDENT, staying within the real, disclosed EXPR_IDENT-only
        // shape restriction Milestone 112's own `*p` codegen already
        // established (see CASE 106 immediately below for the real,
        // disclosed asymmetry this restriction still leaves open now
        // that it is newly reachable).
        //   char *identity_char(char *p) { return p; }
        //   int main() {
        //       char c; c = 200;
        //       char *q; q = identity_char(&c);
        //       return *q == -56;
        //   }
        // Hand-computed: 1.
        // -------------------------------------------------------------
        const SRC105: &[u8] = b"char *identity_char(char *p) { return p; } int main() { char c; c = 200; char *q; q = identity_char(&c); return *q == -56; }";
        let case105_result = compile_and_run_program_callable(SRC105.as_ptr() as u64, SRC105.len() as u64);
        w(b"  case105 (char* RETURN type crosses a call boundary un-corrupted, dereferenced via a local pointer, in-process) returned=");
        if let Some(r) = case105_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 1)\n");
        let case105_ok = case105_result == Some(1);
        write_check(b"case105_char_ptr_return_crosses_call_returns_1=", case105_ok);

        w(b"\n");

        // CASE 106: a real, deliberate, DISCLOSED negative case, not a
        // regression -- the exact same "resolved by operand SHAPE, not
        // attempted for every expression" restriction Milestone 112's own
        // `*p` codegen already established (`e.left` must be a plain
        // `EXPR_IDENT`; a call result `*f()` falls through to the
        // ordinary full-width load) was UNREACHABLE in Milestone 112's
        // own scope (no `char *` return values existed yet) but is
        // genuinely REACHABLE for the first time in THIS milestone's own
        // scope, now that a function may return `char *`. This milestone
        // does NOT resolve that restriction (narrowing `*f()` directly
        // would need `gen_expr()`'s `EXPR_CALL` arm itself to carry
        // pointee-type information forward, real new plumbing out of
        // scope here) -- this case exists to PROVE the real, measured
        // behavior rather than merely assert it in prose: `*identity_
        // char2(&c)` dereferences a CALL result directly, `e.left.kind ==
        // EXPR_CALL`, not `EXPR_IDENT`, so `is_char_pointee` is `false`
        // and the ordinary un-narrowed `emit_mov_rax_mem_rax()` runs,
        // reading back the raw 200 (not -56) -- `200 == -56` is false, 0.
        // A future milestone that closes this gap must also update this
        // case's own expected value and doc comment; until then, this is
        // a real regression tripwire on the disclosed gap's exact
        // numeric fingerprint, the same "diffed for equality across
        // milestones, not silently accepted either way" discipline the
        // fs self-test's own known FAIL-line-count baseline already uses
        // for a different pre-existing, disclosed artifact.
        //   char *identity_char2(char *p) { return p; }
        //   int main() {
        //       char c; c = 200;
        //       return *identity_char2(&c) == -56;
        //   }
        // Hand-computed (real, current, disclosed-gap behavior, NOT the
        // morally "correct" narrowed answer): 0.
        // -------------------------------------------------------------
        const SRC106: &[u8] = b"char *identity_char2(char *p) { return p; } int main() { char c; c = 200; return *identity_char2(&c) == -56; }";
        let case106_result = compile_and_run_program_callable(SRC106.as_ptr() as u64, SRC106.len() as u64);
        w(b"  case106 (DISCLOSED GAP: *call() dereferencing a char*-returning call result directly does NOT narrow -- real, measured, not yet fixed, in-process) returned=");
        if let Some(r) = case106_result {
            write_u64_dec(r);
        } else {
            w(b"(compile failed)");
        }
        w(b" (expected 0 -- the real, disclosed, un-narrowed fallback, NOT a bug this milestone introduces)\n");
        let case106_ok = case106_result == Some(0);
        write_check(b"case106_char_ptr_call_result_deref_disclosed_gap_returns_0=", case106_ok);

        w(b"\n");

        let overall_m113 = case103_ok && case104_ok && case105_ok && case106_ok;
        w(b"OVERALL_M113=");
        w(if overall_m113 { b"PASS" } else { b"FAIL" });
        w(b"\n");

        sys_exit(if overall && overall_m68 && overall_m69 && overall_m70 && overall_m71 && overall_m72 && overall_m73 && overall_m74 && overall_m75 && overall_m76 && overall_m79 && overall_m83 && overall_m84 && overall_m85 && overall_m86 && overall_m87 && overall_m88 && overall_m89 && overall_m90 && overall_m91 && overall_m92 && overall_m93 && overall_m94 && overall_m95 && overall_m96 && overall_m97 && overall_m98 && overall_m99 && overall_m101 && overall_m102 && overall_m103 && overall_m107 && overall_m108 && overall_m109 && overall_m110 && overall_m111 && overall_m112 && overall_m113 { 0 } else { 1 });
    }
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    loop {}
}
