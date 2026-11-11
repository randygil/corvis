# Design

## Context

Source: the engineering-quality review of the validation library recorded in the issue filed from
`quality/q-validation-hot-path.issue.md`. The reviewer read the code only. Every line below was read on `dev` at
`d46758a` (2026-11-11); `q-validation-correctness` lands first and edits some of the same functions (the zod 3 checks,
the intersection's throw, the pipe's error mapping), so **task 1.1 re-checks every line against `dev` after that change
is merged** and records moved lines in "Implementation notes". The decisions depend on what is copied and when, not on
line numbers.

Who pays: the three port apps run zod 4 mode only (consi builds a `ZodValidationPipe` per route over `parse_opt`, facten
uses the global `ZodValidationPipe` with `zod_dto!` types), so every decision targets zod 4 mode and the pipe path. zod 3
mode is not optimized.

How values move today: `Payload` owns a `Val`; objects, arrays, tuples and records move each child value into the
child's payload (`compound.rs:228-240,1036-1044`), which is why recursion is linear today. The copies below are the
places that break that rule.

### Findings as checked

| Id | Finding | Code now (`d46758a`) | Verdict |
|---|---|---|---|
| D2a | Length checks copy the value before the condition | `length_check` (`compound.rs:146`: `let input = p.value.clone()` then the match) | Confirmed: `z.array(x).min(1).max(100)` copies the whole array twice per valid parse |
| D2b | String checks copy the string twice per check | `ZString::apply_v4` (`types.rs:920`, `958`: `p.value.clone()` and `as_str().map(str::to_string)`), `Fmt::check_v4` (`295`), `url_check` (`343`) | Confirmed. Siblings found (cheap values, same pattern): `ZNumber::apply_v4` (`1511`), the bigint checks (`1952`), the date checks (`2252`) |
| D2c | Callbacks copy the value | `.refine` (`wrappers.rs:736`: `to_json()`), `.superRefine` (`813-814`: `to_json()` and `clone()`), `transform` (`964-965`: `clone()` then `to_json()`), `.overwrite` (`920`: `to_json_or_null()`), `codec` (`1238-1239`) | Confirmed. Siblings found: `z.preprocess` (`1158`: `to_json()`), `z.custom` (`1394`: `to_json()`) |
| D3a | Unions copy the input per option | `union_v4` (`compound.rs:1168`), `ZXor` (`1266`) | Confirmed. For `z.lazy(() => z.union([...]))` every level copies its subtree for every option tried before the one that matches, and for that one too: the cost is the input's size times its depth (up to `max_depth`, 128) |
| D3b | Intersections copy twice, and the merge copies each subtree per level | `compound.rs:1555-1556`; `merge_vals` (`1483-1527`: `x.clone()`, then `Val::Json(xv.clone())` and `Val::Json(yv.clone())` per common key, recursively) | Confirmed. The two input copies are needed (Decision 5); the merge copies are not |
| S1 | `ZEnum` recomputes its values per parse | `run_v4` (`types.rs:2519-2526`) calls `enum_values` (`2454-2464`: a `Vec<f64>`, `string_to_number` per key, an O(E²) `contains`, a clone of every value) and `Val::Json(v.clone())` per comparison (`2521`); `values()` (`2527-2529`) and `pattern()` the same | Confirmed |
| S2a | The parse API copies the whole input | `Val::from_opt` (`core.rs:149-154`: `v.clone()`); the pipe owns the value and passes `&value` (`pipe.rs:185-195`) | Confirmed |
| S2b | The serializer copies the response twice | `pipe.rs:369-374` (`Body::Json(v) => v.clone()`, then `parse_with(&value)` copies again) | Confirmed. `ZodSerializerDto::serialize` also builds `Schema::new(entry.schema.array())` per call for array DTOs (`pipe.rs:327-328`), found while confirming |
| S2c | A concrete schema type clones itself on every parse | `impl<T: ZodType + Clone> ZodSchema for T { fn to_schema(&self) -> Schema { Schema::new(self.clone()) } }` (`mod.rs:909-913`), and every parse method starts with `self.to_schema()` (`645-676`) | Confirmed: a `ZObject` used directly clones its shape and allocates an `Arc` per parse. The registry identity part is narrowed: `register()` returns the registered handle, which is the one to keep (rustdoc in `q-validation-api-hygiene`) |
| N1 | Each parse takes the configuration lock twice and allocates | `RunCtx::new` (`core.rs:503-515`: `get_config()` read-locks, clones `ZodConfig`, `Arc::new` of a snapshot), `version()` (`113-116`) read-locks again from `parse_val` (`mod.rs:609`) | Confirmed: two read acquisitions (an atomic read-modify-write on one shared cache line each) and one allocation per parse |
| N2 | Each issue copies its input even when nothing reads it | `issue()` (`types.rs:117-119`), `type_issue` (`compound.rs:16-18`), `invalid_union` (`1184`) | Confirmed, on the failure path only. Decision 10 measures it before deciding |
| N3 | Strict objects scan the shape per key; zod 3 computes extra keys in strip mode | `is_extra` (`compound.rs:1030`, O(shape) per input key, used at `1033` and `1047-1049`); `catchall_schema` allocates a `ZNever`/`ZAny` per parse (`930-937`); zod 3 (`983`) computes `extra` always | Confirmed. The zod 3 part is kept out (zod 3 not optimized), except that it is a one-line guard (Decision 9) |

