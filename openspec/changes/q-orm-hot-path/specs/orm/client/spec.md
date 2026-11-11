# Spec Delta

## ADDED Requirements

### Requirement: createMany writes rows in multi-row statements
`createMany`, `createManyAndReturn` and nested `createMany` SHALL insert consecutive rows that set the same columns with
multi-row `INSERT` statements of at most 1,000 rows, within the dialect's parameter limit, inside one transaction. The
returned rows, their order, the counts and the errors MUST be those of inserting the rows one by one.

#### Scenario: Ten thousand rows
- **WHEN** `createMany` inserts 10,000 rows that set the same columns
- **THEN** it sends at most 10 `INSERT` statements, inside one transaction, and returns `{ count: 10000 }`

#### Scenario: Duplicate in the middle of a batch
- **WHEN** row 1,500 of a `createMany` without `skipDuplicates` violates a unique constraint
- **THEN** the call fails with the same `P2002` code, message and meta as a one-by-one insert, and no row is inserted

#### Scenario: Create many and return keeps the order
- **WHEN** `createManyAndReturn` inserts 10,000 rows
- **THEN** the rows come back in input order, equal to those of a one-by-one insert

### Requirement: Paginated includes take one statement per relation
An include with `take` or `skip` and without `cursor` or `distinct` SHALL load the children of every parent in one
statement per relation (one per chunk of compound parent keys), with the same children, order and nesting as one query per
parent.

#### Scenario: Hundred parents
- **WHEN** a `findMany` of 100 users includes `posts: { take: 3, skip: 1, orderBy: { id: "desc" } }`
- **THEN** two statements are sent, and each user carries the posts a query of its own would return

#### Scenario: Negative take
- **WHEN** the include uses `take: -2`
- **THEN** each parent carries its last two children in the include's order, as recorded from the original client
