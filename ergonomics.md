# ergonomics — evolving the language downhill on token cost

The working hypothesis: if models author most code, language design
becomes an empirical optimization — mutate the surface, measure the
cost of producing *verified-correct* programs, keep what descends. The
verifier and the differential suite hold semantics fixed while the
surface moves; the suite going green is the fitness oracle.

## The metric

Not "tokens to write" — **expected tokens to verified-correct**:

    E[cost] = write + read-context + Σ (retry_i × debug_i)

Three consequences:

- **The retry tail dominates.** One failed attempt costs more than all
  the ceremony in a correct one. Terse-but-treacherous loses to
  verbose-but-first-try. Features are judged on error-rate as much as
  length.
- **Verbosity taxes twice**: once at writing, then forever as context
  cost on every later edit of the same code. Edit tasks belong in the
  battery, not just greenfield ones.
- **Types are cheap insurance.** Explicit types on every line act as
  continuous re-grounding during generation; removing them to save
  tokens would likely *raise* E[cost] via the retry tail. (Testable.)

Two structural observations about why flat SSA suits autoregressive
authorship: each line depends only on names already in the transcript
(no nesting debt, the transcript is the scratchpad), and named
single-assignment lines are ideal retrieval targets when reading.

## Experiment 1: literal operands + expression RHS (landed)

Predicted cheapest wins from observed authorship cost: constant
ceremony (`%one: int = iconst 1` — a third of the rational library's
lines) and pure-arithmetic density (a polynomial took 8 lines).

Mutation: literal operands (arithmetic, comparisons, loop inits; typed
from context) and expression right-hand sides (C precedence, desugared
to flat SSA at parse time; opcode family from the declared type, so `/`
is signed/unsigned/float division by the type alone). Semantics
untouched — one-way parse sugar, printer still prints flat form.

Static measurement (corpus files converted, suite as referee —
197/197 across native/wasm/riscv/arm-qemu, all policies, softfloat,
scalar=rat after conversion):

| file | code lines | bytes | syntactic atoms |
|---|---|---|---|
| suite/scalar.ssa (expression-heavy) | −25.9% | −23.0% | −26.2% |
| lib/rational.ssa (call/control-heavy) | −3.8% | −6.8% | −6.3% |

Reading of the split: the sugar erases exactly the ceremony it targets
— arithmetic — and nothing else. The rational library's remaining cost
is calls, pack/extract, if/yield, and above all the NaR-guard prologue
(6 lines × 8 functions), which is the next mutation candidate (guard /
early-return sugar).

Incidental datum on the retry tail: during this experiment the one
test failure was a wrong *expected value* in a hand-written directive
(`horner 2 -> 49`; the compiler correctly said 37). The language
didn't cause the retry; arithmetic-in-the-head did — which is exactly
the class of token expenditure that expression sugar (and directives
computed by a reference implementation) exists to remove.

## Experiment 2: comparisons + guard sugar (landed)

Driven by experiment 1's residue: the rational library's remaining
ceremony was NaR-guard prologues (6 lines × 8 functions) and `icmp`
lines. Mutations: comparison operators (`< <= > >= == !=`, one
non-associative level, operand type from the value side, icmp/fcmp by
type), literals in `break`/`continue`/`yield`/`ret` (typed positionally
from what they feed), `ret call @f(...)`, and single-line blocks. A
guard collapsed from six lines to one:

    if %ar > %numlim { ret call @rat_nar() }

Cumulative measurement vs the pre-sugar corpus (same referees, now
203/203):

| file | code lines | bytes | atoms |
|---|---|---|---|
| suite/scalar.ssa | −29.4% | −25.5% | −27.9% |
| lib/rational.ssa | −32.1% | −21.0% | −18.8% |

The split from experiment 1 closed: guard sugar targeted exactly the
ceremony that expression sugar couldn't reach.

## Experiment 3: call sugar via whole-module signatures (landed)

Experiment 2's residue was bind-then-test pairs. The blocker was
parse-order: call sugar needs callee types before the callee parses.
Fix: parsing became two-phase — type declarations and all function
signatures first, bodies second — after which three forms fall out:

- literals in call arguments (typed by the callee's parameters):
  `ret call @clamp(%x, 0, 100)`
- `call` as an expression atom (single-result callees):
  `%r: u1 = call @haltstep(%x) == 4`
- `call` as an if condition, giving the NaR guard its terminal form:

      if call @rat_is_nar(%x) { ret call @rat_nar() }

Also landed en route: terminator operands are full expressions
(`ret %x / 2`, `ret 3 * %x + 1`) — a form I reached for *unprompted*
while writing the smoke tests, which is its own kind of evidence.

Cumulative vs the pre-sugar corpus (207/207 everywhere, including
rat-on-softfloat-f32):

| file | code lines | bytes | atoms |
|---|---|---|---|
| suite/scalar.ssa | −29% | −26% | −28% |
| lib/rational.ssa | −38% | −25% | −24% |

Residue now: pack/extract chains, the value-yielding if form, and
multi-result call bindings — all structural rather than ceremonial.
The curve is flattening; the next big wins likely need the live-battery
evidence rather than static intuition.

## Experiment 4: the `call` keyword retires (landed)

User-observed redundancy: `@` already marks globals, so `@f(args)` is
unambiguous in every position — binding, statement, expression atom,
condition, `ret`. The keyword survives as accepted legacy; the printer
and corpus use the bare form. The guard reaches its final shape:

    if @rat_is_nar(%x) { ret @rat_nar() }

Measured: −4% atoms on the call-dense rational files. Small, but the
kind that compounds — every future call in every future file.

## The live half (not yet run)

Static atom counts are a proxy. The real experiment: a task battery —
write-new (function from directive spec), edit-existing (change
behavior of a corpus function), find-the-bug (seeded defect) — run by a
model against the suite, measuring total tokens until green, pass@1,
and retry count; language variants compared on the same battery; run
across different models to separate structural ergonomics from
training-prior familiarity (prior familiarity is real cost too, but it
shouldn't silently steer the design).

## Mutation log

- 2026-08-23: literal operands + expression RHS. Landed. −26% atoms on
  expression-heavy code, −6% on call/control-heavy; zero semantic
  change; suite green everywhere.
- 2026-08-23: comparisons, terminator literals, ret-call, single-line
  blocks. Landed. Cumulative −28%/−19% atoms on the two corpus files;
  the guard prologue is one line; suite green everywhere.
- 2026-08-23: two-phase parse (module signatures first); literal call
  args, call-in-expression, call-in-condition, expression terminator
  operands. Landed. Cumulative −28%/−24% atoms, −29%/−38% lines; suite
  green everywhere.
- 2026-08-24: `call` keyword retired (user-spotted redundancy): `@f()`
  is the call form everywhere, printer canonical. −4% atoms on
  call-dense files.