No finding of this group is dropped. S2c is narrowed (identity) and N2 is deferred to a measurement.

## Goals / Non-Goals

**Goals:**

- Measure first, then remove the copies a successful parse does not need, each against a number.
- A recursive union whose cost is linear in the input.
- An owned path from the request body to the handler and from the handler's result to the response.
- No change to any output: issues, messages, key order and values stay byte for byte.

**Non-Goals:**

- Sharing values between payloads (reference-counted JSON): a rewrite of `Val` and of every node.
- zod 3 mode, `ZodGuard`, the failure path (Decision 10).

## Decisions

### 1. A validation benchmark, run before and after

`openspec/specs/bench` has `bench/synthetic` (HTTP throughput against Node) and `bench/port-vs-original` (whole apps);
`q-validation-correctness` adds none. Validation is CPU work inside one call, and the HTTP benchmarks would bury it under
the server's own costs, so the new capability `bench/validation` is in-process:

- `bench/validation/` is a workspace member (`publish = false`) with one binary, `corvis-validation-bench`, depending on
  `corvis`, `serde_json` and `tokio` (a current-thread runtime to drive the async pipe). A counting global allocator
  (allocations and bytes, relaxed atomics) measures each scenario alone.
- Each scenario first checks its result (the expected success value, or the expected error body), then warms up for one
  second, then makes five runs of N parses (N chosen so a run lasts at least one second). It reports the median time per
  parse with min and max, allocations and bytes per parse (from the counters over one run), and the **clone ratio**:
  bytes per parse divided by the bytes one `Value::clone` of the input allocates (measured by the bench itself).
- `bench/validation/run.sh --label before|after` builds with `cargo build --release -p corvis-validation-bench` in the
  workspace target directory (no target directory of its own), `SKIP_BUILD=1` reuses the binary, writes
  `bench/validation/results/<label>.json` (ignored by git) with the machine, rustc version, commit and parameters, and
  prints a Markdown table.

The DTO of `dto-ok` (about 6 KiB of JSON), written once in `bench/validation/src/main.rs`:

```
id: uuid, email: string().min(3).max(255).email(), name: string().trim().min(1).max(100),
role: enum(["admin","editor","viewer","guest","owner"]), tags: array(string().min(1).max(30)).max(20) (10 items),
address: object({ street: string().min(1).max(200), city: string().min(1).max(100),
                  zip: string().regex(/^[0-9]{5}$/), country: enum([... 8 codes]) }),
items: array(object({ sku: string().regex(/^[A-Z]{3}-[0-9]{4}$/), qty: int().positive(),
                      price: number().nonnegative().multipleOf(0.01), kind: union([literal("a"), literal("b")]) })) (20 items),
contact: union([string().email(), number().int()]),
payment: discriminatedUnion("type", [object({ type: literal("card"), last4: string().length(4) }),
                                     object({ type: literal("cash") })]),
metadata: record(string(), string()).optional(), createdAt: iso.datetime()
```

