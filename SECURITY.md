# Security policy

cadastre reads container metadata, including data that can contain secrets,
and writes into folders that are often synchronised. Reports about leaks are
taken seriously.

## Reporting a vulnerability

Please **do not open a public issue** for a security problem. Use GitHub's
private reporting: on the repository page, open the **Security** tab and choose
**Report a vulnerability**.

Include what you did, what you expected and what happened. If a secret reached
the output, a sanitized `docker inspect` excerpt that reproduces it is the most
useful thing you can send; replace real values before sharing.

This project is maintained on a best-effort basis, so there is no guaranteed
response time. Reports of a leak into the output are looked at first.

## What counts

In scope:

- A value that should have been dropped (environment value, credential-bearing
  label, command line) appearing in any output file or in the state folder.
- Markdown or HTML injection through container names, labels or images.
- A way to make cadastre write or delete outside its output folder, or delete a
  file it did not generate.
- A way to make cadastre send a request other than a `GET` to the Docker API.

Out of scope, documented in the [threat model](docs/threat-model.md): secrets
being readable through the socket proxy by a compromised process, and anything
about the sync tool or the machines it syncs to.

## Supported versions

Only the latest release receives fixes.
