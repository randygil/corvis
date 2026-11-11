## ADDED Requirements

### Requirement: Owned parse entry points
`ZodSchema` SHALL provide `parse_owned(Value)` and `parse_owned_opt(Option<Value>, &ParseOptions)`, and `z` SHALL provide
`parse_val_owned`, which take the input by value and do not copy it before validating. They SHALL give the same output
and errors as `parse`, `parse_opt` and `parse_val`. `ZodValidationPipe` and the serializer SHALL use the owned path.

#### Scenario: Same result
- **WHEN** a schema parses the same input with `parse(&value)` and with `parse_owned(value.clone())`
- **THEN** both return the same output, or the same issues in the same order

#### Scenario: The pipe does not copy the body
- **WHEN** `ZodValidationPipe` validates the valid `dto-ok` body of the benchmark
- **THEN** the bytes allocated are within the bound set for that case in `tests/zod_hot_path_heap.rs`, a bound that the
  code before this change exceeds

### Requirement: Checks and refinements do not copy a valid value
A check SHALL copy the value it checks only when it reports an issue, and `.refine()`, `.superRefine()`, `z.custom()` and
`z.preprocess()` SHALL hand their callback a borrowed value; `.transform()`, `.overwrite()` and codecs SHALL move the
value into their callback. Issues, messages and the input reported with `reportInput` SHALL stay as recorded.

#### Scenario: A refinement over a large body
- **WHEN** `ZodValidationPipe` validates a 1 MiB body with `.refine()` and `.superRefine()` on the root object and both
  pass
- **THEN** the bytes allocated are less than half of one copy of the body

#### Scenario: reportInput still reports the value
- **WHEN** a `.superRefine()` adds an issue without an explicit `input` and the parse runs with `reportInput`
- **THEN** the issue's `input` is the value the refinement received

### Requirement: Recursive unions cost linear in their input
A union SHALL NOT copy its input for an option that fails on the input's JSON type alone, and SHALL run its only
remaining option on the input itself when no issue input and no failed value can be observed (`reportInput`, error maps,
`.catch()` callbacks, `when` callbacks). Outputs and issues SHALL be the same as with a copy per option.

#### Scenario: Depth does not multiply the cost
- **WHEN** a recursive union schema parses valid documents of depth 12 and 24 with the same number of nodes per level
- **THEN** the bytes allocated per node at depth 24 are at most 2.2 times those at depth 12

#### Scenario: Observers see the input
- **WHEN** `.catch_fn` wraps a recursive union whose only remaining option aborts
- **THEN** the callback's `ctx.input` is the union's input

#### Scenario: Both paths agree
- **WHEN** the property test parses random JSON values with union-heavy schemas with the copy paths forced and not forced
- **THEN** the outputs and the finalized issues are equal, with `reportInput` off and on and with a global error map
