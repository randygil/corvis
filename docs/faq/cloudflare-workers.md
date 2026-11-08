---
status: unavailable
---

# Cloudflare Workers

Corvis cannot run on Cloudflare Workers. Workers runs JavaScript or WebAssembly in an
environment without server TCP sockets or threads, and Corvis depends on Tokio (a multithreaded runtime) and on
hyper (a server over TCP), which do not compile for that target.

Alternatives:

- Deploy the application as a container or binary on a server or on a container
  platform, and put Cloudflare in front as a proxy and CDN.
- Move only the edge logic (redirects, caching, header rewriting) to a Worker and leave
  the API in Corvis.

See also [Deployment](/deployment) and [Serverless](/faq/serverless).
