# DByte Sanctum Demo Workspace

This is the foundation of the DByte Personal Computing Environment (Sanctum).

## Getting Started

1.  **Enter the Sanctum**:
    ```powershell
    # Use the local .dbyterc
    ../../target/release/dbyte.exe shell
    ```

2.  **Run the Boot Script**:
    Within the shell or from the host:
    ```powershell
    ../../target/release/dbyte.exe run boot.dby
    ```

## Available Tools (Aliases)
Once inside the Sanctum shell, the following aliases are active:

| Alias | Command | Description |
|---|---|---|
| `boot` | `run boot.dby` | Start the environment |
| `inspect` | `run tools/inspect_demo.dby` | Inspect workspace binaries |
| `patch-demo` | `run tools/patch_demo.dby` | Run a patching demo |
| `u32-demo` | `run tools/u32_demo.dby` | Dump u32 data |

## Structure
- `.dbyterc`: Environment configuration.
- `boot.dby`: Entry point.
- `tools/`: Localized personal tools.
- `workspace/`: Data area (samples).
