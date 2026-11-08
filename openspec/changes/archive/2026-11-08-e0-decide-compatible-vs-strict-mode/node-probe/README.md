# Node probe

`probe.mjs` sends raw bytes to `http.createServer` (bound to `127.0.0.1`) and prints the status line per case.

    node probe.mjs

- `output-node-24.13.0.txt`: run on Node v24.13.0 (llhttp 9.3.0). Matches the Node column of design.md.
- Node 22: not run. Only Node 24.13.0 is installed on the machine that implemented the change, and installing another
  version was out of scope. Run the same script on Node 22 and compare with the Node 24 output; consi and facten
  originals run on Node 22 (houseplan on Node 24).