| Scenario | Entry point | What it exercises |
|---|---|---|
| `dto-ok` | `ZodValidationPipe::transform` (owned value, the app path) and `parse(&Value)`, two rows | the realistic DTO, valid; the bench prints how many string checks pass per parse |
| `dto-fail` | the pipe | the same DTO with 10 invalid fields: issues and the 400 body |
| `recursive-union` | the pipe | `node = lazy(union([string(), number(), array(node), object({ name: string(), children: array(node) })]))`, a valid document 24 levels deep, about 2,000 nodes |
| `intersection` | the pipe | `object({...}).and(object({...}))` over 2 KiB, nested objects on both sides |
| `refine-root` | the pipe | `object({ rows: array(string()) }).refine(..).superRefine(..)` over 1 MiB of rows |
| `enum-heavy` | the pipe | an object of 30 enum fields of 8 values each |
| `strict-wide` | the pipe | a strict object of 64 keys and an input with exactly those keys |
| `serialize` | `ZodSerializerDto::serialize` | the response path for the `dto-ok` output |
| `threads` | the pipe on 1 and 8 threads | `dto-ok`, aggregate parses per second |

The before run is made at the start (task 2.3) on `dev` with `q-validation-correctness` merged, the after run at the end
(task 10.1), same machine and parameters; both tables go to "Implementation notes". The benchmark informs; no test
compares its timings.

Alternatives considered: Criterion (a new dependency, and the repository has none); routes in `bench/corvis-app` driven by
`oha` (the HTTP stack would dominate the numbers and the copies would not show).

### 2. Checks build their issue input when they push an issue

`length_check`, `ZString::apply_v4`, `Fmt::check_v4`, `url_check` and the number, bigint and date checks stop copying the
value up front: they read it by reference (`p.value.as_str()`, `p.value.length()`) and clone it only inside the branch
that pushes an issue. `ZString::apply_v4` borrows the string instead of `to_string()`; where a check must read the string
while pushing to `p.issues`, it computes the outcome first and then builds the issue. `TypeErrors::v4_map` (which
allocates an error-map closure when a zod 3 style `required_error` is set) is called only when an issue is pushed.

Number it must move (`dto-ok`, pipe row): allocations per parse fall by at least twice the number of string checks that
pass (printed by the bench), and bytes per parse by at least twice the bytes of those strings. Verified also by
`tests/zod_hot_path_heap.rs` (Decision 11) and by the fixtures, unchanged.

### 3. Callbacks borrow or take the value

- `.refine()`, `z.custom()`, `.superRefine()`: the closure receives `&Value` borrowed from `p.value` when it is
  `Val::Json` (a temporary is built only for `Date`, `BigInt` or a non-finite number); `refine_issue` clones only when the
  check fails.
- `RefinementCtx` borrows its input: `RefTarget::V4` holds `input: &'a Val` next to `issues: &'a mut Vec<RawIssue>`
  (disjoint fields of the payload), and clones it only when `add`/`add_issue` pushes an issue without an explicit
  `input`. Its public signature does not change.
- `z.preprocess()`: the closure receives `Option<&Value>` borrowed from the payload.
- `.transform()` and `.overwrite()`: the value is taken out of the payload (`std::mem::replace`) and moved into the
  closure. For `.transform()` the context of `ctx.addIssue` needs the value before the transform as the issue's input;
  it is cloned only when an issue input is observable (Decision 4), otherwise the context gets the type stub of the
  value, which gives the same messages.
- `codec`: the decode/encode closure receives the value moved out of the payload, as `.transform()`.

Number it must move: `refine-root` clone ratio, from at least 3 before (pipe copy, refine copy, two superRefine copies)
to at most 0.5 after (with Decision 7). Tests: the fixtures (refine, superRefine, transform, overwrite, codec, preprocess
and custom cases), and a unit test that a `superRefine` adding an issue with `reportInput` reports the pre-refinement
value.

### 4. Unions without speculative copies

