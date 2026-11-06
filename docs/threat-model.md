# Threat model

cadastre reads from the most sensitive API on a host and writes into a folder
that is usually synchronised to other machines. This page says what it
protects, how, and where the protection ends.

## The asset: secrets in `docker inspect`

`docker inspect` returns every environment variable of a container, with its
value, and labels that routinely carry credentials (basic-auth hashes, API
keys). Documentation must never contain them.

## How cadastre keeps them out

1. **Allowlist, not blocklist.** The sanitizer builds a small model
   (`src/model.rs`) from named fields. Anything not copied into the model, such
   as environment values, commands and health-check commands, cannot reach the
   output because there is nowhere to put it.
2. **Environment variables are documented by name only.**
3. **Labels are dropped unless allowlisted.** Only OCI image metadata and a
   handful of Traefik routing labels survive. A Traefik rule is kept only if
   every matcher in it is a `Host`, `Path` or `Method` one; `Header`, `Query`,
   `Cookie` and any matcher not known yet drop the whole rule. Middlewares are
   excluded. Any value containing a URL with credentials
   (`https://user:token@host`) is dropped, whatever its key.
4. **Volumes appear by name, bind mounts by host path.** Host paths are
   disclosed. If your paths are sensitive, do not publish the output.
5. **A canary test enforces it.** Every sensitive value in the test fixtures
   starts with `LEAK-`. Tests fail if that prefix appears in the sanitized
   model, in any rendered page (both flavours), in the changelog or in the
   state folder.
6. **Output is escaped.** Names and values are written in code spans whose
   fence is longer than any backtick run inside, with control characters,
   zero-width characters and bidirectional overrides replaced and table pipes
   escaped, so a hostile container name cannot inject Markdown or HTML into
   your notes or make text read differently from what it is.
7. **The Docker client is narrow.** Plain HTTP and `GET` only, no redirects,
   no proxy taken from the environment, container IDs checked to be
   hexadecimal before they go into a request path, answers and event lines
   size-capped, and no credentials accepted in the URL (it is logged).

## Where the protection ends

- **The socket proxy is not a confidentiality boundary.** To read container
  details it must allow `CONTAINERS=1`, and that section also exposes other
  `GET` endpoints, including the same inspect output cadastre filters. cadastre
  discards secrets in memory; it cannot stop a compromised cadastre process
  from asking the proxy for them. The proxy's value is that it removes write
  access: `POST=0` means nobody can create, start, stop or exec into
  containers through it.
- **cadastre parses the full inspect document in memory** before dropping what
  it must not keep. The process holds secrets transiently.
- **Image references, published ports and Traefik `Path` arguments are
  written as-is.** An image name containing credentials, or a rule such as
  ``PathPrefix(`/hooks/<secret>`)``, would be copied.
- **No offsite or integrity protection** for the output folder. It is only as
  safe as the sync tool and the machines behind it.

## Hardening already in the example deployment

Read-only proxy on an internal network, non-root user, read-only root file
system, all capabilities dropped, `no-new-privileges`, a memory limit.

## Reporting a vulnerability

See [SECURITY.md](../SECURITY.md).
