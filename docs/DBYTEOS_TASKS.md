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
task summary demo
task open demo
task doctor demo
task snapshot demo
task clear-done demo
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

## Task UX

`task summary <project>` prints compact counts, `task open <project>` lists
only open tasks, `task doctor <project>` validates task rows, and
`task snapshot <project>` prints a deterministic task snapshot.

`task clear-done <project>` removes completed rows and preserves remaining open
tasks in stored order. Task IDs are display positions, so remaining tasks are
renumbered after clear-done.

## Boundaries

Tasks are not executable build steps or host OS jobs. They are simple user data
for the Personal Workspace Beta Foundation workspace. `clean` preserves projects and tasks.

Related docs: [Projects](DBYTEOS_PROJECTS.md), [Personal Workspace Beta Foundation](DBYTEOS_PERSONAL_ALPHA.md), [Package Guide](DBYTEOS_PACKAGE.md).