Two facts make this safe to change. In zod a union passes the *same* JavaScript value to every option; Corvis copies it
because a `Payload` owns its value. And after an option fails, what the union returns differs from what it would return
with copies only in places that can be listed:

- **Issue inputs** are observable only through `reportInput` and error maps (per-schema, per-parse, `customError`,
  `localeError`); the `en` messages read only the input's type. `RunCtx` gains `inputs_observable`, computed once per
  parse: `report_input` or any of those maps is set.
- **The value of a failed payload** is observable only through `.catch((ctx) => ..)` (`ctx.input`) and user `when`
  callbacks (refinements and checks with `when`, which run on failed payloads). `RunCtx` gains a counter of value
  observers, incremented by `ZCatchFn` and by every node that has a user `when` while its inner schema runs.

With these:

- **Type stubs.** `Val::type_stub()` returns a value of the same `parsed_type` (`undefined`, `null`, `false`, `0`, NaN,
  `""`, `[]`, `{}`, a `Date` or a `BigInt` of 0). An issue built when inputs are not observable may hold the stub instead
  of a copy; it gives the same message and never reaches the output.
- **Quick rejection by type.** A crate-internal method on `ZodType`, `type_rejects(&self, v: &Val) -> bool` (default
  `false`), is true when the node's parse would fail with its single type issue for that JSON container kind: `ZString`,
  `ZNumber`, `ZBoolean`, `ZBigInt`, `ZDate` without `coerce`; `ZNull`, `ZLiteral`, `ZEnum`; `ZObject` for arrays,
  `ZArray` and `ZTuple` for objects; forwarded by `ZOptional`, `ZNullable`, `ZReadonly`, `ZMeta` and `ZLazy`. For an
  array or object input, `union_v4` and `ZXor` answer such an option with its type issue (a crate-internal
  `type_issue_for`, built with the stub) without running it, when inputs are not observable and the option has no
  instance error map; otherwise they run it on a copy as today.
- **A single remaining option runs on the value.** When exactly one option is left after quick rejection, the direction
  is forward, inputs are not observable and there is no value observer, `union_v4` runs that option on the payload's own
  value. If it succeeds or fails without aborting, the result is the one the copy would have given. If it aborts, the
  `invalid_union` issue gets the stub as input and the returned payload carries the option's value, which nothing can
  observe under those conditions.
- `ZXor` uses quick rejection only (it must run every remaining option).

The old paths stay reachable through a hidden `RunCtx` switch (`#[doc(hidden)]`, documented as test-only) that disables
quick rejection and the single-option move. `tests/zod_union_paths.rs` is a proptest property over random JSON values
(nesting up to 8, mixed kinds) and union-heavy schemas (the `recursive-union` node, unions of objects and arrays, nested
unions with `optional`, `nullable`, `lazy`, a `.catch()` and a refinement with `when`): parse with the switch on and off,
with `reportInput` off and on, and with a global error map; the outputs and the finalized issues JSON are equal.

Number it must move: `recursive-union` clone ratio, from at least 10 before (depth 24) to at most 2 after; `dto-ok`
allocations fall further by the copies of the `contact` and `kind` unions. Also pinned by `zod_hot_path_heap.rs`: the
recursive union at depth 12 and 24 allocates at most 2.2 times as much at 24 as at 12 per node.

