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
ceremony that expression sugar couldn't reach. Remaining residue in the
library: `call`+bind pairs (no literals in call args — needs callee
signatures at parse time), pack/extract, and the value-yielding if
form.

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
