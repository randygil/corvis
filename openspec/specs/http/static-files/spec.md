# http/static-files Specification

## Purpose
Guarantees that serving a folder of static files never exposes anything outside that folder and applies the dotfiles
policy, whatever path, encoding or platform-specific trick a client uses.

## Requirements

### Requirement: Files outside the served folder are never served
For any request path under a static mount (including `..` segments, percent-encoded dots and slashes, backslashes,
drive prefixes, NUL bytes and repeated slashes), the response body MUST NOT contain the contents of a file outside the
served folder.

#### Scenario: Traversal attempts
- **WHEN** a client requests generated paths such as `/static/../outside/secret.txt`,
  `/static/%2e%2e%2foutside%2fsecret.txt`, `/static/..%5coutside%5csecret.txt` or `/static/C:/x`
- **THEN** no response contains the secret file's content, and the server answers with 3xx, 4xx or a file from inside
  the folder

### Requirement: The dotfiles policy holds for every path
Under the default policy (`ignore`) and under `deny`, the content of a file or folder whose name starts with a dot MUST
NOT be served, including through encoded names and, on Windows, short (8.3) aliases and alternate data streams.

#### Scenario: Hidden files under the default policy
- **WHEN** a client requests generated paths that resolve to `.env` or `.git/config` inside the served folder under a
  mount with the default policy
- **THEN** no response contains their content

### Requirement: Static serving is total
Static file serving SHALL answer every request path with an HTTP response and MUST NOT crash. A malformed or hostile
path is a client error: it MUST be answered with a 2xx, 3xx or 4xx status, never with a 5xx.

#### Scenario: Arbitrary paths
- **WHEN** a client sends generated paths made of valid and malformed segments under any static mount
- **THEN** every request gets a response with a status below 500 and the server keeps serving

#### Scenario: Platform-specific names
- **WHEN** a client requests paths with a NUL byte (`%00`), an alternate data stream suffix (`::$DATA`), a drive prefix
  (`C:`) or a reserved device name (`CON`, `NUL`)
- **THEN** the server answers with a 4xx status (or a file from inside the folder), not with a 5xx
