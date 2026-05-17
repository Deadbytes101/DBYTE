# DByteOS Workspace Tasks

DByteOS workspace tasks are deterministic user data stored beside workspace
projects under `home/deadbyte/projects/<name>/tasks.txt`.

```txt
project reset-demo
task reset-demo
task list demo
task add demo write tests
task done demo 1
task status demo
```

Tasks are scoped to validated project names only. The command never accepts raw
paths, and task text rejects the internal `|` delimiter.

## Demo State

`task reset-demo` is idempotent and restores:

```txt
DByteOS project tasks: demo
[ ] 1: inspect workspace
[ ] 2: write project note
```

`project reset-demo` also restores the demo task file so project and task smoke
tests start from the same deterministic workspace.

## Boundaries

Tasks are not executable build steps or host OS jobs. They are simple user data
for the Personal Alpha workspace. `clean` preserves projects and tasks.

Related docs: [Projects](DBYTEOS_PROJECTS.md), [Personal Alpha](DBYTEOS_PERSONAL_ALPHA.md), [Package Guide](DBYTEOS_PACKAGE.md).
