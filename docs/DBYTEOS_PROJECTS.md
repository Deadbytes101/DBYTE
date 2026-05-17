# DByteOS Workspace Projects

DByteOS workspace projects are deterministic user data stored under
`home/deadbyte/projects/`.

## Commands

```txt
project new demo
project list
project status demo
project notes demo
project snapshot demo
project doctor demo
project reset-demo
```

## Boundaries

Projects are not executable manifests or host OS projects. DByteOS derives all
project paths from the project name and rejects path-like names. `clean` does
not remove projects because they are user data.

## v5.1.1 hardening

Project names are exact workspace identifiers, not paths. Empty names, `.`, `..`,
names containing `.`, `/`, `\`, `:`, spaces, or tabs are rejected before any
project path is derived.

Missing project commands are deterministic:

```txt
error: project not found: missing
```

`project reset-demo` is idempotent and always restores the same `demo` project
files and index entry.

`project notes <name>` is read-only in v5.1.1. Editing project notes is deferred
to a future release.

---
[Home](../README.md) | [Personal Alpha](DBYTEOS_PERSONAL_ALPHA.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Package Smoke](DBYTEOS_PACKAGE.md)
