# Security Policy

## Reporting a vulnerability

Use GitHub's private vulnerability reporting: open the repository, go to
**Security**, then **Report a vulnerability**. Please do not open a public
issue for anything you believe is a security problem.

Include what you found, how to reproduce it, and what you think the impact
is. A proof of concept helps but is not required.

## What happens next

- You get an acknowledgement, usually within a few days.
- The fix is developed privately where practical, then released.
- You are credited in the release notes if you want to be.

## Supported versions

Security fixes target `main` and the latest tagged release. There is no
long-term support for older versions; this project moves fast.

## Scope notes

The threat model lives in [docs/threat-model.md](docs/threat-model.md).
Findings that match a documented limitation are still worth reporting — the
documentation may be wrong, or the limitation may no longer be acceptable.
