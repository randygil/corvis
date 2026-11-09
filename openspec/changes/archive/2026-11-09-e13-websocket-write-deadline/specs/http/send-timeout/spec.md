# Spec Delta

## ADDED Requirements

### Requirement: Upgraded connections keep only the write deadline
After a connection has been upgraded (WebSocket gateways, `WebSocketServer`, `upgrade` listeners), the blocked-write
deadline of the send timeout SHALL keep applying to it, as specified in `ws/write-deadline`. The stalled-response
watchdog and the idle timers MUST NOT apply to it: an upgraded connection has no HTTP response bodies, and pausing
reading or having nothing to send is legitimate there.

#### Scenario: WebSocket client that stops reading
- **WHEN** a server with `send_timeout(Duration::from_secs(1))` accepts a WebSocket connection and the client stops
  reading while the gateway sends more than the kernel buffers hold but less than `max_buffered_amount`
- **THEN** the connection is closed within a few seconds

#### Scenario: Quiet WebSocket connection
- **WHEN** an upgraded connection on a server with `send_timeout(Duration::from_secs(1))` carries no frames for five
  seconds
- **THEN** the connection stays open

## REMOVED Requirements

### Requirement: Upgraded connections are not affected
**Reason**: a WebSocket peer that stops reading kept its connection, its `max_connections` slot and its queued frames
forever; replaced by "Upgraded connections keep only the write deadline" (#161).
**Migration**: `send_timeout(None)` restores the old behavior for every connection; a larger `send_timeout` gives
WebSocket peers more time.
