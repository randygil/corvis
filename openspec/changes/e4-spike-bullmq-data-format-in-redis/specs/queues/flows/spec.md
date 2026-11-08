# Spec Delta

## ADDED Requirements

### Requirement: Flows share Node BullMQ's dependency format
`FlowProducer::add(tree)` SHALL store a parent and its children (possibly in other queues with the same prefix) as
Node BullMQ 6 `FlowProducer.add` does: parent in `waiting-children`, children with `parentKey`/`parent`, the parent's
`:dependencies`, `:processed`, `:failed` and `:unsuccessful` keys, and UUID ids for nodes without `job_id`. A Corvis
worker processing a parent SHALL be able to read its children's return values.

#### Scenario: Corvis flow, Node workers
- **WHEN** Corvis adds a parent in `reports` with two children in `pages`, and Node workers process both queues
- **THEN** the children run first, the parent then runs, and Node `job.getChildrenValues()` in the parent returns both
  children's return values

#### Scenario: Node flow, Corvis workers
- **WHEN** Node `FlowProducer` adds the same tree and Corvis workers process both queues
- **THEN** the parent's Corvis processor reads both children's return values, and the final normalized state equals
  the Node-only reference run

### Requirement: Flow producers are injected
`#[inject_flow_producer]` on an `#[injectable]` field SHALL inject a flow producer that uses the connection and
prefix of `BullModule::for_root`.

#### Scenario: Inject a flow producer
- **WHEN** a service has `#[inject_flow_producer] flows: FlowProducer` and the application imports
  `BullModule::for_root` with prefix `{app}`
- **THEN** flows added by the service are stored under `{app}:<queue>:*`

### Requirement: Child failure policies match Node BullMQ
The child options `fail_parent_on_failure`, `continue_parent_on_failure`, `ignore_dependency_on_failure` and
`remove_dependency_on_failure` SHALL change the parent exactly as in Node BullMQ when a child fails for good, and they
MUST be stored with Node's abbreviated keys (`fpof`, `cpof`, `idof`, `rdof`).

#### Scenario: Fail parent on failure
- **WHEN** a child with `fail_parent_on_failure` exhausts its attempts in a Corvis worker
- **THEN** the parent moves to `failed` with the same `failedReason` and `defa` handling as when a Node worker fails
  that child
