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
task reset-demo
task list demo
task add demo write tests
task done demo 1
task status demo
task summary demo
task open demo
task doctor demo
task snapshot demo
task clear-done demo
```

## Boundaries

Projects are not executable manifests or host OS projects. DByteOS derives all
project paths from the project name and rejects path-like names. `clean` does
not remove projects because they are user data.

## v5.5.1 hardening

Project names are exact workspace identifiers, not paths. Empty names, `.`, `..`,
names containing `.`, `/`, `\`, `:`, spaces, or tabs are rejected before any
project path is derived.

Missing project commands are deterministic:

```txt
error: project not found: missing
```

`project reset-demo` is idempotent and always restores the same `demo` project
files, index entry, and demo task file.

Workspace tasks are stored at `home/deadbyte/projects/<name>/tasks.txt` and are
managed with `task`. See [Tasks](DBYTEOS_TASKS.md).

`project notes <name>` is read-only in v5.5.1. Editing project notes is deferred
to a future release.

---
[Home](../README.md) | [Personal Alpha](DBYTEOS_PERSONAL_ALPHA.md) | [Onboarding](DBYTEOS_ONBOARDING.md) | [Tasks](DBYTEOS_TASKS.md) | [Package Smoke](DBYTEOS_PACKAGE.md)