Alternative considered: give the last option the payload by value in every case (the reviewer's proposal). Rejected: when
that option aborts, the union returns a payload whose value is no longer the input, which `.catch((ctx) => ..)` and `when`
callbacks would see; the conditions above are what makes the move invisible.

### 5. Intersections: two copies, no copies in the merge

Both sides of an intersection need the input, and after an abort the intersection returns the original value (observable
by the same means as in Decision 4), so the two input copies stay. `merge_vals` takes both values by value
(`merge_vals(a: Val, b: Val)`): object entries are moved out of the left map and merged with the right entry moved out of
the right map, arrays are zipped by value, equal primitives return the left one; the error path is built as today. The
key order of the merged object (`js_map`) is unchanged.

Number it must move: `intersection` clone ratio from at least 4 before to at most 2.5 after. Tests: the intersection
fixtures (including the conflict case, now an `Err` with `thrown()`), and a unit test merging two nested objects of
depth 10 whose bytes allocated are at most those of one copy of the inputs.

### 6. Enum values computed once

`ZEnum` stores its values (`Vec<Val>`, computed by `getEnumValues`) when it is built: in `new`, `from_entries`, `extract`
and `exclude`. `run_v4` compares the payload with the stored values without cloning (`same_value_zero` on references);
`values()`, `pattern()`, `literal_values()` and `json_schema()` read them. `enum_values()` keeps its public signature and
clones the stored values.

Number it must move: `enum-heavy` allocations per parse fall by at least 8 per enum field (240 for the scenario).

### 7. Owned parse entry points; schemas that do not clone themselves

- Public: `ZodSchema::parse_owned(&self, value: Value) -> Result<Value, ZodError>`,
  `ZodSchema::parse_owned_opt(&self, value: Option<Value>, opts: &ParseOptions) -> Result<Value, ZodError>` and
  `z::parse_val_owned(&Schema, Option<Value>, &ParseOptions) -> Result<Val, ZodError>`. The existing by-reference methods
  stay and copy once, as today.
- `ZodValidationPipe::transform` parses the value it owns (`null` still means undefined); `ZodSerializerInterceptor` takes
  the JSON body out of the response through `body_mut()` (`std::mem::replace` with `Body::Empty`), serializes it by value
  and puts the output back; `ZodSerializerDto::serialize` keeps its public by-reference signature and gains a
  crate-internal owned variant. The array schema of an array DTO is built once, when it is registered.
- Concrete types: `ZodSchema` gains a hidden accessor returning `&dyn ZodType` (the blanket implementation returns
  `self`, `Schema` returns its node); the parse methods use it instead of `to_schema()`, so a `ZObject` used directly is
  not cloned per parse.

Numbers it must move: `dto-ok` pipe row clone ratio falls by at least 1.0; `serialize` clone ratio by at least 2.0; the
`dto-ok` `parse(&Value)` row allocates one `Arc` less when the schema is a `ZObject`.

### 8. The configuration read without a lock

`config()` and `configure()` increment a global `AtomicU64` generation under the write lock (release ordering). Each
thread keeps, in a `thread_local!`, the generation it last saw and its own `Arc` of a snapshot (version, error maps,
`date_strings`, `max_depth`). `RunCtx::new` and `version()` load the generation (acquire); if it matches the thread's, they
use the thread's snapshot (an `Arc` clone of a thread-private allocation: no shared cache line, no allocation); otherwise
they take the read lock once and rebuild it. `with_version` keeps its thread override.

Tests: a parse on a thread that already parsed sees a `configure` made on another thread (generation ordering, joined
threads, no sleep); `z::config` then `get_config` round-trips; the zod_security configuration test passes unchanged.
Number it must move: allocations per parse fall by one in every scenario; the 8-thread rate of `threads` is recorded,
not asserted.

### 9. Strict objects: a key set and shared catch-all schemas

- `ZObject` keeps a `OnceLock<HashSet<String>>` of its shape keys, filled at the first strict or catch-all parse when the
  shape has more than 16 keys (a linear scan stays below that); every builder that adds or removes keys (`field`,
  `extend`, `safe_extend`, `merge`, `pick`, `omit`) resets it. A unit test fills the set, applies each of those builders
  and checks the set against the new shape.
- `catchall_schema` returns clones of two `static` schemas (`ZNever` for strict, `ZAny` for passthrough) instead of
  allocating them per parse.
- zod 3 mode computes its extra keys only when the unknown-key policy is not strip (`compound.rs:983`).

Number it must move: `strict-wide` allocations per parse fall by one, and its time per parse is recorded.

### 10. Issue inputs on the failure path: measured, not changed here

Copying the input into every issue costs only on the failure path, which is bounded by the request size limits upstream,
and storing stubs everywhere would thread `RunCtx` through every check of every node. The `dto-fail` scenario reports
bytes per failed parse. If the after run shows more than three input copies per failed parse, a follow-up issue applies the
stub rule of Decision 4 to every issue; otherwise the finding is closed with that number.

### 11. A heap test that holds the gains

`crates/corvis/tests/zod_hot_path_heap.rs`, a binary with a counting allocator and a single test function that runs its
cases in sequence (the allocator counts the whole process): `dto-ok` through `ZodValidationPipe` within a byte bound, the
recursive union depth ratio of Decision 4, `refine-root` under 0.5 input copies, `enum-heavy` within an allocation bound.
Each bound is chosen from the before and after numbers, and the task that adds it checks that it fails on the code before
the change.

## Security invariants this change comes near

From the earlier security audit of the module (nesting limit, quadratic recursion) and from `q-validation-correctness`:

| Invariant | Decisions near it | Pinned by |
|---|---|---|
| Recursive schemas stop at `max_depth`; the nesting issue has an undefined input and is not wrapped in `invalid_union` | 4 (unions answer some options without running them; the single option runs on the value) | `zod_security.rs` nesting tests unchanged; `too_deep_issue_is_not_wrapped_in_invalid_union` with quick rejection on; the property test of Decision 4 includes inputs deeper than a small `max_depth` |
| Recursive traversal is linear | 2 to 5 | `tests/zod_linear_traversal.rs` (from the first change) and `zod_hot_path_heap.rs` |
| `reportInput` and error maps see the real input | 3, 4 (stubs only when not observable) | the property test with `reportInput` on and with a global error map; the `error.reportInput` and `error.param.fn` fixtures |
| `.catch((ctx) => ..)` sees the input it was given | 4 (value observers) | a unit test with `catch_fn` over a recursive union whose last option aborts |
| What the original throws is returned as `Err` and unwinds through depth guards | 5 (`merge_vals` keeps the error path) | `tests/zod_thrown.rs` |
| The pipe answers the same bodies | 7 (owned path) | `zod_pipe.rs` fixtures and HTTP test |

## Behavior visible to applications

| What | Before | After | Consequence |
|---|---|---|---|
| `parse_owned`, `parse_owned_opt`, `parse_val_owned` | Absent | New entry points that take the value | Additive |
| Any parse output, issue, message or key order | — | Unchanged | Pinned by the fixtures and the property test |
| A `ZObject` used directly with `parse` | Cloned per parse | Not cloned | Faster; same identity semantics for the registry |
| A configuration change made with `z::config`/`configure` | Seen by the next parse (lock) | Seen by the next parse on every thread (generation) | Same rule, no lock on the hot path |

## Risks / Trade-offs

- The union conditions of Decision 4 miss an observer → the property test runs both paths over schemas with every
  observer kind; a new observer kind added later must increment the counter (a comment on the counter lists them, and the
  security review checks the list).
- `type_rejects` disagrees with a node's real parse → it is implemented only for nodes whose type check is the first thing
  their parse does, and the property test compares against running the option.
- The thread-local snapshot outlives a configuration change → refreshed by the generation on the next parse; a parse
  already running keeps the configuration it started with, as today.
- `q-validation-correctness` changes the same files → this change starts after it is merged (task 1.1).

## Open questions

1. Make the single-option move unconditional by giving `.catch()` and `when` callbacks the value through a different
   channel? It would remove the observer counter but change what those callbacks receive.
2. Expose the test-only union switch publicly as an escape hatch?
3. Apply the stub rule to every issue now (finding N2)?
4. Name of the owned methods: `parse_owned` or `parse_value`?

## Review notes

These settle the open questions and override the design where they differ.

1. No: the counter keeps the callbacks' contract exactly as zod's; the move stays conditional.
2. No: it stays `#[doc(hidden)]`, documented as test-only; it is not part of the API.
3. No: Decision 10 decides from the `dto-fail` number after this change.
4. `parse_owned` and `parse_owned_opt`, beside `parse` and `parse_opt`, and `parse_val_owned` beside `parse_val`.
5. Starts after `q-validation-correctness` is merged; task 1.1 is mandatory.
6. The security review is mandatory before archiving: the observer list and the stub rule, quick rejection and the nesting
   limit, the owned pipe and serializer paths, the configuration snapshot.
