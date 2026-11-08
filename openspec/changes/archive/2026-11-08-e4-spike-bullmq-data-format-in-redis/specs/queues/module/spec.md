# Spec Delta

## ADDED Requirements

### Requirement: Root module configures the Redis connection
`BullModule::for_root(options)` and `BullModule::for_root_async(imports, factory)` SHALL register a global Redis
connection configuration used by every queue, worker and listener of the application, accepting either a URL
(`redis://`, `rediss://`) or typed fields (host, port, username, password, db, TLS), and a key prefix (default
`bull`). The queues feature MUST be optional and off by default. Connection errors and logs MUST NOT include the
password.

#### Scenario: URL with credentials
- **WHEN** the application imports `BullModule::for_root` with `redis://user:secret@127.0.0.1:6379/2` and Redis is
  reachable
- **THEN** queues use database 2 with user `user`, and no log line or error message contains `secret`

#### Scenario: Unreachable Redis at bootstrap
- **WHEN** Redis is not reachable when the application starts
- **THEN** the behavior (fail the bootstrap or retry in the background) matches what #50 documents, and the error
  message names the host and port without the password

#### Scenario: Redis older than the minimum
- **WHEN** the server reports a Redis version below 6.2
- **THEN** the queues module refuses to start with a message naming the found and the required version

### Requirement: Queues are registered and injected by name
`BullModule::register_queue([...])` SHALL register one queue provider per name, and a `#[injectable]` field marked
`#[inject_queue("name")]` SHALL receive that queue. Injecting a queue that was not registered MUST fail the bootstrap
with a message that names the queue and `register_queue`.

#### Scenario: Inject a registered queue
- **WHEN** `EmailsModule` imports `BullModule::register_queue(["emails"])` and `EmailsService` has
  `#[inject_queue("emails")] queue: Queue`
- **THEN** `EmailsService` adds jobs to `<prefix>:emails`

#### Scenario: Inject an unregistered queue
- **WHEN** a provider injects `#[inject_queue("reports")]` and no module registered `reports`
- **THEN** bootstrap fails with an error naming `reports` and `BullModule::register_queue`

### Requirement: Connections close on shutdown
Queues, workers and queue event listeners SHALL close their Redis connections when the application closes, after
workers finished or gave up on their active jobs, so the process can exit.

#### Scenario: Close the application
- **WHEN** `app.close()` runs with one registered queue and no workers
- **THEN** it returns and the Redis server no longer lists the queue's connections (`CLIENT LIST`)
