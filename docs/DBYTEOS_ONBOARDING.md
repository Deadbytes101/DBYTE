# DByteOS Onboarding

DByteOS onboarding is a deterministic first-run flow for users opening a release zip or entering the DByteOS shell for the first time.

## First commands

```txt
welcome
profile show
getting-started
commands
man-index
help
status
```

## Discovery path

- `welcome` gives the first screen and next commands.
- `profile show` prints the deterministic user profile.
- `getting-started` prints the first-run checklist.
- `commands` lists commands by category.
- `man-index` lists manual topics.
- `man <topic>` opens a manual page.
- `which <command>` shows how autopath resolves a command.

## Boundaries

DByteOS onboarding is userland documentation and command discovery only. It does not add a kernel, bootloader, OS passthrough, language syntax, or security-policy changes.
