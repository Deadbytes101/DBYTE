# BYTEDECK

BYTEDECK is a small DByte-controlled music tool.

The DByte script owns commands and display. The native helper owns Windows audio
playback state.

## Commands

```powershell
.\personal_tools\bytedeck\run.ps1 help
.\personal_tools\bytedeck\run.ps1 play "D:\Music\track.mp3"
.\personal_tools\bytedeck\run.ps1 pause
.\personal_tools\bytedeck\run.ps1 resume
.\personal_tools\bytedeck\run.ps1 status
.\personal_tools\bytedeck\run.ps1 stop
```

## Native Helper

`native\dbyte-audio.c` builds to `native\dbyte-audio.exe`.

The executable has a simple command-line surface:

```txt
dbyte-audio.exe play <file>
dbyte-audio.exe pause
dbyte-audio.exe resume
dbyte-audio.exe stop
dbyte-audio.exe status
```

It starts a tiny resident helper process on first use so pause, resume, stop,
and status can control the active track across separate CLI invocations.

## Scope

v0.1 intentionally does not include persistent queues, album art, streaming,
accounts, cloud sync, a database, or a GUI framework.
